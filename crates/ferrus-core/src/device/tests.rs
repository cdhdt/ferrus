//! Unit tests for the device decision rules and formatting.
//!
//! These cover the safety-critical refusal cases, not just the happy path, as
//! mandated by the working method. They are pure — no device is ever touched.

use std::path::PathBuf;

use super::target::ensure_static_guards;
use super::types::{Bus, Device, format_size};
use crate::Error;

/// Build a device with sensible, safe-by-default fields for tests.
fn device(path: &str, removable: bool, system: bool) -> Device {
    Device {
        path: PathBuf::from(path),
        stable_id: None,
        model: Some("Test Model".to_owned()),
        bus: Bus::Usb,
        size_bytes: 16 * 1000 * 1000 * 1000,
        removable,
        is_system_or_critical: system,
    }
}

#[test]
fn valid_removable_passes_static_guards() {
    let dev = device("/dev/sdz", true, false);
    let path = dev.path.clone();
    assert!(ensure_static_guards(&dev, &path).is_ok());
}

#[test]
fn non_removable_is_refused() {
    let dev = device("/dev/sda", false, false);
    let path = dev.path.clone();
    let err = ensure_static_guards(&dev, &path).unwrap_err();
    assert!(matches!(err, Error::UnsafeTarget(_)));
}

#[test]
fn system_device_is_refused() {
    // Even a removable-looking device that backs the system is refused.
    let dev = device("/dev/sdz", true, true);
    let path = dev.path.clone();
    let err = ensure_static_guards(&dev, &path).unwrap_err();
    assert!(matches!(err, Error::UnsafeTarget(_)));
}

#[test]
fn confirmed_path_mismatch_is_refused() {
    let dev = device("/dev/sdz", true, false);
    let wrong = PathBuf::from("/dev/sdy");
    let err = ensure_static_guards(&dev, &wrong).unwrap_err();
    assert!(matches!(err, Error::UnsafeTarget(_)));
}

#[test]
fn is_plausible_target_matches_guards() {
    assert!(device("/dev/sdz", true, false).is_plausible_target());
    assert!(!device("/dev/sda", false, false).is_plausible_target());
    assert!(!device("/dev/sdz", true, true).is_plausible_target());
}

#[test]
fn format_size_is_human_readable() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(16_000_000_000), "16.0 GB");
    assert_eq!(format_size(2_000_398_934_016), "2.0 TB");
}
