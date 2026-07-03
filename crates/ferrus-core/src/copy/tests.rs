//! Tests for the raw-copy loop and the destructive orchestration.
//!
//! No real device is ever touched: the copy loop runs against an in-memory
//! [`WriteSink`], and the orchestration runs against a fake [`WriteBackend`]
//! that records unmounts and captures written/synced state. Images are backed
//! by tempfiles.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::stream::{BLOCK_SIZE, copy_stream};
use super::tree::{TreeEntry, TreeIo, copy_tree, scan};
use super::windows::copy_windows_with;
use super::{ensure_fits, raw_copy_with};
use crate::device::{Bus, Device, SafeTarget};
use crate::platform::{Mount, MountBackend, WriteBackend, WriteSink};
use crate::progress::{ProgressSink, Stage};
use crate::source::{RawImage, detect_windows_install};
use crate::windows::WindowsTweaks;
use crate::{Error, Result};

// --- test doubles ---------------------------------------------------------

/// Records progress so tests can assert coverage.
#[derive(Default)]
struct RecordingProgress {
    last: Option<(u64, Option<u64>)>,
    messages: Vec<String>,
    stages: Vec<Stage>,
}

impl ProgressSink for RecordingProgress {
    fn stage(&mut self, stage: Stage) {
        self.stages.push(stage);
    }
    fn advance(&mut self, done: u64, total: Option<u64>) {
        self.last = Some((done, total));
    }
    fn message(&mut self, text: &str) {
        self.messages.push(text.to_owned());
    }
}

/// In-memory sink capturing bytes and whether `sync` was called.
struct MemSink {
    data: Rc<RefCell<Vec<u8>>>,
    synced: Rc<Cell<bool>>,
}

impl WriteSink for MemSink {
    fn write_chunk(&mut self, buf: &[u8]) -> Result<()> {
        self.data.borrow_mut().extend_from_slice(buf);
        Ok(())
    }
    fn sync(&mut self) -> Result<()> {
        self.synced.set(true);
        Ok(())
    }
}

/// Configurable fake of the OS write backend.
struct FakeBackend {
    euid: u32,
    system_critical: bool,
    mounts: Vec<PathBuf>,
    corrupt_readback: bool,
    written: Rc<RefCell<Vec<u8>>>,
    synced: Rc<Cell<bool>>,
    unmounted: RefCell<Vec<PathBuf>>,
    writer_opened: Cell<bool>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            euid: 0,
            system_critical: false,
            mounts: Vec::new(),
            corrupt_readback: false,
            written: Rc::new(RefCell::new(Vec::new())),
            synced: Rc::new(Cell::new(false)),
            unmounted: RefCell::new(Vec::new()),
            writer_opened: Cell::new(false),
        }
    }
}

impl WriteBackend for FakeBackend {
    fn effective_uid(&self) -> Result<u32> {
        Ok(self.euid)
    }
    fn is_system_or_critical(&self, _device_path: &Path) -> Result<bool> {
        Ok(self.system_critical)
    }
    fn mounted_partitions(&self, _device_path: &Path) -> Result<Vec<PathBuf>> {
        Ok(self.mounts.clone())
    }
    fn unmount(&self, mountpoint: &Path) -> Result<()> {
        self.unmounted.borrow_mut().push(mountpoint.to_path_buf());
        Ok(())
    }
    fn open_exclusive_writer(&self, _device_path: &Path) -> Result<Box<dyn WriteSink>> {
        self.writer_opened.set(true);
        Ok(Box::new(MemSink {
            data: self.written.clone(),
            synced: self.synced.clone(),
        }))
    }
    fn open_reader(&self, _device_path: &Path) -> Result<Box<dyn Read>> {
        let mut data = self.written.borrow().clone();
        if self.corrupt_readback && !data.is_empty() {
            data[0] ^= 0xff;
        }
        Ok(Box::new(Cursor::new(data)))
    }
}

// --- helpers --------------------------------------------------------------

