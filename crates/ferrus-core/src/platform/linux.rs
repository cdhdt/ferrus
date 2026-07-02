//! Linux backend (the first-class, implemented platform).
//!
//! Device enumeration will read from sysfs (`/sys/block`) and/or `lsblk`, and
//! mount/critical-state checks from `/proc/mounts` and swap info. In Phase 0
//! these are documented stubs.

use std::path::Path;

use crate::Result;
use crate::device::Device;
use crate::platform::Backend;

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
        // TODO(phase1): read /sys/block, resolve size (`.../size` * 512),
        // `removable` flag, model (`.../device/model`), and mark
        // is_system_or_critical by cross-referencing /proc/mounts and swaps.
        todo!("Linux device enumeration lands in Phase 1")
    }

    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool> {
        // TODO(phase1): return true if any partition of `device_path` backs /,
        // /boot, swap, or another mount we treat as critical.
        let _ = device_path;
        todo!("Linux mount-state check lands in Phase 1")
    }
}
