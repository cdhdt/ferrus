//! Copying source content onto the target media.
//!
//! Phase 2 implements the **raw copy** path (SPEC-0002): a byte-for-byte write
//! of an already-bootable image onto the whole device, `dd`-style. The
//! file-level copy path (mount/extract onto a fresh filesystem, for the Windows
//! NTFS strategy) remains a Phase 3 stub.
//!
//! Layout:
//!
//! - [`stream`] — the pure block-copy loops.
//! - [`raw_copy`] — the Phase 2 raw (dd-style) whole-device write.
//! - [`tree`] — recursive scan/copy over a testable `TreeIo` seam.
//! - [`copy_windows`] — the Phase 3b Windows ISO → NTFS file copy.

mod stream;
mod tree;
mod windows;

pub use windows::copy_windows;

/// The Phase 2 block-copy loop, reused by the bootloader writer (Phase 3c).
pub(crate) use stream::copy_stream;

use std::io::Read;
use std::path::Path;

use crate::device::{SafeTarget, format_size};
use crate::platform::WriteBackend;
use crate::progress::{ProgressSink, Stage};
use crate::source::RawImage;
use crate::{Error, Result};

use stream::BLOCK_SIZE;

/// Byte-for-byte copy of `image` onto the target device behind `target`.
///
/// The entry point takes a [`SafeTarget`], so it cannot be reached without
/// having passed the SPEC-0001 checkpoint. Honors
/// [`SafeTarget::is_dry_run`]: in dry-run nothing is opened, unmounted, or
/// written. When `verify` is set, the device is read back and compared to the
/// image after the write.
///
/// # Errors
///
/// Returns [`Error::ImageTooLarge`], [`Error::PrivilegeRequired`],
/// [`Error::UnsafeTarget`], [`Error::DeviceBusy`], [`Error::VerifyMismatch`], or
/// [`Error::Io`] as appropriate.
pub fn raw_copy(
    image: &RawImage,
    target: &SafeTarget,
    verify: bool,
    progress: &mut dyn ProgressSink,
) -> Result<()> {
    let backend = crate::platform::write_backend()?;
    raw_copy_with(image, target, verify, progress, backend.as_ref())
}

/// Size guard: refuse when the image would not fit on the device.
fn ensure_fits(image_bytes: u64, device_bytes: u64) -> Result<()> {
    if image_bytes > device_bytes {
        return Err(Error::ImageTooLarge {
            image_bytes,
            device_bytes,
        });
    }
    Ok(())
}

/// The orchestration, with the OS backend injected so it can be driven by a fake
/// in tests. See SPEC-0002 for the exact ordering and rationale.
fn raw_copy_with(
    image: &RawImage,
    target: &SafeTarget,
    verify: bool,
    progress: &mut dyn ProgressSink,
    backend: &dyn WriteBackend,
) -> Result<()> {
    let device = target.device();
    ensure_fits(image.size_bytes(), device.size_bytes)?;

    if target.is_dry_run() {
        progress.stage(Stage::Copying);
        progress.message(&format!(
            "dry-run: would write {} to {} in {} blocks, then fsync — nothing touched",
            format_size(image.size_bytes()),
            device.path.display(),
            format_size(BLOCK_SIZE as u64),
        ));
        return Ok(());
    }

    // Writing to a block device requires root (enumeration did not — ADR-0003).
    if backend.effective_uid()? != 0 {
        return Err(Error::PrivilegeRequired(
            "writing to a block device requires root".to_owned(),
        ));
    }

    // Defense in depth: never write a disk that backs the system. SafeTarget
    // already guarantees this, but we re-check live right before unmounting.
    if backend.is_system_or_critical(&device.path)? {
        return Err(Error::UnsafeTarget(format!(
            "{} backs the system or a critical mount — aborting",
            device.path.display()
        )));
    }

    // Unmount every mounted partition of the target before writing.
    for mountpoint in backend.mounted_partitions(&device.path)? {
        progress.message(&format!("unmounting {}", mountpoint.display()));
        backend.unmount(&mountpoint)?;
    }

    // Exclusive open, then stream the image and fsync before declaring success.
    let mut sink = backend.open_exclusive_writer(&device.path)?;
    let mut reader = image.open_reader()?;
    let written = copy_stream(&mut reader, sink.as_mut(), image.size_bytes(), progress)?;

    progress.stage(Stage::Finalizing);
    progress.message("flushing to device (fsync)");
    sink.sync()?;

    if verify {
        verify_readback(image, &device.path, written, progress, backend)?;
    }

    progress.message(&format!("done: {} written", format_size(written)));
    Ok(())
}

/// Read the device back and compare the first `len` bytes to the image.
fn verify_readback(
    image: &RawImage,
    device_path: &Path,
    len: u64,
    progress: &mut dyn ProgressSink,
    backend: &dyn WriteBackend,
) -> Result<()> {
    progress.stage(Stage::Finalizing);
    progress.message("verifying written data");

    let mut image_reader = image.open_reader()?;
    let mut device_reader = backend.open_reader(device_path)?;
    let mut image_buf = vec![0u8; BLOCK_SIZE];
    let mut device_buf = vec![0u8; BLOCK_SIZE];
    let mut offset: u64 = 0;

    while offset < len {
        let want = usize::try_from((len - offset).min(BLOCK_SIZE as u64)).unwrap_or(BLOCK_SIZE);
        read_exact_into(&mut image_reader, &mut image_buf[..want])?;
        read_exact_into(&mut device_reader, &mut device_buf[..want])?;
        if let Some(pos) = first_difference(&image_buf[..want], &device_buf[..want]) {
            return Err(Error::VerifyMismatch {
                offset: offset + pos as u64,
            });
        }
        offset += want as u64;
        progress.advance(offset, Some(len));
    }
    Ok(())
}

/// Read exactly `buf.len()` bytes, coping with short reads.
fn read_exact_into(reader: &mut dyn Read, buf: &mut [u8]) -> Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    if filled < buf.len() {
        // Device shorter than the image we just wrote: treat as a mismatch at
        // the truncation point.
        return Err(Error::VerifyMismatch {
            offset: filled as u64,
        });
    }
    Ok(())
}

/// Index of the first differing byte between two equal-length slices.
fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

/// Copy the extracted contents of a source image onto a mounted filesystem.
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

#[cfg(test)]
mod tests;