fn target_with_size(size_bytes: u64, dry_run: bool) -> SafeTarget {
    let device = Device {
        path: PathBuf::from("/dev/sdz"),
        stable_id: None,
        model: Some("Test USB".to_owned()),
        bus: Bus::Usb,
        size_bytes,
        removable: true,
        is_system_or_critical: false,
    };
    SafeTarget::new_for_test(device, dry_run)
}

fn image_of(bytes: &[u8]) -> (tempfile::NamedTempFile, RawImage) {
    let mut file = tempfile::NamedTempFile::new().expect("temp image");
    file.write_all(bytes).expect("write image");
    file.flush().expect("flush image");
    let image = RawImage::open(file.path()).expect("open image");
    (file, image)
}

/// A byte pattern a few blocks long, deliberately not a block multiple.
fn sample_bytes() -> Vec<u8> {
    (0..(super::stream::BLOCK_SIZE * 2 + 1234))
        .map(|i| (i % 251) as u8)
        .collect()
}

// --- copy loop ------------------------------------------------------------

#[test]
fn copy_stream_writes_all_bytes_and_reaches_full_progress() {
    let data = sample_bytes();
    let mut reader = Cursor::new(data.clone());
    let sink_data = Rc::new(RefCell::new(Vec::new()));
    let mut sink = MemSink {
        data: sink_data.clone(),
        synced: Rc::new(Cell::new(false)),
    };
    let mut progress = RecordingProgress::default();

    let total = data.len() as u64;
    let written = copy_stream(&mut reader, &mut sink, total, &mut progress).unwrap();

    assert_eq!(written, total);
    assert_eq!(*sink_data.borrow(), data);
    assert_eq!(progress.last, Some((total, Some(total))));
}

// --- size guard -----------------------------------------------------------

#[test]
fn ensure_fits_rejects_oversized_image() {
    assert!(ensure_fits(100, 100).is_ok());
    assert!(matches!(
        ensure_fits(101, 100),
        Err(Error::ImageTooLarge { .. })
    ));
}

#[test]
fn oversized_image_is_refused_without_writing() {
    let data = sample_bytes();
    let (_f, image) = image_of(&data);
    let target = target_with_size(data.len() as u64 - 1, false);
    let backend = FakeBackend::new();
    let mut progress = RecordingProgress::default();

    let err = raw_copy_with(&image, &target, false, &mut progress, &backend).unwrap_err();
    assert!(matches!(err, Error::ImageTooLarge { .. }));
    assert!(!backend.writer_opened.get());
    assert!(backend.written.borrow().is_empty());
}

// --- dry-run --------------------------------------------------------------

#[test]
fn dry_run_touches_nothing() {
    let data = sample_bytes();
    let (_f, image) = image_of(&data);
    let target = target_with_size(1_000_000_000, true);
    // Non-root on purpose: dry-run must not even consult the EUID gate.
    let mut backend = FakeBackend::new();
    backend.euid = 1000;
    backend.mounts = vec![PathBuf::from("/mnt/a")];
    let mut progress = RecordingProgress::default();

    raw_copy_with(&image, &target, false, &mut progress, &backend).unwrap();

    assert!(!backend.writer_opened.get());
    assert!(backend.written.borrow().is_empty());
    assert!(backend.unmounted.borrow().is_empty());
    assert!(!backend.synced.get());
}

// --- real write -----------------------------------------------------------

#[test]
fn real_write_unmounts_copies_and_fsyncs() {
    let data = sample_bytes();
    let (_f, image) = image_of(&data);
    let target = target_with_size(1_000_000_000, false);
    let mut backend = FakeBackend::new();
    backend.mounts = vec![PathBuf::from("/mnt/a"), PathBuf::from("/mnt/b")];
    let mut progress = RecordingProgress::default();

    raw_copy_with(&image, &target, false, &mut progress, &backend).unwrap();

    assert_eq!(*backend.written.borrow(), data);
    assert!(backend.synced.get(), "must fsync before success");
    assert_eq!(
        *backend.unmounted.borrow(),
        vec![PathBuf::from("/mnt/a"), PathBuf::from("/mnt/b")]
    );
}

