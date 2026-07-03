//! Block-device enumeration and **safe target selection**.
//!
//! This is the most safety-critical module in Ferrus: it decides which devices
//! may be written to. The guard rails live here, in code, and every destructive
//! operation must pass through [`SafeTarget::acquire`] — the single, tested
//! checkpoint described in `CLAUDE.md`.
//!
//! The behavior contract is specified in `docs/specs/device.md` (SPEC-0001);
//! read it before changing anything here.
//!
//! Layout:
//!
//! - [`types`] — the [`Device`] data type, its [`Bus`], and size formatting.
//! - [`target`] — the [`SafeTarget`] checkpoint and the enumeration entry
//!   points.
//!
//! Design rules encoded by this API:
//!
//! - Enumeration is **defensive**: [`list_writable_candidates`] only returns
//!   plausible targets and always reports size / model / bus / path so a human
//!   can disambiguate.
//! - A raw [`Device`] is **not** writable. Callers must convert one into a
//!   [`SafeTarget`], which can only be produced after the checkpoint has
//!   verified the device is removable and hosts neither the system nor a
//!   critical mount, and after the caller has explicitly confirmed it.

mod target;
mod types;

pub use target::{
    LARGE_TARGET_THRESHOLD_BYTES, SafeTarget, list_all_devices, list_writable_candidates,
};
pub use types::{Bus, Device, format_size};

#[cfg(test)]
mod tests;
