//! Ferrus privileged helper binary (SPEC-0008).
//!
//! Runs elevated (via `pkexec`). It accepts exactly one subcommand (`dry-run`),
//! reads one JSON [`Request`] from **stdin** (so any password never appears in
//! argv or the environment), performs the root-side re-validated dry-run, and
//! writes one JSON [`Response`] to stdout. No shell, no other verbs, no writes.

#![forbid(unsafe_code)]

use std::io::Read;
use std::process::ExitCode;

use ferrus_helper::{Request, Response, SUBCOMMAND, run_dry_run};

fn main() -> ExitCode {
    // Strict subcommand allow-list: exactly `dry-run`, nothing else, no extra args.
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some(SUBCOMMAND), None) => {}
        _ => {
            eprintln!("ferrus-helper: usage: ferrus-helper {SUBCOMMAND}  (JSON request on stdin)");
            return ExitCode::from(2);
        }
    }

    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut input) {
        return emit(Response {
            ok: false,
            log: Vec::new(),
            error: Some(format!("read stdin: {e}")),
        });
    }

    let request: Request = match serde_json::from_slice(&input) {
        Ok(request) => request,
        Err(e) => {
            return emit(Response {
                ok: false,
                log: Vec::new(),
                error: Some(format!("malformed request: {e}")),
            });
        }
    };

    emit(run_dry_run(&request))
}

/// Serialize the response to stdout and map `ok` to the process exit code. The
/// response carries no secret, so printing it is safe.
fn emit(response: Response) -> ExitCode {
    let ok = response.ok;
    let _ = serde_json::to_writer(std::io::stdout(), &response);
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