#[test]
fn non_root_is_refused() {
    let data = sample_bytes();
    let (_f, image) = image_of(&data);
    let target = target_with_size(1_000_000_000, false);
    let mut backend = FakeBackend::new();
    backend.euid = 1000;
    let mut progress = RecordingProgress::default();

    let err = raw_copy_with(&image, &target, false, &mut progress, &backend).unwrap_err();
    assert!(matches!(err, Error::PrivilegeRequired(_)));
    assert!(backend.written.borrow().is_empty());
}

#[test]
fn critical_target_aborts_before_unmount() {
    let data = sample_bytes();
    let (_f, image) = image_of(&data);
    let target = target_with_size(1_000_000_000, false);
    let mut backend = FakeBackend::new();
    backend.system_critical = true;
    backend.mounts = vec![PathBuf::from("/mnt/a")];
    let mut progress = RecordingProgress::default();

    let err = raw_copy_with(&image, &target, false, &mut progress, &backend).unwrap_err();
    assert!(matches!(err, Error::UnsafeTarget(_)));
    assert!(backend.unmounted.borrow().is_empty());
    assert!(backend.written.borrow().is_empty());
}

// --- verify ---------------------------------------------------------------

#[test]
fn verify_passes_when_readback_matches() {
    let data = sample_bytes();
    let (_f, image) = image_of(&data);
    let target = target_with_size(1_000_000_000, false);
    let backend = FakeBackend::new();
    let mut progress = RecordingProgress::default();

    raw_copy_with(&image, &target, true, &mut progress, &backend).unwrap();
    assert_eq!(*backend.written.borrow(), data);
}

#[test]
fn verify_detects_corrupted_readback() {
    let data = sample_bytes();
    let (_f, image) = image_of(&data);
    let target = target_with_size(1_000_000_000, false);
    let mut backend = FakeBackend::new();
    backend.corrupt_readback = true;
    let mut progress = RecordingProgress::default();

    let err = raw_copy_with(&image, &target, true, &mut progress, &backend).unwrap_err();
    assert!(matches!(err, Error::VerifyMismatch { offset: 0 }));
}

// =========================================================================
// Phase 3b — Windows ISO detection, recursive copy, and orchestration.
// =========================================================================

// --- Windows detection (pure) --------------------------------------------

fn windows_marker_map() -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    m.insert("sources/install.wim".to_owned(), 5_000_000_000); // > 4 GB
    m.insert("bootmgr".to_owned(), 1024);
    m.insert("efi/boot/bootx64.efi".to_owned(), 2048);
    m
}

#[test]
fn detects_windows_iso_with_wim() {
    let install = detect_windows_install(&windows_marker_map()).unwrap();
    assert_eq!(install.install_image, "sources/install.wim");
    assert_eq!(install.install_image_bytes, 5_000_000_000);
}

#[test]
fn detects_windows_iso_with_esd() {
    let mut m = windows_marker_map();
    m.remove("sources/install.wim");
    m.insert("sources/install.esd".to_owned(), 4_500_000_000);
    let install = detect_windows_install(&m).unwrap();
    assert_eq!(install.install_image, "sources/install.esd");
}

#[test]
fn non_windows_tree_is_rejected() {
    // Missing bootmgr.
    let mut m = windows_marker_map();
    m.remove("bootmgr");
    assert!(detect_windows_install(&m).is_none());
    // Missing any install image.
    let mut m = windows_marker_map();
    m.remove("sources/install.wim");
    assert!(detect_windows_install(&m).is_none());
    // A generic ISO.
    let mut m = BTreeMap::new();
    m.insert("readme.txt".to_owned(), 10);
    assert!(detect_windows_install(&m).is_none());
}

// --- in-memory TreeIo -----------------------------------------------------

