//! Windows partitioning + formatting orchestration (SPEC-00010, Phase 6.2a).
//!
//! The Windows counterpart of the Linux `prepare_windows_with` flow. It reuses
//! the **shared** geometry ([`compute_windows_layout`]) and drives a small,
//! injectable [`WinPartitionBackend`] so the ordering, the dry-run contract, and
//! the ESP guard are unit-tested on any host with a fake — the real backend
//! (native IOCTLs + a `Format-Volume` shell-out) lives in `platform::windows`.
//!
//! The layout is SPEC-0003's: GPT, P1 = Microsoft basic-data NTFS spanning almost
//! the whole disk, P2 = a ~1 MiB FAT helper at the end (left **raw** here, exactly
//! as on Linux — the UEFI:NTFS image written in 6.2c carries its own filesystem).

use std::path::Path;

use crate::device::{SafeTarget, format_size};
use crate::error::{Error, Result};
use crate::partition::{FsKind, GptLayout, compute_windows_layout};
use crate::progress::{ProgressSink, Stage};

// --- The ESP / system-partition guard (closes the SPEC-0009 6.1.1 TODO) --------

/// EFI System Partition (`PARTITION_SYSTEM_GUID`). Verified against Microsoft
/// Learn, *PARTITION_INFORMATION_GPT*.
pub const EFI_SYSTEM_PARTITION_GUID: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
/// Microsoft Reserved Partition (`PARTITION_MSFT_RESERVED_GUID`).
pub const MICROSOFT_RESERVED_GUID: &str = "E3C9E316-0B5C-4DB8-817D-F92DF00215AE";
/// Microsoft Recovery Partition (`PARTITION_MSFT_RECOVERY_GUID`).
pub const MICROSOFT_RECOVERY_GUID: &str = "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC";

/// Whether a GPT type GUID marks a partition we must never overwrite.
fn is_system_partition_guid(guid: &str) -> bool {
    let g = guid
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_uppercase();
    g == EFI_SYSTEM_PARTITION_GUID || g == MICROSOFT_RESERVED_GUID || g == MICROSOFT_RECOVERY_GUID
}

/// Refuse a target whose current partition table carries an EFI system, Microsoft
/// reserved, or recovery partition.
///
/// This is the **real** guard for the write path: once Ferrus writes partition
/// tables, the transport gate (removable-only) is no longer the sole line of
/// defense, so before any destructive step the target's existing layout is
/// inspected and a disk that looks like a boot/system disk is refused outright.
///
/// # Errors
///
/// [`Error::UnsafeTarget`] if any type GUID is a system/reserved/recovery type.
pub fn refuse_if_system_partition(type_guids: &[String]) -> Result<()> {
    if type_guids.iter().any(|g| is_system_partition_guid(g)) {
        return Err(Error::UnsafeTarget(
            "target disk carries an EFI system / reserved / recovery partition — refusing to \
             repartition it"
                .to_owned(),
        ));
    }
    Ok(())
}

// --- The injectable backend ---------------------------------------------------

/// Host operations the Windows partitioning flow needs, behind a trait so the
/// orchestration is testable with a fake. The real implementation is in
/// `platform::windows`; every method there is a native IOCTL or a shell-out.
pub trait WinPartitionBackend {
    /// Whether the process is elevated (Administrator). Writing a partition table
    /// requires it — the Windows equivalent of the Linux `EUID == 0` gate.
    ///
    /// # Errors
    /// If the process token cannot be queried.
    fn is_elevated(&self) -> Result<bool>;

    /// Live re-check that `disk` does not back the running system (defense in
    /// depth; `SafeTarget` already guarantees it).
    ///
    /// # Errors
    /// If system state cannot be determined.
    fn is_system_or_critical(&self, disk: &Path) -> Result<bool>;

    /// The GPT partition **type** GUIDs currently on `disk`, for the ESP guard.
    ///
    /// # Errors
    /// If the current layout cannot be read.
    fn read_partition_type_guids(&self, disk: &Path) -> Result<Vec<String>>;

    /// Lock + dismount the disk's volumes, write the GPT `layout`, and make the
    /// system re-read it (one atomic destructive step from the caller's view).
    /// `disk_size_bytes` bounds the GPT usable range.
    ///
    /// # Errors
    /// If any step fails; the disk is left as the OS reports it.
    fn write_gpt_layout(&self, disk: &Path, layout: &GptLayout, disk_size_bytes: u64)
    -> Result<()>;

    /// Format the `partition_number`-th (1-based) partition of `disk`.
    ///
    /// # Errors
    /// If the format tool fails.
    fn format_partition(
        &self,
        disk: &Path,
        partition_number: u32,
        fs: FsKind,
        label: &str,
    ) -> Result<()>;
}

