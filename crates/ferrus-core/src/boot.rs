//! Bootloader installation.
//!
//! For the Windows NTFS strategy, Ferrus writes **UEFI:NTFS** (pbatard) onto a
//! small FAT helper partition. UEFI:NTFS is a GPLv3 EFI bootloader that lets a
//! UEFI firmware chain-load the main NTFS partition; Secure Boot-signed binaries
//! are available (shipped by Rufus). The blob is vendored under `res/uefi/`
//! (see ADR-0002) together with its license/source NOTICE.
//!
//! Legacy BIOS boot (writing an MBR boot sector) is planned for a later phase.

use crate::Result;
use crate::device::SafeTarget;

/// Install the UEFI:NTFS bootloader onto the helper FAT partition of `target`.
///
/// Honors [`SafeTarget::is_dry_run`].
///
/// # Errors
///
/// Returns [`Error::MissingFile`](crate::Error::MissingFile) if the vendored
/// UEFI:NTFS asset cannot be found, or an I/O error on write failure.
pub fn install_uefi_ntfs(target: &SafeTarget) -> Result<()> {
    // TODO(phase3): locate the vendored image under res/uefi/, write it to the
    // FAT helper partition. Verify against a checked-in hash. No-op (log only)
    // when `target.is_dry_run()`.
    //
    // NOTE(verify): confirm the exact asset name/hash and the FAT partition
    // sizing against the current UEFI:NTFS release before implementing — do not
    // hardcode from memory.
    let _ = target;
    todo!("UEFI:NTFS installation lands in Phase 3")
}

/// Install a Legacy BIOS boot sector on the target.
///
/// # Errors
///
/// Returns an error on write failure.
pub fn install_legacy_boot(target: &SafeTarget) -> Result<()> {
    // TODO(phase3+): legacy/BIOS boot support is deferred; UEFI is the priority.
    let _ = target;
    todo!("legacy boot support is deferred")
}
