//! Ferrus privileged helper — contract + root-side logic + client (SPEC-0008).
//!
//! The **only** privileged component. It is a thin, re-validating shell over
//! `ferrus-core`: it parses a typed request, **re-validates everything on the
//! root side** (never trusting the caller), and calls the engine. It adds no
//! business logic of its own.
//!
//! Two verbs, each **hardcoding** its destructiveness — the `dry_run` flag is
//! never carried by the request (a caller must not be able to flip it):
//! - `dry-run` → `SafeTarget::acquire(.., dry_run = true)` (simulate).
//! - `write`   → `SafeTarget::acquire(.., dry_run = false)` (real, erases data).
//!
//! Progress is streamed to stdout as **NDJSON** (one [`HelperEvent`] per line,
//! flushed), so the GUI can show a live bar during the minutes-long write.

#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ferrus_core::device::{SafeTarget, list_all_devices};
use ferrus_core::partition::prepare_windows;
use ferrus_core::progress::{ProgressSink, Stage};
use ferrus_core::source::RawImage;
use ferrus_core::windows::{LocalAccountSpec, RegionSpec, WindowsTweaks};

use serde::{Deserialize, Serialize};

/// Simulate: forced dry-run. Bound in the polkit action's `exec.argv1`.
pub const SUBCOMMAND_DRY_RUN: &str = "dry-run";
/// Real write: **erases the target**. Bound in a separate polkit action.
pub const SUBCOMMAND_WRITE: &str = "write";

/// Whether `verb` is one of the two accepted subcommands — the argv allow-list.
#[must_use]
pub fn accepted_verb(verb: &str) -> bool {
    verb == SUBCOMMAND_DRY_RUN || verb == SUBCOMMAND_WRITE
}

/// Maximum accepted request size on stdin. A legitimate request is well under
/// 1 KiB; 64 KiB is a generous ceiling that still bounds a root binary's input
/// defensively — no unbounded `read_to_end`.
pub const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// Read and parse exactly one [`Request`] from `reader`, rejecting input larger
/// than [`MAX_REQUEST_BYTES`] (bounded read: over the cap → error, never an
/// unbounded allocation).
///
/// # Errors
///
/// Returns a message if the read fails, the input exceeds the cap, or the JSON is
/// malformed.
pub fn read_request(reader: impl Read) -> Result<Request, String> {
    let mut input = Vec::new();
    reader
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut input)
        .map_err(|e| format!("read stdin: {e}"))?;
    if input.len() as u64 > MAX_REQUEST_BYTES {
        return Err(format!("request exceeds {MAX_REQUEST_BYTES} bytes"));
    }
    serde_json::from_slice(&input).map_err(|e| format!("malformed request: {e}"))
}

/// Wire form of the Windows tweaks. **No `Debug` derive** — it may carry a
/// password, which must never reach a log line. Note: **no `dry_run` field** —
/// destructiveness is decided by the subcommand, never by request data.
#[derive(Clone, Serialize, Deserialize)]
pub struct TweaksWire {
    /// Bypass the Windows 11 hardware checks.
    pub bypass_hardware: bool,
    /// Local account name (enables the local account when `Some`).
    pub account_name: Option<String>,
    /// Local account password (optional). Travels only on stdin; never logged.
    pub account_password: Option<String>,
    /// Minimize telemetry / data collection.
    pub minimize_telemetry: bool,
    /// Disable automatic BitLocker device encryption.
    pub disable_auto_bitlocker: bool,
    /// Regional preset (BCP-47 tag) when `Some`.
    pub region: Option<String>,
}

impl TweaksWire {
    /// Rebuild the core [`WindowsTweaks`] on the root side.
    fn to_tweaks(&self) -> WindowsTweaks {
        WindowsTweaks {
            bypass_hardware: self.bypass_hardware,
            local_account: self.account_name.as_ref().map(|name| LocalAccountSpec {
                name: name.clone(),
                password: self.account_password.clone(),
            }),
            minimize_telemetry: self.minimize_telemetry,
            disable_auto_bitlocker: self.disable_auto_bitlocker,
            region: self.region.as_ref().map(|locale| RegionSpec {
                locale: locale.clone(),
            }),
        }
    }
}

