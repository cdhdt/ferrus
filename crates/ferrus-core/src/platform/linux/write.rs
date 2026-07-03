//! Linux implementation of the destructive write path (SPEC-0002).
//!
//! Provides the real [`WriteSink`] (exclusive `O_EXCL` open + `fsync`), the
//! unmount (via `umount(8)`, since the `umount2` syscall would need `unsafe`),
//! the EUID gate, and mounted-partition discovery. The pure EUID parser is unit
//! tested in `tests.rs`; everything else is I/O.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::LinuxBackend;
use super::disk_is_system_or_critical;
use super::mounts::{RealBlockFs, mountpoints_backed_by};
use crate::platform::{WriteBackend, WriteSink};
use crate::{Error, Result};

/// Parse the effective UID from `/proc/self/status` contents. The `Uid:` line is
/// `real  effective  saved  fsuid` (tab-separated); the effective UID is the
/// second value.
pub(super) fn parse_effective_uid(status: &str) -> Option<u32> {
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            return rest.split_whitespace().nth(1).and_then(|v| v.parse().ok());
        }
    }
    None
}

/// Read the process's effective UID. Shared by the write and partition backends.
pub(super) fn read_effective_uid() -> Result<u32> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    parse_effective_uid(&status).ok_or_else(|| io::Error::other("could not parse effective uid").into())
}

/// Mountpoints of partitions currently mounted from `device_path`.
pub(super) fn list_mounted_partitions(device_path: &Path) -> Result<Vec<PathBuf>> {
    let Some(disk) = device_path.file_name() else {
        return Ok(Vec::new());
    };
    let disk = disk.to_string_lossy().into_owned();
    let mounts = std::fs::read_to_string("/proc/mounts")?;
    Ok(mountpoints_backed_by(&mounts, &disk, &RealBlockFs)
        .into_iter()
        .map(PathBuf::from)
        .collect())
}

/// Unmount the filesystem at `mountpoint` via `umount(8)`.
pub(super) fn run_umount(mountpoint: &Path) -> Result<()> {
    let status = Command::new("umount")
        .arg(mountpoint)
        .status()
        .map_err(|e| Error::Tool {
            tool: "umount".to_owned(),
            reason: e.to_string(),
        })?;
    if !status.success() {
        return Err(Error::Tool {
            tool: "umount".to_owned(),
            reason: format!("failed to unmount {}: {status}", mountpoint.display()),
        });
    }
    Ok(())
}

/// A [`WriteSink`] over an exclusively-opened block device.
struct FileSink {
    file: File,
}

impl WriteSink for FileSink {
    fn write_chunk(&mut self, buf: &[u8]) -> Result<()> {
        self.file.write_all(buf)?;
        Ok(())
    }

    fn sync(&mut self) -> Result<()> {
        // fsync: block until data is durably on the device (SPEC-0002 inv. 7).
        self.file.sync_all()?;
        Ok(())
    }
}

impl WriteBackend for LinuxBackend {
    fn effective_uid(&self) -> Result<u32> {
        read_effective_uid()
    }

    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool> {
        Ok(disk_is_system_or_critical(device_path))
    }

    fn mounted_partitions(&self, device_path: &Path) -> Result<Vec<PathBuf>> {
        list_mounted_partitions(device_path)
    }

    fn unmount(&self, mountpoint: &Path) -> Result<()> {
        run_umount(mountpoint)
    }

    fn open_exclusive_writer(&self, device_path: &Path) -> Result<Box<dyn WriteSink>> {
        // O_WRONLY | O_EXCL: on a block device (Linux 2.6+) O_EXCL without
        // O_CREAT fails with EBUSY if the device is in use — verified against
        // `man 2 open`.
        match OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_EXCL)
            .open(device_path)
        {
            Ok(file) => Ok(Box::new(FileSink { file })),
            Err(e) if e.raw_os_error() == Some(libc::EBUSY) => {
                Err(Error::DeviceBusy(device_path.to_path_buf()))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn open_reader(&self, device_path: &Path) -> Result<Box<dyn Read>> {
        Ok(Box::new(File::open(device_path)?))
    }
}
