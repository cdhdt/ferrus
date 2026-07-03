//! Pure GPT layout geometry and the `sfdisk` script for Windows install media.
//!
//! No I/O — all functions here are pure and unit tested (see `tests.rs`). See
//! SPEC-0003 for the layout rationale and the verified partition type GUIDs.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::FsKind;
use crate::{Error, Result};

/// One mebibyte in bytes.
pub const MIB: u64 = 1024 * 1024;

/// Minimum device size we accept for the Windows layout, so P1 is a sane,
/// formattable size. (Real Windows needs far more; that is a Phase 3b concern.)
pub const MIN_DEVICE_BYTES: u64 = 64 * MIB;

/// GPT type GUID for a **Microsoft basic data** partition.
///
/// Both P1 (NTFS) and P2 (UEFI:NTFS helper) use this. P2 is deliberately NOT an
/// EFI System Partition — verified against Rufus `src/drive.c` (see SPEC-0003).
pub const MICROSOFT_BASIC_DATA_GUID: &str = "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7";

/// A planned partition. Offsets and sizes are byte values, all 1 MiB-aligned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPartition {
    /// Start offset in bytes (1 MiB-aligned).
    pub start_bytes: u64,
    /// Size in bytes (1 MiB-aligned).
    pub size_bytes: u64,
    /// Filesystem to create on it.
    pub fs: FsKind,
    /// GPT partition type GUID.
    pub type_guid: &'static str,
    /// GPT partition name / intended filesystem label.
    pub name: &'static str,
}

/// The two-partition GPT layout for a Windows install stick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GptLayout {
    /// P1: the large NTFS partition holding the Windows files (Phase 3b).
    pub windows: PlannedPartition,
    /// P2: the small FAT helper at the end for UEFI:NTFS (Phase 3c).
    pub helper: PlannedPartition,
}

impl GptLayout {
    /// The partitions in on-disk order.
    #[must_use]
    pub fn partitions(&self) -> [&PlannedPartition; 2] {
        [&self.windows, &self.helper]
    }

    /// Serialize to an `sfdisk` script. Sizes are emitted in MiB so `sfdisk`
    /// converts them using the device's real logical sector size.
    #[must_use]
    pub fn sfdisk_script(&self) -> String {
        let mut s = String::from("label: gpt\n");
        for p in self.partitions() {
            // All offsets/sizes are whole MiB by construction.
            let _ = writeln!(
                s,
                "start={}MiB, size={}MiB, type={}, name=\"{}\"",
                p.start_bytes / MIB,
                p.size_bytes / MIB,
                p.type_guid,
                p.name,
            );
        }
        s
    }
}

/// Compute the 1 MiB-aligned GPT layout for a device of `device_bytes`.
///
/// Reserves 1 MiB at the front (protective MBR + primary GPT), 1 MiB at the back
/// (backup GPT), and a 1 MiB FAT helper immediately before the back reserve; P1
/// fills the middle.
///
/// # Errors
///
/// Returns [`Error::DeviceTooSmall`] when the device is below
/// [`MIN_DEVICE_BYTES`].
pub fn compute_windows_layout(device_bytes: u64) -> Result<GptLayout> {
    if device_bytes < MIN_DEVICE_BYTES {
        return Err(Error::DeviceTooSmall {
            device_bytes,
            minimum_bytes: MIN_DEVICE_BYTES,
        });
    }

    let device_mib = device_bytes / MIB; // floor to whole MiB
    let front_mib = 1;
    let back_mib = 1;
    let helper_mib = 1;

    let windows_start = front_mib;
    let windows_size = device_mib - front_mib - back_mib - helper_mib;
    let helper_start = windows_start + windows_size;

    Ok(GptLayout {
        windows: PlannedPartition {
            start_bytes: windows_start * MIB,
            size_bytes: windows_size * MIB,
            fs: FsKind::Ntfs,
            type_guid: MICROSOFT_BASIC_DATA_GUID,
            name: "Windows",
        },
        helper: PlannedPartition {
            start_bytes: helper_start * MIB,
            size_bytes: helper_mib * MIB,
            fs: FsKind::Fat,
            type_guid: MICROSOFT_BASIC_DATA_GUID,
            name: "UEFI_NTFS",
        },
    })
}

/// The device-node path of the `index`-th partition of `device_path`.
///
/// Handles both `sdX` → `sdX1` and NVMe/MMC `…N` → `…Np1` naming.
#[must_use]
pub fn partition_path(device_path: &Path, index: usize) -> PathBuf {
    let base = device_path.to_string_lossy();
    let needs_p = base.chars().next_back().is_some_and(|c| c.is_ascii_digit());
    if needs_p {
        PathBuf::from(format!("{base}p{index}"))
    } else {
        PathBuf::from(format!("{base}{index}"))
    }
}
