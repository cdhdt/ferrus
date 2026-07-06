//! State-logic tests for the GUI (no rendering — see SPEC-0007).
//!
//! iced views cannot be asserted reliably, so this covers the pure parts: the
//! `WindowsTweaks` mapping, the run/gating predicates, the Generic gating, the
//! password hygiene, and the key `update` transitions. Rendering is validated
//! manually.

use ferrus_core::device::{Bus, Device};

use super::{Ferrus, IsoInfo, MediaKind, Message, Password};

fn dev(path: &str) -> Device {
    Device {
        path: path.into(),
        stable_id: None,
        model: Some("Test Stick".to_owned()),
        bus: Bus::Usb,
        size_bytes: 32_000_000_000,
        removable: true,
        is_system_or_critical: false,
    }
}

fn iso() -> IsoInfo {
    IsoInfo {
        path: "/images/win.iso".into(),
        size: 5_000_000_000,
    }
}

/// A state that is ready to run: device + ISO selected and the target path typed
/// into the type-to-confirm field.
fn ready() -> Ferrus {
    let mut s = Ferrus::default();
    let _ = s.update(Message::SelectDevice(dev("/dev/sdb")));
    let _ = s.update(Message::IsoValidated(Ok(iso())));
    let _ = s.update(Message::ConfirmInput("/dev/sdb".to_owned()));
    s
}

// --- WindowsTweaks mapping (1:1) ------------------------------------------

#[test]
fn tweaks_map_state_one_to_one() {
    let s = Ferrus {
        bypass_hardware: true,
        account_enabled: true,
        account_name: "ferrus".to_owned(),
        account_password: Password("hunter2".to_owned()),
        minimize_telemetry: true,
        disable_auto_bitlocker: true,
        region_enabled: true,
        region_locale: "fr-FR".to_owned(),
        ..Ferrus::default()
    };

    let t = s.tweaks_wire();
    assert!(t.bypass_hardware);
    assert!(t.minimize_telemetry);
    assert!(t.disable_auto_bitlocker);
    assert_eq!(t.account_name.as_deref(), Some("ferrus"));
    assert_eq!(t.account_password.as_deref(), Some("hunter2"));
    assert_eq!(t.region.as_deref(), Some("fr-FR"));
}

#[test]
fn empty_password_maps_to_none() {
    let s = Ferrus {
        account_enabled: true,
        account_name: "ferrus".to_owned(),
        // password left empty
        ..Ferrus::default()
    };
    let t = s.tweaks_wire();
    assert_eq!(t.account_name.as_deref(), Some("ferrus"));
    assert_eq!(t.account_password, None);
}

#[test]
fn account_disabled_means_no_local_account() {
    let t = Ferrus::default().tweaks_wire();
    assert!(t.account_name.is_none());
    assert!(t.account_password.is_none());
}

// --- password hygiene -----------------------------------------------------

#[test]
fn password_never_appears_in_debug() {
    let secret = "S3cr3t-Passw0rd";
    let s = Ferrus {
        account_enabled: true,
        account_password: Password(secret.to_owned()),
        ..Ferrus::default()
    };

    assert!(!format!("{s:?}").contains(secret));
    assert_eq!(format!("{:?}", Password(secret.to_owned())), "<redacted>");
    assert_eq!(format!("{:?}", Password(String::new())), "<empty>");
}

// --- run / gating predicates ----------------------------------------------

#[test]
fn cannot_run_until_device_iso_and_confirmation() {
    let mut s = Ferrus::default();
    assert!(!s.can_run(), "nothing selected");

    let _ = s.update(Message::SelectDevice(dev("/dev/sdb")));
    assert!(!s.can_run(), "device only");

    let _ = s.update(Message::IsoValidated(Ok(iso())));
    assert!(!s.can_run(), "device + ISO but not yet confirmed");

    let _ = s.update(Message::ConfirmInput("/dev/sdb".to_owned()));
    assert!(s.can_run(), "device + ISO + exact confirmation");
}

// --- type-to-confirm (the guard against wiping the wrong disk) -------------

