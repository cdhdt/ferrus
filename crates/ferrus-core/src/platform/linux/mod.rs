//! Linux backend (the first-class, implemented platform).
//!
//! Device enumeration reads sysfs (`/sys/block`, `/sys/class/block`) and
//! mount/critical-state from `/proc/mounts` and `/proc/swaps`. No external
//! commands and no C libraries are used — see the report/spec for the
//! sysfs-vs-`udev` decision.
//!
//! This module holds the I/O and orchestration; the parsing logic lives in
//! [`sysfs`] (pure) and [`mounts`] (resolution behind a testable trait).

mod mount;
mod mounts;
mod partition;
mod sysfs;
mod write;

use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::device::Device;
use crate::platform::Backend;

use mounts::{RealBlockFs, build_system_disk_set};
use sysfs::{
    bus_from_syspath, flag_is_true, is_virtual_name, pick_stable_id, size_from_sectors,
};

/// Linux implementation of [`Backend`].
#[derive(Debug, Default)]
pub struct LinuxBackend {
    _private: (),
}

impl LinuxBackend {
    /// Create a new Linux backend.
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Backend for LinuxBackend {
    fn enumerate_devices(&self) -> Result<Vec<Device>> {
        let system_disks = current_system_disks();
        let by_id = read_by_id_entries();

        let mut devices = Vec::new();
        for entry in fs::read_dir("/sys/block")? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if is_virtual_name(&name) {
                continue;
            }
            let base = format!("/sys/block/{name}");

            // Physical block devices expose a `device` symlink; virtual ones
            // (dm, loop, md, zram, …) do not. This is the primary exclusion.
            if !Path::new(&format!("{base}/device")).exists() {
                continue;
            }

            let size_bytes = read_trimmed(&format!("{base}/size"))
                .as_deref()
                .and_then(size_from_sectors)
                .unwrap_or(0);
            if size_bytes == 0 {
                // Empty card reader / no media inserted.
                continue;
            }

            let removable = read_trimmed(&format!("{base}/removable"))
                .as_deref()
                .map(flag_is_true)
                .unwrap_or(false);

            let syspath = fs::canonicalize(&base)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();

            devices.push(Device {
                path: PathBuf::from(format!("/dev/{name}")),
                stable_id: pick_stable_id(&by_id, &name),
                model: read_model(&base),
                bus: bus_from_syspath(&syspath),
                size_bytes,
                removable,
                is_system_or_critical: system_disks.contains(&name),
            });
        }

        devices.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(devices)
    }

    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool> {
        Ok(disk_is_system_or_critical(device_path))
    }
}

/// Read the live set of system/critical disk names from `/proc`.
fn current_system_disks() -> std::collections::BTreeSet<String> {
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let swaps = fs::read_to_string("/proc/swaps").unwrap_or_default();
    build_system_disk_set(&mounts, &swaps, &RealBlockFs)
}

/// Whether the whole disk at `device_path` currently backs the system or a
/// critical mount. Shared by both the enumeration and write backends.
pub(super) fn disk_is_system_or_critical(device_path: &Path) -> bool {
    match device_path.file_name() {
        Some(name) => current_system_disks().contains(&name.to_string_lossy().into_owned()),
        None => false,
    }
}

/// Read a sysfs file and trim it, returning `None` on any error.
fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// Build a model string from `device/vendor` and `device/model`, trimming the
/// padding whitespace sysfs includes. Returns `None` when neither is present.
fn read_model(base: &str) -> Option<String> {
    let vendor = read_trimmed(&format!("{base}/device/vendor")).filter(|s| !s.is_empty());
    let model = read_trimmed(&format!("{base}/device/model")).filter(|s| !s.is_empty());
    match (vendor, model) {
        (Some(vendor), Some(model)) => Some(format!("{vendor} {model}")),
        (Some(single), None) | (None, Some(single)) => Some(single),
        (None, None) => None,
    }
}

/// Gather `/dev/disk/by-id` as `(link name, resolved block name)` pairs so a
/// stable identifier can be chosen per device.
fn read_by_id_entries() -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir("/dev/disk/by-id") else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let link = entry.file_name().to_string_lossy().into_owned();
        if let Ok(target) = fs::canonicalize(entry.path())
            && let Some(target_name) = target.file_name()
        {
            pairs.push((link, target_name.to_string_lossy().into_owned()));
        }
    }
    pairs
}

#[cfg(test)]
mod tests;
