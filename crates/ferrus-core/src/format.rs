//! Filesystem creation.
//!
//! Thin wrapper: the actual `mkfs.ntfs` / `mkfs.vfat` invocation lives behind
//! the platform [`PartitionBackend`](crate::platform::PartitionBackend) so it is
//! testable and swappable per OS. The engine talks in terms of [`FsKind`].

use std::path::Path;

use crate::Result;
use crate::partition::FsKind;

/// Create a filesystem of `fs` on the partition at `partition_path`, applying
/// `label`.
///
/// # Errors
///
/// Returns [`Error::MissingTool`](crate::Error::MissingTool) or
/// [`Error::Tool`](crate::Error::Tool) if the underlying `mkfs` tool is missing
/// or fails.
pub fn make_filesystem(partition_path: &Path, fs: FsKind, label: &str) -> Result<()> {
    crate::platform::partition_backend()?.make_filesystem(partition_path, fs, label)
}
