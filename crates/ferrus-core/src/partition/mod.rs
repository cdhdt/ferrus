//! Partitioning + formatting for Windows install media (Phase 3a, SPEC-0003).
//!
//! [`prepare_windows`] is the destructive entry point: it wipes the target's
//! partition table, writes a GPT (a large NTFS partition + a small FAT helper at
//! the end), and formats both. It does **not** copy files (3b), install the
//! UEFI:NTFS bootloader (3c), or apply Windows tweaks (Phase 4) — a stick it
//! produces does not boot yet.
//!
//! Layout:
//!
//! - [`plan`] — pure geometry and the `sfdisk` script (unit tested).
//! - orchestration ([`prepare_windows`]) — the destructive sequence with the OS
//!   backend injected for testability.

mod plan;

pub use plan::{
    GptLayout, MICROSOFT_BASIC_DATA_GUID, MIN_DEVICE_BYTES, PlannedPartition,
    compute_windows_layout, partition_path,
};

use crate::device::{SafeTarget, format_size};
use crate::platform::PartitionBackend;
use crate::progress::{ProgressSink, Stage};
use crate::{Error, Result};

/// Partition-table style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableScheme {
    /// GUID Partition Table (used for UEFI boot).
    Gpt,
    /// Master Boot Record (used for Legacy BIOS boot).
    Mbr,
}

/// Intended firmware boot style for the produced media.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootStyle {
    /// UEFI firmware.
    Uefi,
    /// Legacy BIOS / CSM.
    Legacy,
}

/// Filesystem to create on a partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsKind {
    /// NTFS — required for Windows install media (`install.wim` > 4 GB).
    Ntfs,
    /// FAT — used for the small UEFI:NTFS helper partition.
    Fat,
}

/// External tools the Windows-prepare path needs, checked before any write.
const REQUIRED_TOOLS: [&str; 4] = ["sfdisk", "mkfs.ntfs", "mkfs.vfat", "partprobe"];

/// Partition and format `target` as the skeleton of a Windows install stick.
///
/// Takes a [`SafeTarget`], so it cannot be reached without the SPEC-0001
/// checkpoint. Honors [`SafeTarget::is_dry_run`]: in dry-run it only reports the
/// intended layout.
///
/// # Errors
///
/// Returns [`Error::DeviceTooSmall`], [`Error::PrivilegeRequired`],
/// [`Error::MissingTool`], [`Error::UnsafeTarget`],
/// [`Error::PartitionNodesMissing`], [`Error::Tool`], or [`Error::Io`].
pub fn prepare_windows(target: &SafeTarget, progress: &mut dyn ProgressSink) -> Result<()> {
    let backend = crate::platform::partition_backend()?;
    prepare_windows_with(target, progress, backend.as_ref())
}

/// The orchestration with the backend injected, for testing. See SPEC-0003 for
/// the exact ordering and rationale.
fn prepare_windows_with(
    target: &SafeTarget,
    progress: &mut dyn ProgressSink,
    backend: &dyn PartitionBackend,
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
        progress.message("dry-run: no table written, nothing formatted");
        return Ok(());
    }

    // Fail fast, before any destructive step.
    if backend.effective_uid()? != 0 {
        return Err(Error::PrivilegeRequired(
            "partitioning a device requires root".to_owned(),
        ));
    }
    backend.ensure_tools(&REQUIRED_TOOLS)?;

    // Defense in depth (SafeTarget already guarantees this).
    if backend.is_system_or_critical(&device.path)? {
        return Err(Error::UnsafeTarget(format!(
            "{} backs the system or a critical mount — aborting",
            device.path.display()
        )));
    }

    for mountpoint in backend.mounted_partitions(&device.path)? {
        progress.message(&format!("unmounting {}", mountpoint.display()));
        backend.unmount(&mountpoint)?;
    }

    progress.message("writing GPT partition table");
    backend.write_partition_table(&device.path, &layout.sfdisk_script())?;

    progress.stage(Stage::Formatting);
    progress.message("re-reading partition table");
    backend.reread_partition_table(&device.path)?;
    let nodes = backend.wait_for_partitions(&device.path, 2)?;
    let (Some(p1), Some(p2)) = (nodes.first(), nodes.get(1)) else {
        return Err(Error::PartitionNodesMissing {
            device: device.path.clone(),
            expected: 2,
        });
    };

    progress.message(&format!("formatting {} as NTFS", p1.display()));
    backend.make_filesystem(p1, layout.windows.fs, layout.windows.name)?;
    progress.message(&format!("formatting {} as FAT", p2.display()));
    backend.make_filesystem(p2, layout.helper.fs, layout.helper.name)?;

    progress.message(
        "done: partitioned and formatted. NOT bootable yet — files (3b) and \
         UEFI:NTFS bootloader (3c) come next.",
    );
    Ok(())
}

/// Short label for a filesystem kind (for progress messages).
fn fs_label(fs: FsKind) -> &'static str {
    match fs {
        FsKind::Ntfs => "NTFS",
        FsKind::Fat => "FAT",
    }
}

#[cfg(test)]
mod tests;
