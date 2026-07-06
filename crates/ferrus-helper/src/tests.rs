//! Helper contract + root-side tests (SPEC-0008).
//!
//! Honest scope: real polkit elevation is not unit-testable (validated manually).
//! What is covered here: the JSON contract (valid accepted, malformed rejected),
//! the `TweaksWire -> WindowsTweaks` mapping, secret hygiene at this boundary, and
//! the fail-closed behavior (never `ok` without a genuine elevated, valid target).

use super::{MAX_REQUEST_BYTES, Request, Response, TweaksWire, read_request, run_dry_run};

fn wire(name: Option<&str>, password: Option<&str>) -> TweaksWire {
    TweaksWire {
        bypass_hardware: true,
        account_name: name.map(str::to_owned),
        account_password: password.map(str::to_owned),
        minimize_telemetry: true,
        disable_auto_bitlocker: false,
        region: Some("fr-FR".to_owned()),
    }
}

fn request(target: &str) -> Request {
    Request {
        target: target.to_owned(),
        image: None,
        tweaks: wire(Some("ferrus"), Some("hunter2")),
    }
}

// --- contract --------------------------------------------------------------

#[test]
fn request_roundtrips_through_json() {
    let req = request("/dev/sdb");
    let json = serde_json::to_string(&req).unwrap();
    let back: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(back.target, "/dev/sdb");
    assert_eq!(back.tweaks.account_name.as_deref(), Some("ferrus"));
    assert_eq!(back.tweaks.account_password.as_deref(), Some("hunter2"));
    assert_eq!(back.tweaks.region.as_deref(), Some("fr-FR"));
}

#[test]
fn malformed_request_is_rejected() {
    assert!(serde_json::from_str::<Request>("not json").is_err());
    // Missing required field `target`.
    assert!(serde_json::from_str::<Request>(r#"{"image":null}"#).is_err());
}

#[test]
fn read_request_accepts_small_and_rejects_oversized() {
    // A legitimate request is far under the cap and round-trips.
    let json = serde_json::to_vec(&request("/dev/sdb")).unwrap();
    assert!((json.len() as u64) < MAX_REQUEST_BYTES);
    let req = read_request(json.as_slice()).expect("small request accepted");
    assert_eq!(req.target, "/dev/sdb");

    // Anything over the cap is rejected cleanly — bounded read, no panic.
    // (Request has no Debug — secret hygiene — so extract the Err via `.err()`.)
    let oversized = vec![b' '; (MAX_REQUEST_BYTES + 10) as usize];
    let err = read_request(oversized.as_slice())
        .err()
        .expect("oversized rejected");
    assert!(err.contains("exceeds"), "got: {err}");
}

#[test]
fn response_roundtrips_through_json() {
    let resp = Response {
        ok: true,
        log: vec!["a".to_owned(), "b".to_owned()],
        error: None,
    };
    let back: Response = serde_json::from_str(&serde_json::to_string(&resp).unwrap()).unwrap();
    assert!(back.ok);
    assert_eq!(back.log.len(), 2);
}

// --- mapping ---------------------------------------------------------------

#[test]
fn tweaks_wire_maps_to_core_including_password() {
    let tweaks = wire(Some("ferrus"), Some("hunter2")).to_tweaks();
    assert!(tweaks.bypass_hardware);
    assert!(tweaks.minimize_telemetry);
    assert!(!tweaks.disable_auto_bitlocker);
    let account = tweaks.local_account.expect("account present");
    assert_eq!(account.name, "ferrus");
    assert_eq!(account.password.as_deref(), Some("hunter2"));
    assert_eq!(tweaks.region.expect("region").locale, "fr-FR");

    // No account name → no local account.
    assert!(wire(None, None).to_tweaks().local_account.is_none());
}

// --- secret hygiene --------------------------------------------------------

#[test]
fn password_does_not_leak_through_the_core_debug() {
    let secret = "S3cr3t-Passw0rd";
    let tweaks = wire(Some("ferrus"), Some(secret)).to_tweaks();
    // WindowsTweaks derives Debug; LocalAccountSpec redacts the password.
    assert!(!format!("{tweaks:?}").contains(secret));
    // The wire types deliberately do not derive Debug, so they cannot be
    // debug-printed at all — enforced at compile time.
}

// --- fail closed -----------------------------------------------------------

#[test]
fn never_ok_for_a_bogus_target() {
    // Whether or not the test runs as root, a non-existent device must be
    // refused (not root → euid gate; root → not-a-block-device / SafeTarget).
    // Crucially: nothing is ever written (forced dry-run), and ok is false.
    let resp = run_dry_run(&request("/dev/ferrus-does-not-exist-xyz"));
    assert!(!resp.ok);
    assert!(resp.error.is_some());
}
