//! Windows ISO → NTFS partition copy orchestration (Phase 3b, SPEC-0004).
//!
//! Runs after Phase 3a, inside `prepare-windows`. Mounts the ISO read-only,
//! validates it is a Windows install ISO, space-guards, mounts the NTFS
//! partition read-write, streams the whole tree across, syncs, and (via RAII
//! guards) unmounts — even on a mid-copy failure.

use std::path::Path;

use super::tree::{self, RealTreeIo, TreeIo};
use crate::device::format_size;
use crate::platform::{MountBackend, mount_backend};
use crate::progress::{ProgressSink, Stage};
use crate::source::{RawImage, detect_windows_install};
use crate::{Error, Result};

/// Copy the contents of the Windows ISO `image` onto the NTFS `partition`.
///
/// `capacity` is the partition's size in bytes (for the space guard). Honors
/// `dry_run` (mounts/copies nothing). With `verify`, checks the destination
/// install-image size matches the source after the copy.
///
/// # Errors
///
/// Returns [`Error::NotWindowsMedia`], [`Error::InsufficientSpace`],
/// [`Error::MissingTool`], [`Error::VerificationFailed`], [`Error::Tool`], or
/// [`Error::Io`].
pub fn copy_windows(
    image: &RawImage,
    partition: &Path,
    capacity: u64,
    dry_run: bool,
    verify: bool,
    progress: &mut dyn ProgressSink,
) -> Result<()> {
    let backend = mount_backend()?;
    let io = RealTreeIo;
    copy_windows_with(
        image,
        partition,
        capacity,
        dry_run,
        verify,
        progress,
        backend.as_ref(),
        &io,
    )
}

/// Space guard: refuse when the content would not fit on the partition.
fn ensure_space(needed: u64, available: u64) -> Result<()> {
    if needed > available {
        return Err(Error::InsufficientSpace { needed, available });
    }
    Ok(())
}

/// The orchestration with backends injected, for testing. See SPEC-0004.
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_windows_with(
    image: &RawImage,
    partition: &Path,
    capacity: u64,
    dry_run: bool,
    verify: bool,
    progress: &mut dyn ProgressSink,
    mounts: &dyn MountBackend,
    io: &dyn TreeIo,
) -> Result<()> {
    progress.stage(Stage::Copying);

    if dry_run {
        progress.message(&format!(
            "dry-run: would validate {} and copy its contents (image is {}) to {} \
             — nothing mounted",
            image.path().display(),
            format_size(image.size_bytes()),
            partition.display(),
        ));
        return Ok(());
    }

    // Mount ISO read-only. The guard unmounts on any early return below.
    progress.message("mounting ISO (read-only)");
    let iso = mounts.mount_iso_ro(image.path())?;

    progress.message("inspecting ISO");
    let scan = tree::scan(io, iso.path())?;
    let install = detect_windows_install(&scan.files).ok_or_else(|| Error::NotWindowsMedia {
        path: image.path().to_path_buf(),
    })?;

    ensure_space(scan.total_bytes, capacity)?;

    // Mount the NTFS partition read-write (second guard).
    progress.message("mounting NTFS partition (read-write)");
    let ntfs = mounts.mount_ntfs_rw(partition)?;

    progress.message(&format!(
        "copying {} of Windows files",
        format_size(scan.total_bytes)
    ));
    let copied = tree::copy_tree(io, iso.path(), ntfs.path(), scan.total_bytes, progress)?;

    progress.stage(Stage::Finalizing);
    progress.message("flushing NTFS (sync)");
    mounts.sync_path(ntfs.path())?;

    if verify {
        progress.message("verifying install image size");
        // Use the original-case relative path (mounts are case-sensitive).
        let rel = scan
            .original_of
            .get(&install.install_image)
            .cloned()
            .unwrap_or_else(|| std::path::PathBuf::from(&install.install_image));
        let dest = ntfs.path().join(rel);
        let dest_size = io.file_size(&dest)?;
        if dest_size != install.install_image_bytes {
            return Err(Error::VerificationFailed(format!(
                "{} is {} B on the stick but {} B in the ISO",
                install.install_image, dest_size, install.install_image_bytes
            )));
        }
    }

    progress.message(&format!(
        "done: {} of Windows files copied. Still NOT bootable — the UEFI:NTFS \
         bootloader (Phase 3c) is not installed yet.",
        format_size(copied)
    ));
    Ok(())
    // `iso` and `ntfs` drop here (or on any early error) → unmounted.
}
