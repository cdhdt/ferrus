//! Tests for the autounattend.xml generator (Phase 4 + 4.x, SPEC-0006).

use base64::Engine as _;

use super::{
    LocalAccountSpec, RegionSpec, WindowsTweaks, default_profile, generate_autounattend,
    obfuscate_password,
};

fn parse(xml: &str) -> roxmltree::Document<'_> {
    roxmltree::Document::parse(xml).expect("generated XML must be well-formed")
}

/// Whether the document has an element `tag` inside a `<settings pass="pass">`.
fn has_in_pass(doc: &roxmltree::Document, pass: &str, tag: &str) -> bool {
    doc.descendants()
        .filter(|n| n.has_tag_name("settings") && n.attribute("pass") == Some(pass))
        .any(|settings| settings.descendants().any(|n| n.has_tag_name(tag)))
}

/// Count `<settings pass="pass">` blocks (there must be at most one per pass).
fn settings_blocks(doc: &roxmltree::Document, pass: &str) -> usize {
    doc.descendants()
        .filter(|n| n.has_tag_name("settings") && n.attribute("pass") == Some(pass))
        .count()
}

fn account(name: &str, password: Option<&str>) -> LocalAccountSpec {
    LocalAccountSpec {
        name: name.to_owned(),
        password: password.map(str::to_owned),
    }
}

fn region(locale: &str) -> RegionSpec {
    RegionSpec {
        locale: locale.to_owned(),
    }
}

// --- opt-in behavior ------------------------------------------------------

#[test]
fn no_tweaks_means_nothing_to_generate() {
    let tweaks = WindowsTweaks::default();
    assert!(!tweaks.any());
    // The document, if generated at all, carries no settings passes.
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml);
    assert!(!doc.descendants().any(|n| n.has_tag_name("settings")));
}

// --- nominal (the point of the phase) -------------------------------------

#[test]
fn full_tweaks_produce_expected_wellformed_xml() {
    let tweaks = WindowsTweaks {
        bypass_hardware: true,
        local_account: Some(account("cdh", Some("hunter2"))),
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml); // well-formed or panics

    // Hardware bypass: the five LabConfig reg adds in the windowsPE pass.
    assert!(has_in_pass(&doc, "windowsPE", "RunSynchronousCommand"));
    assert!(xml.contains(r"HKLM\SYSTEM\Setup\LabConfig"));
    for key in default_profile().labconfig_keys {
        assert!(xml.contains(key), "missing LabConfig key {key}");
    }

    // BypassNRO complement in specialize.
    assert!(has_in_pass(&doc, "specialize", "RunSynchronousCommand"));
    assert!(xml.contains(r"CurrentVersion\OOBE") && xml.contains("BypassNRO"));

    // Local account + OOBE screens in oobeSystem.
    assert!(has_in_pass(&doc, "oobeSystem", "LocalAccount"));
    assert!(has_in_pass(&doc, "oobeSystem", "HideOnlineAccountScreens"));
    let name = doc
        .descendants()
        .find(|n| n.has_tag_name("Name"))
        .and_then(|n| n.text());
    assert_eq!(name, Some("cdh"));
}

#[test]
fn bypass_only_has_no_account_or_oobe() {
    let tweaks = WindowsTweaks {
        bypass_hardware: true,
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml);
    assert!(has_in_pass(&doc, "windowsPE", "RunSynchronousCommand"));
    assert!(!doc.descendants().any(|n| n.has_tag_name("LocalAccount")));
    assert!(!has_in_pass(&doc, "specialize", "RunSynchronousCommand"));
}

#[test]
fn account_without_password_omits_password_element() {
    let tweaks = WindowsTweaks {
        local_account: Some(account("nopass", None)),
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml);
    assert!(doc.descendants().any(|n| n.has_tag_name("LocalAccount")));
    assert!(!doc.descendants().any(|n| n.has_tag_name("Password")));
}

// --- phase 4.x levers -----------------------------------------------------

#[test]
fn telemetry_and_bitlocker_land_in_specialize() {
    let tweaks = WindowsTweaks {
        minimize_telemetry: true,
        disable_auto_bitlocker: true,
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml);

    assert!(has_in_pass(&doc, "specialize", "RunSynchronousCommand"));
    // Telemetry minimum + the four HKLM privacy policies.
    assert!(xml.contains(r"DataCollection") && xml.contains("AllowTelemetry"));
    assert!(xml.contains("DisabledByGroupPolicy")); // advertising ID
    assert!(xml.contains("DisableLocation"));
    assert!(xml.contains("AllowFindMyDevice"));
    assert!(xml.contains("DoNotShowFeedbackNotifications"));
    // BitLocker prevention.
    assert!(xml.contains("PreventDeviceEncryption"));
    // No account/region requested → no oobeSystem block.
    assert_eq!(settings_blocks(&doc, "oobeSystem"), 0);
}

#[test]
fn region_lands_in_oobe_international_core() {
    let tweaks = WindowsTweaks {
        region: Some(region("fr-FR")),
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml);

    assert!(has_in_pass(&doc, "oobeSystem", "UILanguage"));
    assert!(has_in_pass(&doc, "oobeSystem", "InputLocale"));
    for tag in ["InputLocale", "SystemLocale", "UILanguage", "UserLocale"] {
        let val = doc
            .descendants()
            .find(|n| n.has_tag_name(tag))
            .and_then(|n| n.text());
        assert_eq!(val, Some("fr-FR"), "{tag} should carry the locale");
    }
}

