//! Filesystem creation wrappers.
//!
//! Thin wrappers around the host's `mkfs` tooling (`mkfs.ntfs`, `mkfs.vfat` on
//! Linux). Keeping these behind a small API means the rest of the engine talks
//! in terms of [`FsKind`] rather than shelling out directly, and the platform
//! backend can substitute native calls on other OSes later.

use crate::Result;
use crate::partition::FsKind;

/// Create a filesystem of `fs` on the partition at `partition_path`.
///
/// `label` is applied as the volume label when the target filesystem supports
/// one.
///
/// # Errors
///
/// Returns [`Error::Tool`](crate::Error::Tool) if the underlying `mkfs` tool is
/// missing or exits non-zero.
pub fn make_filesystem(
    partition_path: &std::path::Path,
    fs: FsKind,
    label: Option<&str>,
) -> Result<()> {
    // TODO(phase2/3): dispatch through the platform backend to the right mkfs
    // implementation. On Linux this wraps `mkfs.ntfs --quick` / `mkfs.vfat`.
    let _ = (partition_path, fs, label);
    todo!("filesystem creation lands in Phase 2/3")
}