/// Partition + format a Windows install target (SPEC-00010). Copy and bootloader
/// are 6.2b / 6.2c and are **not** performed here.
///
/// Ordering (SPEC-00010): plan → (dry-run stops here) → require Administrator →
/// re-check not-system → **ESP guard** → write GPT (lock/dismount/create/set/
/// rescan inside the backend) → format P1 NTFS. P2 is left raw.
///
/// # Errors
///
/// [`Error::PrivilegeRequired`] if not elevated, [`Error::UnsafeTarget`] if the
/// target looks like a system disk, or any backend error.
pub(crate) fn prepare_partition(
    target: &SafeTarget,
    progress: &mut dyn ProgressSink,
    backend: &dyn WinPartitionBackend,
) -> Result<()> {
    let device = target.device();
    let layout = compute_windows_layout(device.size_bytes)?;

    progress.stage(Stage::Partitioning);
    for (i, part) in layout.partitions().iter().enumerate() {
        progress.message(&format!(
            "plan P{}: {} {} at {} (type {})",
            i + 1,
            fs_label(part.fs),
            format_size(part.size_bytes),
            format_size(part.start_bytes),
            part.type_guid,
        ));
    }

    if target.is_dry_run() {
        progress.message("dry-run: no lock, dismount, table write, or format");
        return Ok(());
    }

    // Fail fast, before any destructive or locking step.
    if !backend.is_elevated()? {
        return Err(Error::PrivilegeRequired(
            "writing a partition table requires Administrator".to_owned(),
        ));
    }

    // Defense in depth (SafeTarget already guarantees this).
    if backend.is_system_or_critical(&device.path)? {
        return Err(Error::UnsafeTarget(format!(
            "{} backs the system or a critical mount — aborting",
            device.path.display()
        )));
    }

    // The real write-path guard: refuse a disk that already carries a
    // boot/system partition (ESP / MSR / recovery).
    let existing = backend.read_partition_type_guids(&device.path)?;
    refuse_if_system_partition(&existing)?;

    progress.message("writing GPT partition table");
    backend.write_gpt_layout(&device.path, &layout, device.size_bytes)?;

    progress.stage(Stage::Formatting);
    progress.message(&format!(
        "formatting partition 1 as {}",
        layout.windows.name
    ));
    backend.format_partition(&device.path, 1, layout.windows.fs, layout.windows.name)?;

    progress.message("partitioned; P1 formatted NTFS (P2 left raw for the bootloader)");
    Ok(())
}

fn fs_label(fs: FsKind) -> &'static str {
    match fs {
        FsKind::Ntfs => "NTFS",
        FsKind::Fat => "FAT",
    }
}

// --- Pure format-tool helpers (shared with the real backend, tested here) ------

/// PowerShell filesystem token for `Format-Volume -FileSystem`.
pub(crate) fn powershell_fs_token(fs: FsKind) -> &'static str {
    match fs {
        FsKind::Ntfs => "NTFS",
        FsKind::Fat => "FAT",
    }
}

/// Build the `powershell.exe` argument vector that formats a partition by its
/// disk + partition number (no drive letter required), non-interactively.
pub(crate) fn powershell_format_args(
    disk_number: u32,
    partition_number: u32,
    fs: FsKind,
    label: &str,
) -> Vec<String> {
    // Label is one of our own constants ("NTFS"/"FAT"); guard anyway by dropping
    // single quotes so the command cannot be broken out of.
    let safe_label = label.replace('\'', "");
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         Get-Partition -DiskNumber {disk_number} -PartitionNumber {partition_number} | \
         Format-Volume -FileSystem {fs} -NewFileSystemLabel '{safe_label}' -Confirm:$false -Force \
         | Out-Null",
        fs = powershell_fs_token(fs),
    );
    vec![
        "-NoProfile".to_owned(),
        "-NonInteractive".to_owned(),
        "-Command".to_owned(),
        script,
    ]
}

