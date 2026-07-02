//! Windows backend — **stub** (Phase 6).
//!
//! Device enumeration will go through the Win32 volume/disk APIs
//! (`SetupDiGetClassDevs`, `IOCTL_STORAGE_QUERY_PROPERTY`, …). Not implemented
//! yet; this module only exists so the abstraction is real from day one.
#![allow(dead_code)]

use std::path::Path;

use crate::Result;
use crate::device::Device;
use crate::platform::Backend;

/// Windows implementation of [`Backend`] (not yet implemented).
#[derive(Debug, Default)]
pub struct WindowsBackend {
    _private: (),
}

impl Backend for WindowsBackend {
    fn enumerate_devices(&self) -> Result<Vec<Device>> {
        // TODO(phase6): enumerate physical drives via the Win32 storage APIs.
        Err(crate::Error::Unsupported(
            "Windows backend not implemented".to_owned(),
        ))
    }

    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool> {
        // TODO(phase6): map the drive to volumes and check for the system drive.
        let _ = device_path;
        Err(crate::Error::Unsupported(
            "Windows backend not implemented".to_owned(),
        ))
    }
}
