//! Ferrus privileged helper binary (SPEC-0008).
//!
//! Runs elevated (via `pkexec`). It accepts exactly two subcommands — `dry-run`
//! (simulate) and `write` (real, erases the target) — reads one JSON [`Request`]
//! from **stdin** (so any password never appears in argv or the environment),
//! re-validates everything on the root side, and **streams** NDJSON progress
//! events to stdout. The subcommand alone decides destructiveness: the `dry_run`
//! flag is hardcoded per verb, never taken from the request. No shell, no other
//! verbs.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use ferrus_helper::{
    HelperEvent, SUBCOMMAND_DRY_RUN, SUBCOMMAND_WRITE, accepted_verb, read_request, serve_dry_run,
    serve_write,
};

fn main() -> ExitCode {
    // Strict subcommand allow-list: exactly `dry-run` or `write`, nothing else.
    let mut args = std::env::args().skip(1);
    let verb = match (args.next(), args.next()) {
        (Some(verb), None) if accepted_verb(&verb) => verb,
        _ => {
            eprintln!(
                "ferrus-helper: usage: ferrus-helper <{SUBCOMMAND_DRY_RUN}|{SUBCOMMAND_WRITE}>  \
                 (JSON request on stdin)"
            );
            return ExitCode::from(2);
        }
    };

    // Bounded stdin read (defensive on a root binary) + parse. A parse failure is
    // itself reported as a terminal NDJSON event so the client sees it.
    let request = match read_request(std::io::stdin()) {
        Ok(request) => request,
        Err(e) => return emit_error(&e),
    };

    let out = std::io::stdout();
    let ok = if verb == SUBCOMMAND_WRITE {
        serve_write(&request, out)
    } else {
        serve_dry_run(&request, out)
    };
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Emit a single terminal `Result` NDJSON event for an early failure.
fn emit_error(message: &str) -> ExitCode {
    let event = HelperEvent::Result {
        ok: false,
        error: Some(message.to_owned()),
    };
    if let Ok(line) = serde_json::to_string(&event) {
        println!("{line}");
    }
    ExitCode::FAILURE
}
