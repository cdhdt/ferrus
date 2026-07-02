//! macOS backend — **stub** (Phase 7).
//!
//! Device enumeration will go through IOKit / `diskutil`. Not implemented yet;
//! this module only exists so the abstraction is real from day one.
#![allow(dead_code)]

use std::path::Path;

use crate::Result;
use crate::device::Device;
use crate::platform::Backend;

/// macOS implementation of [`Backend`] (not yet implemented).
#[derive(Debug, Default)]
pub struct MacosBackend {
    _private: (),
}

impl Backend for MacosBackend {
    fn enumerate_devices(&self) -> Result<Vec<Device>> {
        // TODO(phase7): enumerate disks via IOKit / `diskutil list -plist`.
        Err(crate::Error::Unsupported(
            "macOS backend not implemented".to_owned(),
        ))
    }

    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool> {
        // TODO(phase7): check whether the disk backs the system volume group.
        let _ = device_path;
        Err(crate::Error::Unsupported(
            "macOS backend not implemented".to_owned(),
        ))
    }
}