#[derive(Default)]
struct FakeTreeIo {
    /// dir path -> [(name, is_dir, size)]
    dirs: BTreeMap<PathBuf, Vec<(String, bool, u64)>>,
    /// file path -> contents (source side)
    contents: BTreeMap<PathBuf, Vec<u8>>,
    /// captured writes (dest side)
    written: RefCell<BTreeMap<PathBuf, Rc<RefCell<Vec<u8>>>>>,
    /// created directories (dest side)
    created: RefCell<Vec<PathBuf>>,
    /// force a read failure at this path
    fail_read: Option<PathBuf>,
}

struct VecWriter {
    buf: Rc<RefCell<Vec<u8>>>,
}

impl Write for VecWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl TreeIo for FakeTreeIo {
    fn read_dir(&self, dir: &Path) -> Result<Vec<TreeEntry>> {
        Ok(self
            .dirs
            .get(dir)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, is_dir, size)| TreeEntry {
                path: dir.join(name),
                is_dir,
                size,
            })
            .collect())
    }
    fn open_read(&self, file: &Path) -> Result<Box<dyn Read>> {
        if self.fail_read.as_deref() == Some(file) {
            return Err(std::io::Error::other("forced read failure").into());
        }
        Ok(Box::new(Cursor::new(
            self.contents.get(file).cloned().unwrap_or_default(),
        )))
    }
    fn create_dir(&self, dir: &Path) -> Result<()> {
        self.created.borrow_mut().push(dir.to_path_buf());
        Ok(())
    }
    fn open_write(&self, file: &Path) -> Result<Box<dyn Write>> {
        let buf = Rc::new(RefCell::new(Vec::new()));
        self.written
            .borrow_mut()
            .insert(file.to_path_buf(), buf.clone());
        Ok(Box::new(VecWriter { buf }))
    }
    fn file_size(&self, file: &Path) -> Result<u64> {
        if let Some(b) = self.written.borrow().get(file) {
            return Ok(b.borrow().len() as u64);
        }
        if let Some(c) = self.contents.get(file) {
            return Ok(c.len() as u64);
        }
        Err(std::io::Error::from(std::io::ErrorKind::NotFound).into())
    }
}

impl FakeTreeIo {
    fn dest_bytes(&self, path: &str) -> Option<Vec<u8>> {
        self.written
            .borrow()
            .get(Path::new(path))
            .map(|b| b.borrow().clone())
    }
}

/// A minimal Windows ISO tree under `root`, with `install.wim` of `wim_size`
/// bytes (contents match the declared size).
fn windows_tree(root: &str, wim_size: usize) -> FakeTreeIo {
    let root = PathBuf::from(root);
    let mut dirs = BTreeMap::new();
    dirs.insert(
        root.clone(),
        vec![
            ("sources".to_owned(), true, 0),
            ("bootmgr".to_owned(), false, 4),
            ("efi".to_owned(), true, 0),
        ],
    );
    dirs.insert(
        root.join("sources"),
        vec![("install.wim".to_owned(), false, wim_size as u64)],
    );
    dirs.insert(root.join("efi"), vec![("boot".to_owned(), true, 0)]);
    dirs.insert(
        root.join("efi").join("boot"),
        vec![("bootx64.efi".to_owned(), false, 6)],
    );
    let mut contents = BTreeMap::new();
    contents.insert(root.join("bootmgr"), vec![1u8; 4]);
    contents.insert(
        root.join("sources").join("install.wim"),
        vec![7u8; wim_size],
    );
    contents.insert(
        root.join("efi").join("boot").join("bootx64.efi"),
        vec![9u8; 6],
    );
    FakeTreeIo {
        dirs,
        contents,
        ..Default::default()
    }
}

// --- recursive copy -------------------------------------------------------

