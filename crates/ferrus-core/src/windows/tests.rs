//! Tests for the autounattend.xml generator (Phase 4, SPEC-0006).

use base64::Engine as _;

use super::{
    LocalAccountSpec, WindowsTweaks, default_profile, generate_autounattend, obfuscate_password,
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

fn account(name: &str, password: Option<&str>) -> LocalAccountSpec {
    LocalAccountSpec {
        name: name.to_owned(),
        password: password.map(str::to_owned),
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
        local_account: None,
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
        bypass_hardware: false,
        local_account: Some(account("nopass", None)),
    };
    let xml = generate_autounattend(&tweaks, default_profile()).unwrap();
    let doc = parse(&xml);
    assert!(doc.descendants().any(|n| n.has_tag_name("LocalAccount")));
    assert!(!doc.descendants().any(|n| n.has_tag_name("Password")));
}

// --- determinism ----------------------------------------------------------

#[test]
fn generation_is_deterministic() {
    let tweaks = WindowsTweaks {
        bypass_hardware: true,
        local_account: Some(account("cdh", Some("pw"))),
    };
    let a = generate_autounattend(&tweaks, default_profile()).unwrap();
    let b = generate_autounattend(&tweaks, default_profile()).unwrap();
    assert_eq!(a, b);
}

// --- secret hygiene -------------------------------------------------------

#[test]
fn password_plaintext_never_leaks() {
    let secret = "S3cr3t!Passw0rd";
    let tweaks = WindowsTweaks {
        bypass_hardware: false,
        local_account: Some(account("cdh", Some(secret))),
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
        bypass_hardware: false,
        local_account: Some(account("   ", None)),
    };
    assert!(generate_autounattend(&tweaks, default_profile()).is_err());
}

// --- XML escaping ---------------------------------------------------------

#[test]
fn account_name_is_xml_escaped() {
    let tweaks = WindowsTweaks {
        bypass_hardware: false,
        local_account: Some(account("a<b&c", None)),
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
