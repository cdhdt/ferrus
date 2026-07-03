//! Error types for the Ferrus core engine.
//!
//! The library uses [`thiserror`] to build a structured [`Error`] enum. Binaries
//! (`ferrus-cli`, `ferrus-gui`) are expected to wrap these with `anyhow` for
//! human-facing reporting.

use std::path::PathBuf;

/// Convenience alias for results produced by the core engine.
pub type Result<T> = std::result::Result<T, Error>;

/// All error conditions the core engine can surface.
///
/// Variants are intentionally coarse for Phase 0 and will be refined as each
/// subsystem is implemented. New variants should stay actionable: prefer
/// carrying enough context (paths, device identifiers) for the caller to build
/// a clear message.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// An underlying I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The requested target device could not be found.
    #[error("device not found: {0}")]
    DeviceNotFound(String),

    /// A safety guard refused the operation (e.g. the target is the system
    /// disk, is not removable, or hosts a critical mount).
    ///
    /// This variant exists so that refusals are a distinct, testable outcome
    /// rather than a generic failure. See [`crate::device`].
    #[error("refused for safety: {0}")]
    UnsafeTarget(String),

    /// The ISO/source could not be understood (unreadable, unrecognized
    /// layout, missing expected files).
    #[error("invalid source: {0}")]
    InvalidSource(String),

    /// An expected external tool (e.g. `mkfs.ntfs`) was missing or failed.
    #[error("external tool `{tool}` failed: {reason}")]
    Tool {
        /// Name of the external tool that was invoked.
        tool: String,
        /// Human-readable reason for the failure.
        reason: String,
    },

    /// A required file or asset was not present on disk.
    #[error("missing file: {0}")]
    MissingFile(PathBuf),

    /// A privileged operation was attempted without the required privileges
    /// (writing to a block device needs root — see ADR-0003).
    #[error("insufficient privileges: {0}")]
    PrivilegeRequired(String),

    /// The source image does not fit on the target device.
    #[error("image is larger than the target device ({image_bytes} B > {device_bytes} B)")]
    ImageTooLarge {
        /// Size of the source image in bytes.
        image_bytes: u64,
        /// Capacity of the target device in bytes.
        device_bytes: u64,
    },

    /// The target device is in use and could not be opened exclusively.
    #[error("device is busy (still mounted or held by another process): {0}")]
    DeviceBusy(PathBuf),

    /// Post-write verification found the device contents differ from the image.
    #[error("verification failed: device differs from image at byte {offset}")]
    VerifyMismatch {
        /// Byte offset of the first difference.
        offset: u64,
    },

    /// A required external tool (e.g. `sfdisk`, `mkfs.ntfs`) is not installed.
    #[error("required tool not found: {name}")]
    MissingTool {
        /// Name of the missing executable.
        name: String,
    },

    /// The target device is too small for the requested layout.
    #[error("device is too small ({device_bytes} B < {minimum_bytes} B minimum)")]
    DeviceTooSmall {
        /// Capacity of the target device in bytes.
        device_bytes: u64,
        /// Minimum capacity required, in bytes.
        minimum_bytes: u64,
    },

    /// After writing the partition table, the expected partition device nodes
    /// did not appear in time.
    #[error("partition nodes for {device} did not appear (expected {expected})")]
    PartitionNodesMissing {
        /// The whole-device path whose partitions were awaited.
        device: PathBuf,
        /// Number of partition nodes expected.
        expected: usize,
    },

    /// The operation, or the current platform target, is not implemented yet.
    #[error("not implemented: {0}")]
    Unsupported(String),
}
