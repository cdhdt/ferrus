//! Tests for the GPT geometry and the prepare orchestration.
//!
//! No real device is touched: geometry is pure, and the orchestration runs
//! against a fake [`PartitionBackend`] that records the operation order and can
//! be told a tool is missing.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::plan::MIB;
use super::{
    FsKind, MICROSOFT_BASIC_DATA_GUID, MIN_DEVICE_BYTES, compute_windows_layout, partition_path,
    prepare_windows_with,
};
use crate::device::{Bus, Device, SafeTarget};
use crate::platform::PartitionBackend;
use crate::progress::NullProgress;
use crate::{Error, Result};

// --- pure geometry --------------------------------------------------------

#[test]
fn layout_for_8gib_is_aligned_and_bounded() {
    let device = 8 * 1024 * MIB; // 8 GiB
    let layout = compute_windows_layout(device).unwrap();

    // P1 starts at 1 MiB, P2 is the 1 MiB before the 1 MiB back reserve.
    assert_eq!(layout.windows.start_bytes, MIB);
    assert_eq!(layout.windows.size_bytes, 8189 * MIB);
    assert_eq!(layout.helper.start_bytes, 8190 * MIB);
    assert_eq!(layout.helper.size_bytes, MIB);

    // P1 exactly bounds P2; P2 is last; nothing overlaps.
    assert_eq!(
        layout.windows.start_bytes + layout.windows.size_bytes,
        layout.helper.start_bytes
    );
    assert!(layout.helper.start_bytes + layout.helper.size_bytes <= device);

    // Everything 1 MiB-aligned and non-empty; correct fs and type.
    for part in layout.partitions() {
        assert_eq!(part.start_bytes % MIB, 0);
        assert_eq!(part.size_bytes % MIB, 0);
        assert!(part.size_bytes > 0);
        assert_eq!(part.type_guid, MICROSOFT_BASIC_DATA_GUID);
    }
    assert_eq!(layout.windows.fs, FsKind::Ntfs);
    assert_eq!(layout.helper.fs, FsKind::Fat);
}

#[test]
fn too_small_device_is_refused() {
    let err = compute_windows_layout(MIN_DEVICE_BYTES - 1).unwrap_err();
    assert!(matches!(err, Error::DeviceTooSmall { .. }));
}

#[test]
fn sfdisk_script_is_gpt_with_both_partitions() {
    let layout = compute_windows_layout(8 * 1024 * MIB).unwrap();
    let script = layout.sfdisk_script();
    assert!(script.starts_with("label: gpt\n"));
    assert_eq!(script.matches(MICROSOFT_BASIC_DATA_GUID).count(), 2);
    assert!(script.contains("size=8189MiB"));
    assert!(script.contains("size=1MiB"));
    assert!(script.contains("name=\"UEFI_NTFS\""));
}

#[test]
fn partition_paths_handle_sd_and_nvme() {
    assert_eq!(partition_path(Path::new("/dev/sda"), 1), Path::new("/dev/sda1"));
    assert_eq!(
        partition_path(Path::new("/dev/nvme0n1"), 2),
        Path::new("/dev/nvme0n1p2")
    );
}

// --- orchestration --------------------------------------------------------

struct FakeBackend {
    euid: u32,
    missing_tool: Option<&'static str>,
    system_critical: bool,
    mounts: Vec<PathBuf>,
    log: RefCell<Vec<String>>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            euid: 0,
            missing_tool: None,
            system_critical: false,
            mounts: Vec::new(),
            log: RefCell::new(Vec::new()),
        }
    }
    fn record(&self, entry: impl Into<String>) {
        self.log.borrow_mut().push(entry.into());
    }
}