/// The request the GUI sends on stdin. **No `Debug` derive** (may carry a
/// password) and **no `dry_run` field**. The `target` is only a *proposal*; it is
/// re-validated on the root side.
#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    /// Device path the GUI proposes (e.g. `/dev/sdb`).
    pub target: String,
    /// Optional image path.
    pub image: Option<String>,
    /// Tweaks (mirrors [`WindowsTweaks`]).
    pub tweaks: TweaksWire,
}

/// One streamed progress event (NDJSON, one per line). Carries **no secret**, so
/// it is safe to print/log. `Result` is always the final event.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HelperEvent {
    /// Entered a new stage.
    Stage {
        /// Stage label (e.g. `Copying`).
        stage: String,
    },
    /// Progress within a stage: `done`/`total` bytes (or items). `total` is
    /// `None` when unknown.
    Advance {
        /// Work done so far.
        done: u64,
        /// Total work, if known.
        total: Option<u64>,
    },
    /// A free-form status line.
    Message {
        /// The status text.
        text: String,
    },
    /// Terminal event: the operation finished.
    Result {
        /// Whether it succeeded.
        ok: bool,
        /// Error message when `ok` is false.
        error: Option<String>,
    },
}

/// A [`ProgressSink`] that streams each event as one flushed NDJSON line.
struct StreamSink<W: Write> {
    out: W,
}

impl<W: Write> StreamSink<W> {
    /// Serialize one event as a line and flush. Errors are ignored — a broken
    /// pipe just means the client went away.
    fn emit(&mut self, event: &HelperEvent) {
        let _ = serde_json::to_writer(&mut self.out, event);
        let _ = self.out.write_all(b"\n");
        let _ = self.out.flush();
    }
}

impl<W: Write> ProgressSink for StreamSink<W> {
    fn stage(&mut self, stage: Stage) {
        self.emit(&HelperEvent::Stage {
            stage: format!("{stage:?}"),
        });
    }
    fn advance(&mut self, done: u64, total: Option<u64>) {
        self.emit(&HelperEvent::Advance { done, total });
    }
    fn message(&mut self, text: &str) {
        self.emit(&HelperEvent::Message {
            text: text.to_owned(),
        });
    }
}

/// Root-side preparation shared by both verbs. `dry_run` is passed by the caller
/// (each subcommand hardcodes it — it is **never** in the request): assert we are
/// actually root, **re-validate the target via the core** (never trust the
/// caller), validate the image, and run the engine.
fn run_prepared(
    request: &Request,
    dry_run: bool,
    sink: &mut dyn ProgressSink,
) -> Result<(), String> {
    // 1. Fail closed unless actually elevated.
    match ferrus_core::platform::effective_uid() {
        Ok(0) => {}
        Ok(uid) => return Err(format!("helper is not root (euid {uid})")),
        Err(e) => return Err(format!("cannot read effective uid: {e}")),
    }

    // 2. Re-validate the target ON THE ROOT SIDE. The GUI proposes; we dispose:
    //    re-enumerate, then run the single safety checkpoint ourselves with the
    //    caller-independent `dry_run` for this verb.
    let target_path = Path::new(&request.target);
    let device = list_all_devices()
        .map_err(|e| format!("enumeration failed: {e}"))?
        .into_iter()
        .find(|d| d.path == target_path)
        .ok_or_else(|| format!("{} is not a block device on this host", request.target))?;
    let target = SafeTarget::acquire(device, target_path, dry_run)
        .map_err(|e| format!("target rejected by the safety checkpoint: {e}"))?;

    // 3. Validate the image, if any.
    let image = match &request.image {
        Some(path) => Some(
            RawImage::open(Path::new(path)).map_err(|e| format!("cannot use image {path}: {e}"))?,
        ),
        None => None,
    };

    // 4. Run the engine (streaming progress through `sink`).
    let tweaks = request.tweaks.to_tweaks();
    let tweaks_opt = if tweaks.any() { Some(&tweaks) } else { None };
    prepare_windows(&target, image.as_ref(), tweaks_opt, false, sink).map_err(|e| e.to_string())
}

