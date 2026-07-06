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
use crate::partition::FsKind;

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
    #[cfg(target_os = "windows")]
    {
        // Enumeration + safe selection (SPEC-0009). Partitioning/writing land in
        // later Phase 6 steps.
        Ok(Box::new(windows::WindowsBackend::default()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        // TODO(phase7): return the macOS backend once implemented.
        Err(crate::Error::Unsupported(
            "no device backend for this OS yet".to_owned(),
        ))
    }
}

/// The current process's effective UID (`0` == root).
///
/// Exposed for the privileged helper (SPEC-0008), which asserts it is actually
/// elevated before doing anything. The destructive engine paths enforce root
/// internally too; this is the same check, made available at the boundary.
///
/// # Errors
///
/// Returns [`Error::Unsupported`](crate::Error::Unsupported) on an OS without a
/// backend, or an I/O error if the UID cannot be read.
pub fn effective_uid() -> Result<u32> {
    #[cfg(target_os = "linux")]
    {
        linux::effective_uid()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(crate::Error::Unsupported(
            "effective uid unavailable on this OS".to_owned(),
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

/// Host-specific operations for partitioning and formatting (SPEC-0003), behind
/// a trait so the orchestration is testable with a fake.
pub trait PartitionBackend {
    /// Effective UID of the current process (0 = root).
    ///
    /// # Errors
    /// Returns an error if it cannot be determined.
    fn effective_uid(&self) -> Result<u32>;

    /// Whether `device_path` currently backs the system or a critical mount.
    ///
    /// # Errors
    /// Returns an error if mount state cannot be read.
    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool>;

    /// Mountpoints of partitions currently mounted from `device_path`.
    ///
    /// # Errors
    /// Returns an error if mount state cannot be read.
    fn mounted_partitions(&self, device_path: &Path) -> Result<Vec<PathBuf>>;

    /// Unmount the filesystem at `mountpoint`.
    ///
    /// # Errors
    /// Returns an error if the unmount fails.
    fn unmount(&self, mountpoint: &Path) -> Result<()>;

    /// Verify every named tool is installed, before any destructive step.
    ///
    /// # Errors
    /// Returns [`Error::MissingTool`](crate::Error::MissingTool) for the first
    /// absent tool.
    fn ensure_tools(&self, tools: &[&str]) -> Result<()>;

    /// Write a partition table to `device_path` from an `sfdisk` script.
    ///
    /// # Errors
    /// Returns an error if the table cannot be written.
    fn write_partition_table(&self, device_path: &Path, script: &str) -> Result<()>;

    /// Ask the kernel to re-read the partition table of `device_path`.
    ///
    /// # Errors
    /// Returns an error if the re-read fails.
    fn reread_partition_table(&self, device_path: &Path) -> Result<()>;

    /// Wait (bounded) for `count` partition device nodes of `device_path` to
    /// appear, returning their paths.
    ///
    /// # Errors
    /// Returns [`Error::PartitionNodesMissing`](crate::Error::PartitionNodesMissing)
    /// if they do not appear in time.
    fn wait_for_partitions(&self, device_path: &Path, count: usize) -> Result<Vec<PathBuf>>;

    /// Create a filesystem of `fs` on `partition_path` with `label`.
    ///
    /// # Errors
    /// Returns an error if the mkfs tool fails.
    fn make_filesystem(&self, partition_path: &Path, fs: FsKind, label: &str) -> Result<()>;
}

/// Returns the [`PartitionBackend`] for the current compilation target.
///
/// # Errors
///
/// Returns [`Error::Unsupported`](crate::Error::Unsupported) when compiled for
/// an OS that does not yet have a partition backend.
pub fn partition_backend() -> Result<Box<dyn PartitionBackend>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LinuxBackend::new()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(crate::Error::Unsupported(
            "no partition backend for this OS yet".to_owned(),
        ))
    }
}

/// Returns the Windows partition backend (SPEC-00010, Phase 6.2a).
#[cfg(windows)]
pub fn win_partition_backend() -> Result<Box<dyn crate::partition::windows::WinPartitionBackend>> {
    Ok(Box::new(windows::WindowsPartitionBackend::default()))
}

/// A temporary mount, unmounted (and its mountpoint removed) when dropped.
///
/// The `Drop` implementation is the reliability guarantee of SPEC-0004: both
/// mounts are torn down even if the copy fails mid-way.
pub trait Mount {
    /// The directory the filesystem is mounted at.
    fn path(&self) -> &Path;
}

/// Host-specific mounting for the Windows file-copy path (SPEC-0004), behind a
/// trait so the orchestration is testable with a fake.
pub trait MountBackend {
    /// Mount the ISO at `image` read-only (loop; kernel autodetects
    /// iso9660/UDF), returning a RAII guard.
    ///
    /// # Errors
    /// Returns an error if the mount fails.
    fn mount_iso_ro(&self, image: &Path) -> Result<Box<dyn Mount>>;

    /// Mount the NTFS `partition` read-write (ntfs3, then ntfs-3g), returning a
    /// RAII guard.
    ///
    /// # Errors
    /// Returns [`Error::MissingTool`](crate::Error::MissingTool) if no NTFS
    /// driver can mount it, or another error on failure.
    fn mount_ntfs_rw(&self, partition: &Path) -> Result<Box<dyn Mount>>;

    /// Flush the filesystem mounted at `path` durably to disk.
    ///
    /// # Errors
    /// Returns an error if the sync fails.
    fn sync_path(&self, path: &Path) -> Result<()>;
}

/// Returns the [`MountBackend`] for the current compilation target.
///
/// # Errors
///
/// Returns [`Error::Unsupported`](crate::Error::Unsupported) when compiled for
/// an OS that does not yet have a mount backend.
pub fn mount_backend() -> Result<Box<dyn MountBackend>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LinuxBackend::new()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(crate::Error::Unsupported(
            "no mount backend for this OS yet".to_owned(),
        ))
    }
}