impl PartitionBackend for FakeBackend {
    fn effective_uid(&self) -> Result<u32> {
        self.record("euid");
        Ok(self.euid)
    }
    fn is_system_or_critical(&self, _device_path: &Path) -> Result<bool> {
        self.record("is_system");
        Ok(self.system_critical)
    }
    fn mounted_partitions(&self, _device_path: &Path) -> Result<Vec<PathBuf>> {
        Ok(self.mounts.clone())
    }
    fn unmount(&self, mountpoint: &Path) -> Result<()> {
        self.record(format!("unmount:{}", mountpoint.display()));
        Ok(())
    }
    fn ensure_tools(&self, tools: &[&str]) -> Result<()> {
        self.record("ensure_tools");
        if let Some(missing) = self.missing_tool
            && tools.contains(&missing)
        {
            return Err(Error::MissingTool {
                name: missing.to_owned(),
            });
        }
        Ok(())
    }
    fn write_partition_table(&self, _device_path: &Path, _script: &str) -> Result<()> {
        self.record("write_table");
        Ok(())
    }
    fn reread_partition_table(&self, _device_path: &Path) -> Result<()> {
        self.record("reread");
        Ok(())
    }
    fn wait_for_partitions(&self, device_path: &Path, count: usize) -> Result<Vec<PathBuf>> {
        self.record("wait");
        Ok((1..=count).map(|i| partition_path(device_path, i)).collect())
    }
    fn make_filesystem(&self, partition_path: &Path, fs: FsKind, _label: &str) -> Result<()> {
        self.record(format!("mkfs:{fs:?}:{}", partition_path.display()));
        Ok(())
    }
}

fn target(dry_run: bool) -> SafeTarget {
    let device = Device {
        path: PathBuf::from("/dev/sda"),
        stable_id: None,
        model: Some("Test USB".to_owned()),
        bus: Bus::Usb,
        size_bytes: 8 * 1024 * MIB,
        removable: true,
        is_system_or_critical: false,
    };
    SafeTarget::new_for_test(device, dry_run)
}

#[test]
fn happy_path_runs_operations_in_order() {
    let mut backend = FakeBackend::new();
    backend.mounts = vec![PathBuf::from("/mnt/a")];
    let mut progress = NullProgress;

    prepare_windows_with(&target(false), &mut progress, &backend).unwrap();

    assert_eq!(
        *backend.log.borrow(),
        vec![
            "euid",
            "ensure_tools",
            "is_system",
            "unmount:/mnt/a",
            "write_table",
            "reread",
            "wait",
            "mkfs:Ntfs:/dev/sda1",
            "mkfs:Fat:/dev/sda2",
        ]
    );
}

#[test]
fn dry_run_touches_nothing() {
    let backend = FakeBackend::new();
    let mut progress = NullProgress;
    prepare_windows_with(&target(true), &mut progress, &backend).unwrap();
    assert!(backend.log.borrow().is_empty());
}

#[test]
fn missing_tool_fails_before_any_write() {
    let mut backend = FakeBackend::new();
    backend.missing_tool = Some("mkfs.ntfs");
    let mut progress = NullProgress;

    let err = prepare_windows_with(&target(false), &mut progress, &backend).unwrap_err();
    assert!(matches!(err, Error::MissingTool { .. }));

    let log = backend.log.borrow();
    assert!(!log.iter().any(|e| e == "write_table"));
    assert!(!log.iter().any(|e| e.starts_with("mkfs:")));
    assert!(!log.iter().any(|e| e.starts_with("unmount")));
}

#[test]
fn non_root_is_refused_before_any_write() {
    let mut backend = FakeBackend::new();
    backend.euid = 1000;
    let mut progress = NullProgress;

    let err = prepare_windows_with(&target(false), &mut progress, &backend).unwrap_err();
    assert!(matches!(err, Error::PrivilegeRequired(_)));
    assert!(!backend.log.borrow().iter().any(|e| e == "write_table"));
}

#[test]
fn critical_target_aborts_before_unmount() {
    let mut backend = FakeBackend::new();
    backend.system_critical = true;
    backend.mounts = vec![PathBuf::from("/mnt/a")];
    let mut progress = NullProgress;

    let err = prepare_windows_with(&target(false), &mut progress, &backend).unwrap_err();
    assert!(matches!(err, Error::UnsafeTarget(_)));
    let log = backend.log.borrow();
    assert!(!log.iter().any(|e| e.starts_with("unmount")));
    assert!(!log.iter().any(|e| e == "write_table"));
}