#[test]
fn copy_tree_preserves_structure_case_and_streams_large_files() {
    // A tree with mixed-case names and a file larger than one block.
    let big = BLOCK_SIZE + 100;
    let mut io = FakeTreeIo::default();
    io.dirs.insert(
        PathBuf::from("/src"),
        vec![
            ("Dir".to_owned(), true, 0),
            ("File.TXT".to_owned(), false, 3),
        ],
    );
    io.dirs.insert(
        PathBuf::from("/src/Dir"),
        vec![("deep.bin".to_owned(), false, big as u64)],
    );
    io.contents
        .insert(PathBuf::from("/src/File.TXT"), vec![1u8, 2, 3]);
    io.contents
        .insert(PathBuf::from("/src/Dir/deep.bin"), vec![7u8; big]);

    let total = 3 + big as u64;
    let mut progress = RecordingProgress::default();
    let copied = copy_tree(
        &io,
        Path::new("/src"),
        Path::new("/dst"),
        total,
        &mut progress,
    )
    .unwrap();

    assert_eq!(copied, total);
    // Case and structure preserved verbatim.
    assert_eq!(io.dest_bytes("/dst/File.TXT"), Some(vec![1u8, 2, 3]));
    assert_eq!(io.dest_bytes("/dst/Dir/deep.bin"), Some(vec![7u8; big]));
    assert!(io.created.borrow().contains(&PathBuf::from("/dst/Dir")));
    // Progress reached the full total.
    assert_eq!(progress.last, Some((total, Some(total))));
}

#[test]
fn scan_totals_and_detects_windows() {
    let io = windows_tree("/iso", 100);
    let s = scan(&io, Path::new("/iso")).unwrap();
    assert_eq!(s.total_bytes, 100 + 4 + 6);
    assert!(detect_windows_install(&s.files).is_some());
}

// --- orchestration --------------------------------------------------------

struct RecordingMount {
    path: PathBuf,
    tag: &'static str,
    log: Rc<RefCell<Vec<String>>>,
}

