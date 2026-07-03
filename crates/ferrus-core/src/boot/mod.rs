//! Bootloader installation — UEFI:NTFS (Phase 3c, SPEC-0005).
//!
//! Writes the vendored UEFI:NTFS image raw onto the small FAT helper partition
//! (P2), the last step that makes a Windows install stick bootable under UEFI.
//! The image is embedded with `include_bytes!` and its integrity is checked
//! against a pinned SHA-256 **before** any write — a bootloader is a
//! chain-of-trust component. The write reuses the Phase 2 path (exclusive
//! `O_EXCL` open + block loop + `fsync`) targeted at the partition node.
//!
//! Legacy BIOS boot (writing an MBR boot sector) is planned for a later phase.

use std::io::Cursor;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::copy::copy_stream;
use crate::device::format_size;
use crate::platform::WriteBackend;
use crate::progress::{ProgressSink, Stage};
use crate::{Error, Result};

/// The vendored UEFI:NTFS image — a 1 MiB FAT12 filesystem carrying the Secure
/// Boot-signed loader and the NTFS driver. See `res/uefi/NOTICE`.
const UEFI_NTFS_IMG: &[u8] = include_bytes!("../../../../res/uefi/uefi-ntfs.img");

/// Pinned SHA-256 of [`UEFI_NTFS_IMG`]. Must match the asset (see NOTICE); a
/// test enforces the equality so code and asset never drift.
pub const UEFI_NTFS_IMG_SHA256: &str =
    "72683fa1250eeea772d3399277b434d4e55ba8dd0dc926e52d817e701fc2eb9e";

/// Lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Verify `bytes` hashes to `expected` (hex SHA-256).
fn verify_integrity(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(Error::BootloaderIntegrity {
            expected: expected.to_owned(),
            actual,
        });
    }
    Ok(())
}

/// Ensure the bootloader image fits the helper partition.
fn ensure_fits(image_bytes: u64, partition_bytes: u64) -> Result<()> {
    if image_bytes > partition_bytes {
        return Err(Error::ImageExceedsPartition {
            image_bytes,
            partition_bytes,
        });
    }
    Ok(())
}

/// Install the UEFI:NTFS bootloader onto the helper partition `partition`
/// (`partition_bytes` is its capacity, for the fit check).
///
/// Verifies the embedded image's integrity, then writes it raw with the Phase 2
/// exclusive-open + block-copy + `fsync` path. Honors `dry_run`.
///
/// # Errors
///
/// Returns [`Error::BootloaderIntegrity`], [`Error::ImageExceedsPartition`],
/// [`Error::DeviceBusy`], or [`Error::Io`].
pub fn install_uefi_ntfs(
    partition: &Path,
    partition_bytes: u64,
    dry_run: bool,
    progress: &mut dyn ProgressSink,
) -> Result<()> {
    let backend = crate::platform::write_backend()?;
    install_uefi_ntfs_with(
        UEFI_NTFS_IMG,
        UEFI_NTFS_IMG_SHA256,
        partition,
        partition_bytes,
        dry_run,
        progress,
        backend.as_ref(),
    )
}

/// The orchestration, with the image, its expected hash, and the write backend
/// all injected so it can be driven by a fake with a small image in tests.
#[allow(clippy::too_many_arguments)]
fn install_uefi_ntfs_with(
    image: &[u8],
    expected_sha: &str,
    partition: &Path,
    partition_bytes: u64,
    dry_run: bool,
    progress: &mut dyn ProgressSink,
    backend: &dyn WriteBackend,
) -> Result<()> {
    progress.stage(Stage::InstallingBoot);

    // Integrity gate first — never write an unverified chain-of-trust blob.
    verify_integrity(image, expected_sha)?;
    ensure_fits(image.len() as u64, partition_bytes)?;

    if dry_run {
        progress.message(&format!(
            "dry-run: would write the UEFI:NTFS bootloader ({}) to {} — nothing written",
            format_size(image.len() as u64),
            partition.display(),
        ));
        return Ok(());
    }

    progress.message(&format!(
        "writing UEFI:NTFS bootloader to {}",
        partition.display()
    ));
    let mut sink = backend.open_exclusive_writer(partition)?;
    let mut reader = Cursor::new(image);
    copy_stream(&mut reader, sink.as_mut(), image.len() as u64, progress)?;

    progress.stage(Stage::Finalizing);
    progress.message("flushing bootloader (fsync)");
    sink.sync()?;

    progress.message("done: UEFI:NTFS installed — the stick is now bootable.");
    Ok(())
}

/// Install a Legacy BIOS boot sector on the target.
///
/// # Errors
///
/// Returns an error on write failure.
pub fn install_legacy_boot(target: &crate::device::SafeTarget) -> Result<()> {
    // TODO(phase3+): legacy/BIOS boot support is deferred; UEFI is the priority.
    let _ = target;
    todo!("legacy boot support is deferred")
}

#[cfg(test)]
mod tests;
