//! Block-device enumeration and **safe target selection**.
//!
//! This is the most safety-critical module in Ferrus: it decides which devices
//! may be written to. The guard rails live here, in code, and every destructive
//! operation must pass through [`SafeTarget::acquire`] — the single, tested
//! checkpoint described in `CLAUDE.md`.
//!
//! Design rules encoded by this API:
//!
//! - Enumeration is **defensive**: [`list_writable_candidates`] only returns
//!   plausible targets and always reports size / model / path so a human can
//!   disambiguate.
//! - A raw [`Device`] is **not** writable. Callers must convert one into a
//!   [`SafeTarget`], which can only be produced after the checkpoint has
//!   verified the device is removable and hosts neither the system nor a
//!   critical mount, and after the caller has explicitly confirmed it.

use std::path::PathBuf;

use crate::Result;

/// A block device discovered on the host.
///
/// This describes a device; it does **not** grant permission to write to it.
/// Obtain a [`SafeTarget`] via [`SafeTarget::acquire`] for that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// OS-level path/identifier of the whole device (e.g. `/dev/sdb` on Linux).
    pub path: PathBuf,
    /// Human-readable model string, when the OS exposes one.
    pub model: Option<String>,
    /// Total capacity in bytes.
    pub size_bytes: u64,
    /// Whether the OS reports this device as removable.
    pub removable: bool,
    /// Whether this device currently hosts the running system or a mount the
    /// engine considers critical (swap, `/`, `/boot`, …). A `true` value is an
    /// automatic refusal.
    pub is_system_or_critical: bool,
}

impl Device {
    /// Returns `true` only if this device is, on the face of it, a plausible
    /// write target (removable and not system/critical).
    ///
    /// This is a **necessary but not sufficient** condition: it is a display
    /// filter for enumeration, not the authorization to write. Authorization
    /// only comes from [`SafeTarget::acquire`].
    #[must_use]
    pub fn is_plausible_target(&self) -> bool {
        self.removable && !self.is_system_or_critical
    }
}

/// Enumerate block devices that are plausible write targets.
///
/// Non-removable devices and the system/critical devices are filtered out here
/// so that the UI never even presents them. The returned list is meant for a
/// human to choose from, with enough detail (size, model, path) to remove
/// ambiguity.
///
/// # Errors
///
/// Returns an error if the host's device inventory cannot be read.
pub fn list_writable_candidates() -> Result<Vec<Device>> {
    // TODO(phase1): implement via the platform backend
    // (`crate::platform::Backend::enumerate_devices`) and keep only
    // `Device::is_plausible_target()` entries.
    todo!("device enumeration lands in Phase 1")
}

/// An authorized, confirmed write target.
///
/// The only way to obtain one is [`SafeTarget::acquire`], which is the single
/// checkpoint every destructive operation must pass. Holding a `SafeTarget`
/// therefore encodes, at the type level, that the safety checks and the user's
/// explicit confirmation both succeeded.
#[derive(Debug, Clone)]
pub struct SafeTarget {
    device: Device,
    dry_run: bool,
}

impl SafeTarget {
    /// The **single safety checkpoint**. Verifies the device is a legitimate
    /// target and that the caller has explicitly confirmed it, then returns a
    /// writable handle.
    ///
    /// `confirmed_path` must exactly match `device.path`; this makes an
    /// accidental or guessed target impossible to authorize — the caller has to
    /// echo back the precise path it intends to erase.
    ///
    /// When `dry_run` is `true`, downstream operations must simulate rather than
    /// perform destructive work.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsafeTarget`](crate::Error::UnsafeTarget) if the device
    /// is not removable, is the system/critical device, or if `confirmed_path`
    /// does not match the device path.
    pub fn acquire(device: Device, confirmed_path: &std::path::Path, dry_run: bool) -> Result<Self> {
        // TODO(phase1): enforce every guard here and nowhere else:
        //   1. device.removable must be true;
        //   2. device.is_system_or_critical must be false;
        //   3. confirmed_path must equal device.path (no guessed /dev/sdX);
        //   4. re-check mount state at acquire time (TOCTOU) via the platform
        //      backend before returning.
        let _ = (device, confirmed_path, dry_run);
        todo!("safe-target checkpoint lands in Phase 1")
    }

    /// The authorized device.
    #[must_use]
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Whether operations on this target must be simulated (no writes).
    #[must_use]
    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }
}
