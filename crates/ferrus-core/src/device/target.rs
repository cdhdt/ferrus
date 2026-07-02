//! The [`SafeTarget`] checkpoint and the enumeration entry points.
//!
//! `SafeTarget` is the single gate every destructive operation must pass. The
//! static guards are factored into [`ensure_static_guards`] so they can be unit
//! tested without any I/O; `acquire` composes them with a live TOCTOU re-check.

use std::path::Path;

use super::types::Device;
use crate::{Error, Result};

/// Enumerate every block device the host exposes, without filtering.
///
/// Prefer [`list_writable_candidates`] for anything user-facing; this fuller
/// list exists so the selection flow can route even a rejected target through
/// [`SafeTarget::acquire`] and get a precise refusal reason.
///
/// # Errors
///
/// Returns an error if the host's device inventory cannot be read.
pub fn list_all_devices() -> Result<Vec<Device>> {
    let backend = crate::platform::backend()?;
    backend.enumerate_devices()
}

/// Enumerate block devices that are plausible write targets.
///
/// Non-removable devices and the system/critical devices are filtered out here
/// so that the UI never even presents them. The returned list is meant for a
/// human to choose from, with enough detail (size, model, bus, path) to remove
/// ambiguity.
///
/// # Errors
///
/// Returns an error if the host's device inventory cannot be read.
pub fn list_writable_candidates() -> Result<Vec<Device>> {
    let mut devices = list_all_devices()?;
    devices.retain(Device::is_plausible_target);
    Ok(devices)
}

/// The static safety guards of the checkpoint, factored out so they can be unit
/// tested without any I/O. Returns `Ok(())` only if the device is removable,
/// not system/critical, and the confirmed path matches exactly.
///
/// This is the pure decision core; [`SafeTarget::acquire`] additionally performs
/// a live TOCTOU re-check via the platform backend.
pub(super) fn ensure_static_guards(device: &Device, confirmed_path: &Path) -> Result<()> {
    if !device.removable {
        return Err(Error::UnsafeTarget(format!(
            "{} is not a removable device",
            device.path.display()
        )));
    }
    if device.is_system_or_critical {
        return Err(Error::UnsafeTarget(format!(
            "{} backs the running system or a critical mount",
            device.path.display()
        )));
    }
    if confirmed_path != device.path {
        return Err(Error::UnsafeTarget(format!(
            "confirmed path {} does not match the selected device {}",
            confirmed_path.display(),
            device.path.display()
        )));
    }
    Ok(())
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
    /// is not removable, is the system/critical device, if `confirmed_path`
    /// does not match the device path, or if a live re-check finds the device
    /// became system/critical since enumeration.
    pub fn acquire(device: Device, confirmed_path: &Path, dry_run: bool) -> Result<Self> {
        // 1-3: static guards (pure, ordered, first failure wins).
        ensure_static_guards(&device, confirmed_path)?;

        // 4: live TOCTOU re-check — the device may have been mounted or become
        // critical between enumeration and now.
        let backend = crate::platform::backend()?;
        if backend.is_system_or_critical(&device.path)? {
            return Err(Error::UnsafeTarget(format!(
                "{} became system or critical since it was listed",
                device.path.display()
            )));
        }

        Ok(Self { device, dry_run })
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
