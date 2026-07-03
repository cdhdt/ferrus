//! Tests for the raw-copy loop and the destructive orchestration.
//!
//! No real device is ever touched: the copy loop runs against an in-memory
//! [`WriteSink`], and the orchestration runs against a fake [`WriteBackend`]
//! that records unmounts and captures written/synced state. Images are backed
//! by tempfiles.

use std::cell::{Cell, RefCell};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::stream::copy_stream;
use super::{ensure_fits, raw_copy_with};
use crate::device::{Bus, Device, SafeTarget};
use crate::platform::{WriteBackend, WriteSink};
use crate::progress::{ProgressSink, Stage};
use crate::source::RawImage;
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
