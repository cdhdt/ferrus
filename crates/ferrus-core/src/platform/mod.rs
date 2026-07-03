//! Operating-system abstraction.
//!
//! All OS-specific behavior is funneled through the [`Backend`] trait, selected
//! at compile time with `#[cfg(...)]`. There are deliberately **no** scattered
//! `if os == ...` checks elsewhere in the crate — code depends on [`Backend`],
//! and [`backend`] returns the one compiled in for the current target.
//!
//! Only the Linux backend is implemented; Windows and macOS are stubs (Phases
//! 6 and 7).

use std::path::{Path, PathBuf};

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

/// A destination the raw-copy loop writes to, decoupled from any concrete file
/// so the copy logic is testable against an in-memory sink.
///
/// Implementations wrap the exclusively-opened target device.
pub trait WriteSink {
    /// Write an entire chunk to the device.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying write fails.
    fn write_chunk(&mut self, buf: &[u8]) -> Result<()>;

    /// Flush all written data durably to the device (`fsync`). Must complete
    /// before any success is reported to the user.
    ///
    /// # Errors
    ///
    /// Returns an error if the sync fails.
    fn sync(&mut self) -> Result<()>;
}

/// Host-specific operations needed by the destructive write path (SPEC-0002),
/// kept behind a trait so the orchestration is testable with a fake.
pub trait WriteBackend {
    /// The effective UID of the current process (0 = root).
    ///
    /// # Errors
    ///
    /// Returns an error if it cannot be determined.
    fn effective_uid(&self) -> Result<u32>;

    /// Whether `device_path` currently backs the system or a critical mount
    /// (the same live check the safety checkpoint uses).
    ///
    /// # Errors
    ///
    /// Returns an error if mount state cannot be read.
    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool>;

    /// Mountpoints of partitions currently mounted from `device_path`.
    ///
    /// # Errors
    ///
    /// Returns an error if mount state cannot be read.
    fn mounted_partitions(&self, device_path: &Path) -> Result<Vec<PathBuf>>;

    /// Unmount the filesystem at `mountpoint`.
    ///
    /// # Errors
    ///
    /// Returns an error if the unmount fails.
    fn unmount(&self, mountpoint: &Path) -> Result<()>;

    /// Open the device for exclusive writing (`O_WRONLY | O_EXCL`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::DeviceBusy`](crate::Error::DeviceBusy) if the device is
    /// in use, or another error on failure.
    fn open_exclusive_writer(&self, device_path: &Path) -> Result<Box<dyn WriteSink>>;

    /// Open the device for reading back (post-write verification).
    ///
    /// # Errors
    ///
    /// Returns an error if the device cannot be opened.
    fn open_reader(&self, device_path: &Path) -> Result<Box<dyn std::io::Read>>;
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

/// Returns the [`WriteBackend`] for the current compilation target.
///
/// # Errors
///
/// Returns [`Error::Unsupported`](crate::Error::Unsupported) when compiled for
/// an OS that does not yet have a write backend.
pub fn write_backend() -> Result<Box<dyn WriteBackend>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LinuxBackend::new()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(crate::Error::Unsupported(
            "no write backend for this OS yet".to_owned(),
        ))
    }
}
