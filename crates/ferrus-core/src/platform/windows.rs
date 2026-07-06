//! Windows backend — device enumeration + safe target selection (SPEC-0009).
//!
//! The Windows equivalent of the Linux backend (SPEC-0001), with the **same**
//! safety guarantees: eligibility is decided on the storage **transport bus**
//! (only USB/SD/MMC qualify), never on a removable flag, and the disk backing the
//! Windows installation is refused. The unsafe Win32 FFI lives entirely in the
//! `ferrus-win32` crate; this module is safe glue that maps its plain descriptors
//! onto [`Device`] and reuses the shared decision logic in [`crate::device`].

use std::path::{Path, PathBuf};

use crate::device::{Bus, Device};
use crate::partition::windows::{
    WinPartitionBackend, interpret_format_status, powershell_format_args,
};
use crate::partition::{FsKind, GptLayout};
use crate::platform::Backend;
use crate::{Error, Result};

/// Windows implementation of [`Backend`].
#[derive(Debug, Default)]
pub struct WindowsBackend {
    _private: (),
}

impl Backend for WindowsBackend {
    fn enumerate_devices(&self) -> Result<Vec<Device>> {
        // Disks backing the Windows volume are marked critical; if that cannot be
        // resolved, propagate the error rather than list disks with no system
        // guard.
        let system = ferrus_win32::system_disk_numbers()?;
        let disks = ferrus_win32::enumerate_physical_disks()?;

        let mut devices: Vec<Device> = disks
            .into_iter()
            .map(|disk| Device {
                path: PathBuf::from(format!(r"\\.\PhysicalDrive{}", disk.number)),
                stable_id: None,
                model: disk.model,
                bus: Bus::from_windows_bus_type(disk.bus_type),
                size_bytes: disk.size_bytes,
                removable: disk.removable_media,
                is_system_or_critical: system.contains(&disk.number),
            })
            .collect();

        devices.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(devices)
    }

    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool> {
        match physical_drive_number(device_path) {
            // A well-formed \\.\PhysicalDriveN path: critical iff it backs Windows.
            Some(number) => Ok(ferrus_win32::system_disk_numbers()?.contains(&number)),
            // Anything else (a volume path, a letter, a typo): fail safe — refuse.
            None => Ok(true),
        }
    }
}

/// Parse the `N` from a `\\.\PhysicalDriveN` path. Pure; cross-compiled and unit
/// tested (the tests run when built for Windows).
fn physical_drive_number(path: &Path) -> Option<u32> {
    path.to_str()?
        .strip_prefix(r"\\.\PhysicalDrive")?
        .parse::<u32>()
        .ok()
}

/// Parse the drive number, or a typed error for a non-physical-drive path.
fn require_drive_number(path: &Path) -> Result<u32> {
    physical_drive_number(path).ok_or_else(|| {
        Error::UnsafeTarget(format!(
            "{} is not a \\\\.\\PhysicalDrive path",
            path.display()
        ))
    })
}

/// Windows implementation of [`WinPartitionBackend`] (SPEC-00010, Phase 6.2a).
#[derive(Debug, Default)]
pub struct WindowsPartitionBackend {
    _private: (),
}

impl WinPartitionBackend for WindowsPartitionBackend {
    fn is_elevated(&self) -> Result<bool> {
        Ok(ferrus_win32::is_process_elevated()?)
    }

    fn is_system_or_critical(&self, disk: &Path) -> Result<bool> {
        let Some(number) = physical_drive_number(disk) else {
            return Ok(true); // fail closed
        };
        Ok(ferrus_win32::system_disk_numbers()?.contains(&number))
    }

    fn read_partition_type_guids(&self, disk: &Path) -> Result<Vec<String>> {
        let number = require_drive_number(disk)?;
        Ok(ferrus_win32::read_partition_type_guids(number)?)
    }

    fn write_gpt_layout(
        &self,
        disk: &Path,
        layout: &GptLayout,
        disk_size_bytes: u64,
    ) -> Result<()> {
        let number = require_drive_number(disk)?;
        let specs = layout_to_specs(layout);
        ferrus_win32::write_gpt_layout(number, disk_size_bytes, &specs)?;
        Ok(())
    }

    fn format_partition(
        &self,
        disk: &Path,
        partition_number: u32,
        fs: FsKind,
        label: &str,
    ) -> Result<()> {
        let number = require_drive_number(disk)?;
        let args = powershell_format_args(number, partition_number, fs, label);
        let output = std::process::Command::new("powershell.exe")
            .args(&args)
            .output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        interpret_format_status(output.status.success(), &stderr)
    }
}

/// Map the shared [`GptLayout`] geometry to the win32 partition specs.
fn layout_to_specs(layout: &GptLayout) -> Vec<ferrus_win32::GptPartitionSpec> {
    layout
        .partitions()
        .iter()
        .enumerate()
        .map(|(i, p)| ferrus_win32::GptPartitionSpec {
            start_bytes: p.start_bytes,
            size_bytes: p.size_bytes,
            type_guid: p.type_guid.to_owned(),
            partition_number: (i + 1) as u32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_physical_drive_number() {
        assert_eq!(
            physical_drive_number(Path::new(r"\\.\PhysicalDrive0")),
            Some(0)
        );
        assert_eq!(
            physical_drive_number(Path::new(r"\\.\PhysicalDrive12")),
            Some(12)
        );
    }

    #[test]
    fn rejects_non_physical_drive_paths() {
        // A volume, a bare letter, or junk is not a physical-drive path → None,
        // which the backend treats as "critical, refuse".
        for p in [r"\\.\C:", r"C:\", r"\\.\PhysicalDriveX", "/dev/sda", ""] {
            assert_eq!(physical_drive_number(Path::new(p)), None, "{p}");
        }
    }
}