#[test]
fn confirmation_must_match_the_device_path_exactly() {
    let mut s = ready();
    assert!(s.can_run());

    let _ = s.update(Message::ConfirmInput("/dev/sdc".to_owned()));
    assert!(!s.can_run(), "a non-matching path keeps the action blocked");

    let _ = s.update(Message::ConfirmInput("/dev/sdb".to_owned()));
    assert!(s.can_run(), "the exact path unlocks it");
}

#[test]
fn changing_device_clears_confirmation() {
    let mut s = ready();
    assert!(s.can_run());
    // Selecting a different device invalidates the confirmation.
    let _ = s.update(Message::SelectDevice(dev("/dev/sdc")));
    assert!(
        s.confirm.is_empty(),
        "confirmation cleared on device change"
    );
    assert!(!s.can_run(), "re-blocked until the new device is confirmed");
}

#[test]
fn enabled_account_without_name_blocks_run() {
    let mut s = ready();
    assert!(s.can_run());
    s.account_enabled = true;
    s.account_name = "   ".to_owned();
    assert!(!s.can_run(), "account enabled but name blank");
    s.account_name = "ferrus".to_owned();
    assert!(s.can_run());
}

#[test]
fn enabled_region_without_locale_blocks_run() {
    let mut s = ready();
    s.region_enabled = true;
    s.region_locale = String::new();
    assert!(!s.can_run());
    s.region_locale = "fr-FR".to_owned();
    assert!(s.can_run());
}

// --- Generic gating (mechanism; detection deferred, SPEC-0007) ------------

#[test]
fn generic_media_hides_tweaks() {
    let mut s = ready();
    assert_eq!(s.media, MediaKind::Unknown);
    assert!(s.show_tweaks(), "unknown media still shows tweaks");

    s.media = MediaKind::Generic;
    assert!(!s.show_tweaks(), "generic media hides Windows tweaks");

    s.media = MediaKind::Windows;
    assert!(s.show_tweaks());
}

#[test]
fn no_iso_hides_tweaks() {
    let s = Ferrus::default();
    assert!(!s.show_tweaks());
}

#[test]
fn media_detection_applies_to_current_iso() {
    let mut s = ready(); // iso = /images/win.iso, media Unknown
    let _ = s.update(Message::MediaDetected(
        "/images/win.iso".into(),
        MediaKind::Windows,
    ));
    assert_eq!(s.media, MediaKind::Windows);
}

#[test]
fn stale_media_detection_is_ignored() {
    let mut s = ready();
    // A result for a different (already replaced) image must not apply.
    let _ = s.update(Message::MediaDetected(
        "/other.iso".into(),
        MediaKind::Generic,
    ));
    assert_eq!(s.media, MediaKind::Unknown, "stale detection ignored");
}

// --- update transitions ---------------------------------------------------

#[test]
fn devices_loaded_populates_and_clears_loading() {
    let mut s = Ferrus {
        loading_devices: true,
        ..Ferrus::default()
    };
    let _ = s.update(Message::DevicesLoaded(Ok(vec![
        dev("/dev/sdb"),
        dev("/dev/sdc"),
    ])));
    assert!(!s.loading_devices);
    assert_eq!(s.devices.len(), 2);
    assert!(s.device_error.is_none());
}

#[test]
fn selection_is_dropped_when_device_disappears() {
    let mut s = Ferrus::default();
    let _ = s.update(Message::SelectDevice(dev("/dev/sdb")));
    assert!(s.selected.is_some());
    // A refresh returns a list without the previously selected device.
    let _ = s.update(Message::DevicesLoaded(Ok(vec![dev("/dev/sdc")])));
    assert!(s.selected.is_none(), "stale selection must be cleared");
}

#[test]
fn device_enumeration_error_is_surfaced() {
    let mut s = Ferrus::default();
    let _ = s.update(Message::DevicesLoaded(Err("boom".to_owned())));
    assert_eq!(s.device_error.as_deref(), Some("boom"));
    assert!(s.devices.is_empty());
}

#[test]
fn iso_validation_error_clears_iso() {
    let mut s = ready();
    assert!(s.iso.is_some());
    let _ = s.update(Message::IsoValidated(Err("not a file".to_owned())));
    assert!(s.iso.is_none());
    assert_eq!(s.iso_error.as_deref(), Some("not a file"));
}