#[test]
fn new_levers_off_are_absent() {
    let tweaks = WindowsTweaks {
        bypass_hardware: true,
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    assert!(!xml.contains("AllowTelemetry"));
    assert!(!xml.contains("PreventDeviceEncryption"));
    assert!(!xml.contains("International-Core"));
}

#[test]
fn all_specialize_commands_share_one_block() {
    // telemetry (5) + bitlocker (1) + bypassnro (1) = 7 commands, one settings.
    let tweaks = WindowsTweaks {
        local_account: Some(account("cdh", None)),
        minimize_telemetry: true,
        disable_auto_bitlocker: true,
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml);

    assert_eq!(settings_blocks(&doc, "specialize"), 1);
    let cmds = doc
        .descendants()
        .filter(|n| n.has_tag_name("settings") && n.attribute("pass") == Some("specialize"))
        .flat_map(|s| s.descendants())
        .filter(|n| n.has_tag_name("RunSynchronousCommand"))
        .count();
    assert_eq!(cmds, 7);
    assert!(xml.contains("BypassNRO"));
}

#[test]
fn region_and_account_share_one_oobe_block() {
    let tweaks = WindowsTweaks {
        local_account: Some(account("cdh", None)),
        region: Some(region("fr-FR")),
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml);
    assert_eq!(settings_blocks(&doc, "oobeSystem"), 1);
    assert!(has_in_pass(&doc, "oobeSystem", "UILanguage"));
    assert!(has_in_pass(&doc, "oobeSystem", "LocalAccount"));
}

#[test]
fn empty_region_locale_is_refused() {
    let tweaks = WindowsTweaks {
        region: Some(region("   ")),
        ..WindowsTweaks::default()
    };
    assert!(generate_autounattend(&tweaks, default_profile()).is_err());
}

// --- non-regression on the Phase 4 levers ---------------------------------

#[test]
fn phase4_output_is_unchanged_by_new_toggles_off() {
    let phase4 = WindowsTweaks {
        bypass_hardware: true,
        local_account: Some(account("cdh", Some("pw"))),
        ..WindowsTweaks::default()
    };
    let explicit_off = WindowsTweaks {
        bypass_hardware: true,
        local_account: Some(account("cdh", Some("pw"))),
        minimize_telemetry: false,
        disable_auto_bitlocker: false,
        region: None,
    };
    let a = generate_autounattend(&phase4, default_profile()).unwrap();
    let b = generate_autounattend(&explicit_off, default_profile()).unwrap();
    assert_eq!(a, b);
    // And it carries none of the phase 4.x markers.
    assert!(!a.contains("AllowTelemetry"));
    assert!(!a.contains("PreventDeviceEncryption"));
    assert!(!a.contains("International-Core"));
}

// --- determinism ----------------------------------------------------------

#[test]
fn generation_is_deterministic() {
    let tweaks = WindowsTweaks {
        bypass_hardware: true,
        local_account: Some(account("cdh", Some("pw"))),
        ..WindowsTweaks::default()
    };
    let a = generate_autounattend(&tweaks, default_profile()).unwrap();
    let b = generate_autounattend(&tweaks, default_profile()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn determinism_holds_with_all_levers() {
    let tweaks = WindowsTweaks {
        bypass_hardware: true,
        local_account: Some(account("cdh", Some("pw"))),
        minimize_telemetry: true,
        disable_auto_bitlocker: true,
        region: Some(region("fr-FR")),
    };
    let a = generate_autounattend(&tweaks, default_profile()).unwrap();
    let b = generate_autounattend(&tweaks, default_profile()).unwrap();
    assert_eq!(a, b);
    parse(&a); // every pass present and still well-formed
    assert_eq!(settings_blocks(&parse(&a), "specialize"), 1);
    assert_eq!(settings_blocks(&parse(&a), "oobeSystem"), 1);
}

// --- secret hygiene -------------------------------------------------------

#[test]
fn password_plaintext_never_leaks() {
    let secret = "S3cr3t!Passw0rd";
    let tweaks = WindowsTweaks {
        local_account: Some(account("cdh", Some(secret))),
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();

    // The plaintext never appears in the XML (only the obfuscated form).
    assert!(!xml.contains(secret));
    // Nor in Debug output of the model.
    assert!(!format!("{tweaks:?}").contains(secret));
    assert!(format!("{tweaks:?}").contains("<redacted>"));
}

#[test]
fn invalid_empty_account_name_is_refused() {
    let tweaks = WindowsTweaks {
        local_account: Some(account("   ", None)),
        ..WindowsTweaks::default()
    };
    assert!(generate_autounattend(&tweaks, default_profile()).is_err());
}

// --- XML escaping ---------------------------------------------------------

#[test]
fn account_name_is_xml_escaped() {
    let tweaks = WindowsTweaks {
        local_account: Some(account("a<b&c", None)),
        ..WindowsTweaks::default()
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    assert!(xml.contains("a&lt;b&amp;c"));
    // Still well-formed, and the parsed name round-trips to the original.
    let doc = parse(&xml);
    let name = doc
        .descendants()
        .find(|n| n.has_tag_name("Name"))
        .and_then(|n| n.text());
    assert_eq!(name, Some("a<b&c"));
}

// --- password obfuscation format ------------------------------------------

#[test]
fn obfuscate_matches_utf16le_plus_password_suffix() {
    // Windows expects base64(UTF-16LE(password + "Password")).
    let encoded = obfuscate_password("pw");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&encoded)
        .unwrap();
    let utf16: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(String::from_utf16(&utf16).unwrap(), "pwPassword");
}
