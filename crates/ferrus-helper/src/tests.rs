//! Helper contract + root-side tests (SPEC-0008).
//!
//! Honest scope: real polkit elevation and the real write are not unit-testable
//! (validated manually). Covered here: the JSON contract, the NDJSON streaming
//! events, the argv allow-list, `dry_run` being absent from the request,
//! `TweaksWire -> WindowsTweaks` mapping, secret hygiene, and fail-closed
//! behavior (never a successful terminal event without a genuine elevated, valid
//! target).

use super::{
    HelperEvent, MAX_REQUEST_BYTES, Request, TweaksWire, accepted_verb, read_request,
    serve_dry_run, serve_write,
};

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

/// Parse the NDJSON lines a `serve_*` call wrote into a buffer.
fn events(buf: &[u8]) -> Vec<HelperEvent> {
    String::from_utf8_lossy(buf)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<HelperEvent>(l).expect("valid NDJSON event"))
        .collect()
}

// --- contract --------------------------------------------------------------

#[test]
fn request_roundtrips_through_json() {
    let req = request("/dev/sdb");
    let back: Request = serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
    assert_eq!(back.target, "/dev/sdb");
    assert_eq!(back.tweaks.account_name.as_deref(), Some("ferrus"));
    assert_eq!(back.tweaks.account_password.as_deref(), Some("hunter2"));
}

#[test]
fn malformed_request_is_rejected() {
    assert!(serde_json::from_str::<Request>("not json").is_err());
    assert!(serde_json::from_str::<Request>(r#"{"image":null}"#).is_err());
}

#[test]
fn request_has_no_dry_run_field() {
    // dry_run must never be caller-controlled: the type does not carry it.
    let json = serde_json::to_string(&request("/dev/sdb")).unwrap();
    assert!(
        !json.contains("dry_run"),
        "request must not carry dry_run: {json}"
    );
}

// --- NDJSON streaming events ----------------------------------------------

#[test]
fn helper_events_ndjson_roundtrip() {
    let cases = [
        HelperEvent::Stage {
            stage: "Copying".to_owned(),
        },
        HelperEvent::Advance {
            done: 42,
            total: Some(100),
        },
        HelperEvent::Message {
            text: "hello".to_owned(),
        },
        HelperEvent::Result {
            ok: true,
            error: None,
        },
    ];
    for ev in cases {
        let line = serde_json::to_string(&ev).unwrap();
        assert!(line.contains("\"type\":"), "tagged: {line}");
        assert_eq!(serde_json::from_str::<HelperEvent>(&line).unwrap(), ev);
    }
}

// --- argv allow-list -------------------------------------------------------

#[test]
fn verb_allow_list_is_exactly_two() {
    assert!(accepted_verb("dry-run"));
    assert!(accepted_verb("write"));
    for bad in ["", "format", "run", "DRY-RUN", "write ", "dry-run;write"] {
        assert!(!accepted_verb(bad), "must reject {bad:?}");
    }
}

// --- mapping ---------------------------------------------------------------

#[test]
fn tweaks_wire_maps_to_core_including_password() {
    let tweaks = wire(Some("ferrus"), Some("hunter2")).to_tweaks();
    assert!(tweaks.bypass_hardware);
    let account = tweaks.local_account.expect("account present");
    assert_eq!(account.name, "ferrus");
    assert_eq!(account.password.as_deref(), Some("hunter2"));
    assert_eq!(tweaks.region.expect("region").locale, "fr-FR");
    assert!(wire(None, None).to_tweaks().local_account.is_none());
}

// --- secret hygiene --------------------------------------------------------

#[test]
fn password_does_not_leak_through_the_core_debug() {
    let secret = "S3cr3t-Passw0rd";
    let tweaks = wire(Some("ferrus"), Some(secret)).to_tweaks();
    assert!(!format!("{tweaks:?}").contains(secret));
    // Wire types have no Debug (secret hygiene) — enforced at compile time.
}

// --- bounded stdin ---------------------------------------------------------

#[test]
fn read_request_accepts_small_and_rejects_oversized() {
    let json = serde_json::to_vec(&request("/dev/sdb")).unwrap();
    assert!((json.len() as u64) < MAX_REQUEST_BYTES);
    assert_eq!(read_request(json.as_slice()).unwrap().target, "/dev/sdb");

    let oversized = vec![b' '; (MAX_REQUEST_BYTES + 10) as usize];
    let err = read_request(oversized.as_slice())
        .err()
        .expect("oversized rejected");
    assert!(err.contains("exceeds"), "got: {err}");
}

// --- fail closed (both verbs) ---------------------------------------------

fn assert_bogus_failed(ok: bool, buf: &[u8]) {
    assert!(!ok, "must not succeed on a bogus target");
    let evs = events(buf);
    let last = evs.last().expect("at least a Result event");
    assert!(
        matches!(last, HelperEvent::Result { ok: false, .. }),
        "terminal event must be Result{{ok:false}}, got {last:?}"
    );
}

#[test]
fn both_verbs_stream_a_failed_result_for_a_bogus_target() {
    // Whether or not the test runs as root, a non-existent device must be refused
    // and nothing written. Each verb streams a terminal Result{ok:false}.
    let target = "/dev/ferrus-does-not-exist-xyz";

    let mut dry = Vec::new();
    let ok = serve_dry_run(&request(target), &mut dry);
    assert_bogus_failed(ok, &dry);

    let mut wr = Vec::new();
    let ok = serve_write(&request(target), &mut wr);
    assert_bogus_failed(ok, &wr);
}
