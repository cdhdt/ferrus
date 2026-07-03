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

use std::path::PathBuf;

use crate::device::{SafeTarget, format_size};
use crate::platform::PartitionBackend;
use crate::progress::{ProgressSink, Stage};
use crate::source::RawImage;
use crate::windows::WindowsTweaks;
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
// P2 is not formatted (the UEFI:NTFS image carries its own FAT — SPEC-0005), so
// mkfs.vfat is no longer required.
const REQUIRED_TOOLS: [&str; 3] = ["sfdisk", "mkfs.ntfs", "partprobe"];

/// Partition and format `target` as the skeleton of a Windows install stick,
/// and — when `image` is given — copy the Windows ISO contents onto it (3b).
///
/// Takes a [`SafeTarget`], so it cannot be reached without the SPEC-0001
/// checkpoint. Honors [`SafeTarget::is_dry_run`]: in dry-run it only reports the
/// intended layout and copy plan. `verify` enables the light post-copy check.
///
/// # Errors
///
/// Returns the partitioning errors ([`Error::DeviceTooSmall`],
/// [`Error::PrivilegeRequired`], [`Error::MissingTool`], [`Error::UnsafeTarget`],
/// [`Error::PartitionNodesMissing`], [`Error::Tool`], [`Error::Io`]) and, when
/// copying, the SPEC-0004 errors ([`Error::NotWindowsMedia`],
/// [`Error::InsufficientSpace`], [`Error::VerificationFailed`]).
pub fn prepare_windows(
    target: &SafeTarget,
    image: Option<&RawImage>,
    tweaks: Option<&WindowsTweaks>,
    verify: bool,
    progress: &mut dyn ProgressSink,
) -> Result<()> {
    let backend = crate::platform::partition_backend()?;
    let nodes = prepare_windows_with(target, progress, backend.as_ref())?;

    if let Some(image) = image {
        let layout = compute_windows_layout(target.device().size_bytes)?;
        let dev = &target.device().path;
        // In dry-run there are no real nodes; fall back to the intended paths.
        let p1 = nodes.first().cloned().unwrap_or_else(|| partition_path(dev, 1));
        let p2 = nodes.get(1).cloned().unwrap_or_else(|| partition_path(dev, 2));

        // 3b: copy the Windows files onto P1 (NTFS), plus autounattend.xml (Ph.4).
        crate::copy::copy_windows(
            image,
            &p1,
            layout.windows.size_bytes,
            tweaks,
            target.is_dry_run(),
            verify,
            progress,
        )?;

        // 3c: write the UEFI:NTFS bootloader onto P2 — makes the stick bootable.
        crate::boot::install_uefi_ntfs(
            &p2,
            layout.helper.size_bytes,
            target.is_dry_run(),
            progress,
        )?;
    }
    Ok(())
}

/// The orchestration with the backend injected, for testing. See SPEC-0003 for
/// the exact ordering and rationale.
fn prepare_windows_with(
    target: &SafeTarget,
    progress: &mut dyn ProgressSink,
    backend: &dyn PartitionBackend,
) -> Result<Vec<PathBuf>> {
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
        return Ok(Vec::new());
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
    let Some(p1) = nodes.first() else {
        return Err(Error::PartitionNodesMissing {
            device: device.path.clone(),
            expected: 2,
        });
    };

    // Only P1 is formatted here. P2 is left raw: the UEFI:NTFS image written in
    // 3c carries its own FAT filesystem (SPEC-0005), so an mkfs.vfat is redundant.
    progress.message(&format!("formatting {} as NTFS", p1.display()));
    backend.make_filesystem(p1, layout.windows.fs, layout.windows.name)?;

    progress.message("partitioned; P1 formatted NTFS (P2 left raw for the bootloader)");
    Ok(nodes)
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
