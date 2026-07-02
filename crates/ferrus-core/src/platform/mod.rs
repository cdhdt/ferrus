//! Operating-system abstraction.
//!
//! All OS-specific behavior is funneled through the [`Backend`] trait, selected
//! at compile time with `#[cfg(...)]`. There are deliberately **no** scattered
//! `if os == ...` checks elsewhere in the crate — code depends on [`Backend`],
//! and [`backend`] returns the one compiled in for the current target.
//!
//! Only the Linux backend is implemented; Windows and macOS are stubs (Phases
//! 6 and 7).

use std::path::Path;

use crate::Result;
use crate::device::Device;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Host-specific operations the engine needs.
///
/// Implementations must uphold the safety contract of [`crate::device`]: in
/// particular [`Backend::enumerate_devices`] must report `removable` and
/// `is_system_or_critical` accurately, since the safety checkpoint relies on
/// them.
pub trait Backend {
    /// Enumerate all block devices visible to the host.
    ///
    /// # Errors
    ///
    /// Returns an error if the device inventory cannot be read.
    fn enumerate_devices(&self) -> Result<Vec<Device>>;

    /// Re-check, at the moment of use, whether `device_path` currently hosts the
    /// running system or a critical mount. Used to close the TOCTOU gap in the
    /// safety checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an error if mount state cannot be determined.
    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool>;
}

/// Returns the [`Backend`] for the current compilation target.
///
/// # Errors
///
/// Returns [`Error::Unsupported`](crate::Error::Unsupported) when compiled for
/// an OS that does not yet have a backend.
pub fn backend() -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LinuxBackend::new()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        // TODO(phase6/7): return the Windows/macOS backend once implemented.
        Err(crate::Error::Unsupported(
            "no device backend for this OS yet".to_owned(),
        ))
    }
}
