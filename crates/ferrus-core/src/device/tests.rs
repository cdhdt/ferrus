//! Unit tests for the device decision rules and formatting.
//!
//! These cover the safety-critical refusal cases, not just the happy path, as
//! mandated by the working method. They are pure — no device is ever touched.
//!
//! Eligibility is decided on the transport bus, never on the `removable` bit,
//! so every fixture sets `removable: false` on purpose: a USB stick reporting
//! `removable = 0` must still be accepted.

use std::path::PathBuf;

use super::target::{LARGE_TARGET_THRESHOLD_BYTES, ensure_static_guards, is_default_listed};
use super::types::{Bus, Device, format_size};
use crate::Error;

/// Build a device on the given transport. `removable` is deliberately `false`
/// (display-only, never a gate) and the size is small (below the listing
/// threshold) unless overridden via [`sized`].
fn device(path: &str, bus: Bus, system: bool) -> Device {
    Device {
        path: PathBuf::from(path),
        stable_id: None,
        model: Some("Test Model".to_owned()),
        bus,
        size_bytes: 16 * 1000 * 1000 * 1000,
        removable: false,
        is_system_or_critical: system,
    }
}

/// Like [`device`] but with an explicit size.
fn sized(path: &str, bus: Bus, system: bool, size_bytes: u64) -> Device {
    Device {
        size_bytes,
        ..device(path, bus, system)
    }
}

#[test]
fn usb_transport_with_removable_false_is_accepted() {
    // The bug this fixes: a USB stick reporting removable=0 must NOT be refused.
    let dev = device("/dev/sdz", Bus::Usb, false);
    let path = dev.path.clone();
    assert!(ensure_static_guards(&dev, &path).is_ok());
    assert!(dev.is_plausible_target());
}

#[test]
fn internal_fixed_disk_is_refused_on_transport() {
    // Fixed internal transports are refused regardless of any other signal.
    for bus in [Bus::Nvme, Bus::Sata] {
        let dev = device("/dev/nvme0n1", bus, false);
        let path = dev.path.clone();
        let err = ensure_static_guards(&dev, &path).unwrap_err();
        assert!(
            matches!(err, Error::UnsafeTarget(_)),
            "bus {bus} should be refused"
        );
    }
}

#[test]
fn system_device_is_refused() {
    // A USB-transport device that backs the system is still refused (the system
    // guard fires after the transport guard passes).
    let dev = device("/dev/sdz", Bus::Usb, true);
    let path = dev.path.clone();
    let err = ensure_static_guards(&dev, &path).unwrap_err();
    assert!(matches!(err, Error::UnsafeTarget(_)));
}

#[test]
fn confirmed_path_mismatch_is_refused() {
    let dev = device("/dev/sdz", Bus::Usb, false);
    let wrong = PathBuf::from("/dev/sdy");
    let err = ensure_static_guards(&dev, &wrong).unwrap_err();
    assert!(matches!(err, Error::UnsafeTarget(_)));
}

#[test]
fn is_plausible_target_matches_transport_and_system() {
    assert!(device("/dev/sdz", Bus::Usb, false).is_plausible_target());
    assert!(device("/dev/mmcblk0", Bus::Mmc, false).is_plausible_target());
    assert!(!device("/dev/nvme0n1", Bus::Nvme, false).is_plausible_target());
    assert!(!device("/dev/sda", Bus::Sata, false).is_plausible_target());
    assert!(!device("/dev/sdz", Bus::Usb, true).is_plausible_target());
}

#[test]
fn large_usb_is_hidden_by_default_but_acquirable() {
    let big = sized(
        "/dev/sdz",
        Bus::Usb,
        false,
        LARGE_TARGET_THRESHOLD_BYTES + 1,
    );

    // Hidden from the default listing, revealed with include_large.
    assert!(!is_default_listed(&big, false));
    assert!(is_default_listed(&big, true));

    // But size never blocks acquisition: still passes the static guards when
    // explicitly targeted and confirmed.
    let path = big.path.clone();
    assert!(ensure_static_guards(&big, &path).is_ok());
}

#[test]
fn small_usb_is_listed_by_default() {
    let small = sized("/dev/sdz", Bus::Usb, false, LARGE_TARGET_THRESHOLD_BYTES);
    assert!(is_default_listed(&small, false));
}

#[test]
fn format_size_is_human_readable() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(16_000_000_000), "16.0 GB");
    assert_eq!(format_size(2_000_398_934_016), "2.0 TB");
}

// --- Windows STORAGE_BUS_TYPE mapping (Phase 6.1, SPEC-0009) ---------------

#[test]
fn windows_bus_type_maps_to_transport() {
    // The removable transports — the only eligible ones.
    assert_eq!(Bus::from_windows_bus_type(7), Bus::Usb); // BusTypeUsb
    assert_eq!(Bus::from_windows_bus_type(12), Bus::Mmc); // BusTypeSd
    assert_eq!(Bus::from_windows_bus_type(13), Bus::Mmc); // BusTypeMmc
    // Fixed/internal buses.
    assert_eq!(Bus::from_windows_bus_type(11), Bus::Sata); // BusTypeSata
    assert_eq!(Bus::from_windows_bus_type(3), Bus::Sata); // BusTypeAta
    assert_eq!(Bus::from_windows_bus_type(17), Bus::Nvme); // BusTypeNvme
    assert_eq!(Bus::from_windows_bus_type(1), Bus::Scsi); // BusTypeScsi
    assert_eq!(Bus::from_windows_bus_type(10), Bus::Scsi); // BusTypeSas
}

#[test]
fn windows_virtual_and_unknown_buses_are_not_removable() {
    // Virtual disks (VHD / Storage Spaces) and anything unmapped must NOT be an
    // eligible target — they map to Unknown → not a removable transport.
    for bus_type in [0u32, 14, 15, 16, 18, 19, 20, 99] {
        let bus = Bus::from_windows_bus_type(bus_type);
        assert_eq!(bus, Bus::Unknown, "bus_type {bus_type}");
        assert!(!bus.is_removable_transport(), "bus_type {bus_type}");
    }
    // And the removable ones are removable transports; the fixed ones are not.
    assert!(Bus::from_windows_bus_type(7).is_removable_transport());
    assert!(Bus::from_windows_bus_type(13).is_removable_transport());
    assert!(!Bus::from_windows_bus_type(11).is_removable_transport()); // SATA
    assert!(!Bus::from_windows_bus_type(17).is_removable_transport()); // NVMe
}