/// Interpret a format tool result: nonzero exit → a `ToolFailed` error carrying
/// the tool's stderr.
pub(crate) fn interpret_format_status(success: bool, stderr: &str) -> Result<()> {
    if success {
        return Ok(());
    }
    Err(Error::Tool {
        tool: "Format-Volume".to_owned(),
        reason: stderr.trim().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::partition::MICROSOFT_BASIC_DATA_GUID;

    #[test]
    fn esp_guard_allows_plain_basic_data_disk() {
        let types = vec![MICROSOFT_BASIC_DATA_GUID.to_owned()];
        assert!(refuse_if_system_partition(&types).is_ok());
        assert!(refuse_if_system_partition(&[]).is_ok());
    }

    #[test]
    fn esp_guard_refuses_system_partitions() {
        // ESP, MSR, recovery — each alone triggers a refusal, case-insensitive and
        // tolerant of `{...}` braces.
        for g in [
            EFI_SYSTEM_PARTITION_GUID.to_lowercase(),
            format!("{{{MICROSOFT_RESERVED_GUID}}}"),
            MICROSOFT_RECOVERY_GUID.to_owned(),
        ] {
            let types = vec![MICROSOFT_BASIC_DATA_GUID.to_owned(), g.clone()];
            let err = refuse_if_system_partition(&types).unwrap_err();
            assert!(matches!(err, Error::UnsafeTarget(_)), "should refuse {g}");
        }
    }

    #[test]
    fn powershell_command_is_non_interactive_and_targets_the_partition() {
        let args = powershell_format_args(2, 1, FsKind::Ntfs, "NTFS");
        assert!(args.contains(&"-NonInteractive".to_owned()));
        let script = args.last().unwrap();
        assert!(script.contains("Get-Partition -DiskNumber 2 -PartitionNumber 1"));
        assert!(script.contains("-FileSystem NTFS"));
        assert!(script.contains("-Confirm:$false"));
        assert!(script.contains("-Force"));
        // FAT maps to the FAT token.
        assert!(
            powershell_format_args(0, 2, FsKind::Fat, "FAT")
                .last()
                .unwrap()
                .contains("-FileSystem FAT")
        );
    }

    #[test]
    fn format_status_maps_failure_to_tool_error() {
        assert!(interpret_format_status(true, "").is_ok());
        let err = interpret_format_status(false, "  access denied\n").unwrap_err();
        match err {
            Error::Tool { tool, reason } => {
                assert_eq!(tool, "Format-Volume");
                assert_eq!(reason, "access denied");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    // --- Orchestration ordering + the dry-run contract, via a fake backend -----

    use std::cell::RefCell;

    use crate::device::{Bus, Device};
    use crate::progress::NullProgress;

    struct FakeWin {
        elevated: bool,
        system_critical: bool,
        existing_types: Vec<String>,
        log: RefCell<Vec<String>>,
    }

    impl FakeWin {
        fn new() -> Self {
            Self {
                elevated: true,
                system_critical: false,
                existing_types: vec![MICROSOFT_BASIC_DATA_GUID.to_owned()],
                log: RefCell::new(Vec::new()),
            }
        }
        fn rec(&self, s: impl Into<String>) {
            self.log.borrow_mut().push(s.into());
        }
    }

    impl WinPartitionBackend for FakeWin {
        fn is_elevated(&self) -> Result<bool> {
            self.rec("is_elevated");
            Ok(self.elevated)
        }
        fn is_system_or_critical(&self, _disk: &Path) -> Result<bool> {
            self.rec("is_system");
            Ok(self.system_critical)
        }
        fn read_partition_type_guids(&self, _disk: &Path) -> Result<Vec<String>> {
            self.rec("read_types");
            Ok(self.existing_types.clone())
        }
        fn write_gpt_layout(
            &self,
            _disk: &Path,
            _layout: &GptLayout,
            _disk_size_bytes: u64,
        ) -> Result<()> {
            self.rec("write_gpt");
            Ok(())
        }
        fn format_partition(&self, _disk: &Path, n: u32, fs: FsKind, _label: &str) -> Result<()> {
            self.rec(format!("format:{n}:{fs:?}"));
            Ok(())
        }
    }

    fn target(dry_run: bool) -> SafeTarget {
        let device = Device {
            path: std::path::PathBuf::from(r"\\.\PhysicalDrive2"),
            stable_id: None,
            model: Some("Test USB".to_owned()),
            bus: Bus::Usb,
            size_bytes: 8 * 1024 * 1024 * 1024,
            removable: true,
            is_system_or_critical: false,
        };
        SafeTarget::new_for_test(device, dry_run)
    }

    #[test]
    fn dry_run_locks_writes_and_formats_nothing() {
        let backend = FakeWin::new();
        prepare_partition(&target(true), &mut NullProgress, &backend).unwrap();
        // Not even the elevation probe runs in dry-run: only the plan is printed.
        assert!(backend.log.borrow().is_empty());
    }

    #[test]
    fn write_requires_administrator() {
        let backend = FakeWin {
            elevated: false,
            ..FakeWin::new()
        };
        let err = prepare_partition(&target(false), &mut NullProgress, &backend).unwrap_err();
        assert!(matches!(err, Error::PrivilegeRequired(_)));
        assert!(!backend.log.borrow().contains(&"write_gpt".to_owned()));
    }

    #[test]
    fn esp_on_target_blocks_before_any_write() {
        let backend = FakeWin {
            existing_types: vec![
                MICROSOFT_BASIC_DATA_GUID.to_owned(),
                EFI_SYSTEM_PARTITION_GUID.to_owned(),
            ],
            ..FakeWin::new()
        };
        let err = prepare_partition(&target(false), &mut NullProgress, &backend).unwrap_err();
        assert!(matches!(err, Error::UnsafeTarget(_)));
        let log = backend.log.borrow();
        assert!(log.contains(&"read_types".to_owned()));
        assert!(!log.contains(&"write_gpt".to_owned()));
        assert!(!log.iter().any(|e| e.starts_with("format")));
    }

    #[test]
    fn happy_path_runs_checks_then_write_then_format_p1() {
        let backend = FakeWin::new();
        prepare_partition(&target(false), &mut NullProgress, &backend).unwrap();
        assert_eq!(
            *backend.log.borrow(),
            vec![
                "is_elevated",
                "is_system",
                "read_types",
                "write_gpt",
                "format:1:Ntfs",
            ]
        );
    }
}
