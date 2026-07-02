//! Copying source content onto the target media.
//!
//! Two broad strategies exist and are selected from [`crate::source`]:
//!
//! - **Raw copy** — for generic hybrid ISOs that are already bootable images,
//!   stream the bytes to the block device.
//! - **File-level copy** — mount/extract the ISO and copy its tree onto a
//!   freshly created filesystem (required for the Windows NTFS strategy).

use std::path::Path;

use crate::Result;
use crate::device::SafeTarget;
use crate::progress::ProgressSink;

/// Stream a source image byte-for-byte onto the target device.
///
/// Honors [`SafeTarget::is_dry_run`]. Reports progress through `progress`.
///
/// # Errors
///
/// Returns an error on read/write failure.
pub fn raw_copy(source: &Path, target: &SafeTarget, progress: &mut dyn ProgressSink) -> Result<()> {
    // TODO(phase2): stream with periodic fsync and progress reporting; no-op
    // (log only) when `target.is_dry_run()`.
    let _ = (source, target, progress);
    todo!("raw copy lands in Phase 2")
}

/// Copy the extracted contents of a source image onto a mounted filesystem.
///
/// Reports progress through `progress`.
///
/// # Errors
///
/// Returns an error on read/write failure.
pub fn file_copy(
    source_root: &Path,
    dest_mount: &Path,
    progress: &mut dyn ProgressSink,
) -> Result<()> {
    // TODO(phase3): recursive copy of the ISO tree; handle large `install.wim`.
    let _ = (source_root, dest_mount, progress);
    todo!("file-level copy lands in Phase 3")
}