impl Mount for RecordingMount {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RecordingMount {
    fn drop(&mut self) {
        self.log.borrow_mut().push(format!("umount:{}", self.tag));
    }
}

struct FakeMountBackend {
    log: Rc<RefCell<Vec<String>>>,
}

impl FakeMountBackend {
    fn new() -> Self {
        Self {
            log: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl MountBackend for FakeMountBackend {
    fn mount_iso_ro(&self, _image: &Path) -> Result<Box<dyn Mount>> {
        self.log.borrow_mut().push("mount_iso".to_owned());
        Ok(Box::new(RecordingMount {
            path: PathBuf::from("/iso"),
            tag: "iso",
            log: self.log.clone(),
        }))
    }
    fn mount_ntfs_rw(&self, _partition: &Path) -> Result<Box<dyn Mount>> {
        self.log.borrow_mut().push("mount_ntfs".to_owned());
        Ok(Box::new(RecordingMount {
            path: PathBuf::from("/ntfs"),
            tag: "ntfs",
            log: self.log.clone(),
        }))
    }
    fn sync_path(&self, _path: &Path) -> Result<()> {
        self.log.borrow_mut().push("sync".to_owned());
        Ok(())
    }
}

#[test]
fn windows_copy_dry_run_mounts_nothing() {
    let (_f, image) = image_of(&[0u8; 16]);
    let io = windows_tree("/iso", 10);
    let mounts = FakeMountBackend::new();
    let mut progress = RecordingProgress::default();

    copy_windows_with(
        &image,
        Path::new("/dev/sda1"),
        1_000_000,
        None,
        true,
        false,
        &mut progress,
        &mounts,
        &io,
    )
    .unwrap();

    assert!(mounts.log.borrow().is_empty());
}

#[test]
fn windows_copy_happy_path_order_and_content() {
    let (_f, image) = image_of(&[0u8; 16]);
    let io = windows_tree("/iso", 10);
    let mounts = FakeMountBackend::new();
    let mut progress = RecordingProgress::default();

    copy_windows_with(
        &image,
        Path::new("/dev/sda1"),
        1_000_000,
        None,
        false,
        true, // verify
        &mut progress,
        &mounts,
        &io,
    )
    .unwrap();

    // Mounts, sync, then RAII unmount of ntfs (declared last) before iso.
    assert_eq!(
        *mounts.log.borrow(),
        vec![
            "mount_iso",
            "mount_ntfs",
            "sync",
            "umount:ntfs",
            "umount:iso"
        ]
    );
    // Files landed on the NTFS mount, case preserved.
    assert_eq!(
        io.dest_bytes("/ntfs/sources/install.wim"),
        Some(vec![7u8; 10])
    );
    assert!(io.dest_bytes("/ntfs/efi/boot/bootx64.efi").is_some());
    // Opt-in: no tweaks → no autounattend.xml.
    assert!(io.dest_bytes("/ntfs/autounattend.xml").is_none());
}

#[test]
fn windows_copy_deposits_autounattend_when_tweaks_present() {
    let (_f, image) = image_of(&[0u8; 16]);
    let io = windows_tree("/iso", 10);
    let mounts = FakeMountBackend::new();
    let mut progress = RecordingProgress::default();
    let tweaks = WindowsTweaks {
        bypass_hardware: true,
        ..WindowsTweaks::default()
    };

    copy_windows_with(
        &image,
        Path::new("/dev/sda1"),
        1_000_000,
        Some(&tweaks),
        false,
        false,
        &mut progress,
        &mounts,
        &io,
    )
    .unwrap();

    let dropped = io
        .dest_bytes("/ntfs/autounattend.xml")
        .expect("autounattend.xml must be written to the NTFS root");
    let xml = String::from_utf8(dropped).unwrap();
    assert!(xml.contains(r"HKLM\SYSTEM\Setup\LabConfig"));
    assert!(xml.contains("BypassTPMCheck"));
}

#[test]
fn windows_copy_rejects_non_windows_before_ntfs_mount() {
    let (_f, image) = image_of(&[0u8; 16]);
    let mut io = FakeTreeIo::default();
    io.dirs.insert(
        PathBuf::from("/iso"),
        vec![("readme.txt".to_owned(), false, 5)],
    );
    io.contents
        .insert(PathBuf::from("/iso/readme.txt"), vec![0u8; 5]);
    let mounts = FakeMountBackend::new();
    let mut progress = RecordingProgress::default();

    let err = copy_windows_with(
        &image,
        Path::new("/dev/sda1"),
        1_000_000,
        None,
        false,
        false,
        &mut progress,
        &mounts,
        &io,
    )
    .unwrap_err();
    assert!(matches!(err, Error::NotWindowsMedia { .. }));

    let log = mounts.log.borrow();
    assert!(log.contains(&"mount_iso".to_owned()));
    assert!(log.contains(&"umount:iso".to_owned()));
    assert!(!log.contains(&"mount_ntfs".to_owned()));
}

#[test]
fn windows_copy_space_guard_before_ntfs_mount() {
    let (_f, image) = image_of(&[0u8; 16]);
    let io = windows_tree("/iso", 1000);
    let mounts = FakeMountBackend::new();
    let mut progress = RecordingProgress::default();

    let err = copy_windows_with(
        &image,
        Path::new("/dev/sda1"),
        100, // capacity smaller than the content
        None,
        false,
        false,
        &mut progress,
        &mounts,
        &io,
    )
    .unwrap_err();
    assert!(matches!(err, Error::InsufficientSpace { .. }));

    let log = mounts.log.borrow();
    assert!(log.contains(&"umount:iso".to_owned()));
    assert!(!log.contains(&"mount_ntfs".to_owned()));
}

#[test]
fn windows_copy_unmounts_both_on_copy_failure() {
    let (_f, image) = image_of(&[0u8; 16]);
    let mut io = windows_tree("/iso", 10);
    io.fail_read = Some(PathBuf::from("/iso/sources/install.wim"));
    let mounts = FakeMountBackend::new();
    let mut progress = RecordingProgress::default();

    let err = copy_windows_with(
        &image,
        Path::new("/dev/sda1"),
        1_000_000,
        None,
        false,
        false,
        &mut progress,
        &mounts,
        &io,
    )
    .unwrap_err();
    assert!(matches!(err, Error::Io(_)));

    // Both mounts torn down despite the mid-copy failure; sync never reached.
    let log = mounts.log.borrow();
    assert!(log.contains(&"umount:iso".to_owned()));
    assert!(log.contains(&"umount:ntfs".to_owned()));
    assert!(!log.contains(&"sync".to_owned()));
}