/// Serve one request with `dry_run`, streaming NDJSON events (including the final
/// `Result`) to `out`. Returns whether it succeeded (for the process exit code).
fn serve(request: &Request, dry_run: bool, out: impl Write) -> bool {
    let mut sink = StreamSink { out };
    match run_prepared(request, dry_run, &mut sink) {
        Ok(()) => {
            sink.emit(&HelperEvent::Result {
                ok: true,
                error: None,
            });
            true
        }
        Err(e) => {
            sink.emit(&HelperEvent::Result {
                ok: false,
                error: Some(e),
            });
            false
        }
    }
}

/// `dry-run` verb: **forced simulation** (`dry_run = true` is a literal here).
pub fn serve_dry_run(request: &Request, out: impl Write) -> bool {
    serve(request, true, out)
}

/// `write` verb: **real write** (`dry_run = false` is a literal here). Erases the
/// target.
pub fn serve_write(request: &Request, out: impl Write) -> bool {
    serve(request, false, out)
}

// --- client side (used by the unprivileged GUI) ---------------------------

/// Resolve the helper binary, **installed path first** (SPEC-0008).
///
/// Priority is deliberate and security-relevant:
/// 1. `/usr/libexec/ferrus-helper` — the installed, root-owned helper. In
///    production it exists and its path is exactly the `exec.path` locked into the
///    **named** polkit actions, so a compromised environment **cannot** redirect
///    the GUI to an arbitrary binary via `$FERRUS_HELPER` (which is ignored here).
/// 2. `$FERRUS_HELPER` — dev override, consulted **only** when nothing is
///    installed (in dev the helper is not in a system path).
/// 3. a sibling of the current executable (`target/debug/ferrus-helper`).
///
/// Note: if a developer has *also* run `make install`, the installed helper
/// shadows `$FERRUS_HELPER` (the secure behavior); `make uninstall` returns to the
/// dev workflow.
#[must_use]
pub fn resolve_helper_path() -> Option<PathBuf> {
    let installed = PathBuf::from("/usr/libexec/ferrus-helper");
    if installed.exists() {
        return Some(installed);
    }
    if let Some(path) = std::env::var_os("FERRUS_HELPER") {
        return Some(PathBuf::from(path));
    }
    let sibling = std::env::current_exe()
        .ok()?
        .parent()?
        .join("ferrus-helper");
    sibling.exists().then_some(sibling)
}

/// Spawn the helper **elevated via `pkexec`** for `verb` (`dry-run` or `write`),
/// send `request` on stdin (so the password never touches argv or the
/// environment), and stream each stdout NDJSON line to `on_event` until EOF.
///
/// **Blocking** — run it on a background thread and forward the events into an
/// async channel (the GUI does exactly this). No `wait_with_output`: events are
/// delivered live.
///
/// # Errors
///
/// Returns a message if the helper cannot be spawned, or if authentication is
/// dismissed/denied (no `Result` event was produced).
pub fn run_streaming(
    helper: &Path,
    verb: &str,
    request: &Request,
    mut on_event: impl FnMut(HelperEvent),
) -> Result<(), String> {
    let payload = serde_json::to_vec(request).map_err(|e| format!("encode request: {e}"))?;

    let mut child = Command::new("pkexec")
        .arg(helper)
        .arg(verb)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot spawn pkexec: {e}"))?;

    // Write and close stdin so the helper sees EOF, then stream stdout by line.
    child
        .stdin
        .take()
        .ok_or_else(|| "no stdin pipe to the helper".to_owned())?
        .write_all(&payload)
        .map_err(|e| format!("write request: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "no stdout pipe from the helper".to_owned())?;
    let mut saw_result = false;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| format!("read helper output: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<HelperEvent>(&line) {
            saw_result |= matches!(event, HelperEvent::Result { .. });
            on_event(event);
        }
        // Non-JSON noise on stdout is ignored, not fatal.
    }

    let status = child
        .wait()
        .map_err(|e| format!("helper did not complete: {e}"))?;
    if !saw_result {
        // No terminal event → pkexec dismissed (126) / not authorized (127) / crash.
        let mut stderr = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut stderr);
        }
        return Err(format!("elevation failed ({status}): {}", stderr.trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
