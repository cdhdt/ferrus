//! Partitioning: choosing and applying a disk layout.
//!
//! Ferrus supports both UEFI (GPT) and Legacy BIOS (MBR) boot styles. For
//! Windows media the layout is a main **NTFS** partition plus a small **FAT**
//! helper partition carrying the UEFI:NTFS bootloader (see [`crate::boot`]).

use crate::Result;
use crate::device::SafeTarget;

/// Partition table style.
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
    /// FAT32 / FAT — used for the small UEFI:NTFS helper partition and for
    /// generic media whose files all fit the FAT32 limit.
    Fat,
}

/// A single planned partition.
#[derive(Debug, Clone)]
pub struct PartitionPlan {
    /// Filesystem to create on the partition.
    pub fs: FsKind,
    /// Size in bytes, or `None` for "fill remaining space".
    pub size_bytes: Option<u64>,
    /// Optional volume label.
    pub label: Option<String>,
}

/// A full layout to apply to a target device.
#[derive(Debug, Clone)]
pub struct LayoutPlan {
    /// Partition-table style.
    pub scheme: TableScheme,
    /// Intended firmware boot style.
    pub boot: BootStyle,
    /// Ordered list of partitions to create.
    pub partitions: Vec<PartitionPlan>,
}

/// Apply a layout plan to an authorized target.
///
/// Honors [`SafeTarget::is_dry_run`]: in dry-run mode this must describe the
/// intended layout without touching the device.
///
/// # Errors
///
/// Returns an error if the device cannot be repartitioned.
pub fn apply(target: &SafeTarget, plan: &LayoutPlan) -> Result<()> {
    // TODO(phase2/3): wipe existing signatures, write the partition table, and
    // create partitions per `plan`. Must be a no-op that only logs the plan when
    // `target.is_dry_run()` is true.
    let _ = (target, plan);
    todo!("partitioning lands in Phase 2/3")
}
