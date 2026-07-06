//! Ferrus privileged helper — contract + root-side logic + client (SPEC-0008).
//!
//! The **only** privileged component. It is a thin, re-validating shell over
//! `ferrus-core`: it parses a typed request, **re-validates everything on the
//! root side** (never trusting the caller), and calls the engine. It adds no
//! business logic of its own.
//!
//! Phase 5b-1: the sole privileged operation is a **forced dry-run** — no write,
//! format, partition or unmount is possible here. The real write is 5b-2.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ferrus_core::device::{SafeTarget, list_all_devices};
use ferrus_core::partition::prepare_windows;
use ferrus_core::progress::{ProgressSink, Stage};
use ferrus_core::source::RawImage;
use ferrus_core::windows::{LocalAccountSpec, RegionSpec, WindowsTweaks};

use serde::{Deserialize, Serialize};

/// The one subcommand the helper accepts (also bound in the polkit action's
/// `org.freedesktop.policykit.exec.argv1`).
pub const SUBCOMMAND: &str = "dry-run";

/// Maximum accepted request size on stdin. A legitimate request is well under
/// 1 KiB (a path, an optional image path, a handful of booleans, a short name and
/// password); 64 KiB is a generous ceiling that still bounds a root binary's input
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
/// password, which must never reach a log line.
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
/// password). The `target` is only a *proposal*; it is re-validated on the root
/// side.
#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    /// Device path the GUI proposes (e.g. `/dev/sdb`).
    pub target: String,
    /// Optional image path.
    pub image: Option<String>,
    /// Tweaks (mirrors [`WindowsTweaks`]).
    pub tweaks: TweaksWire,
}

/// The helper's reply on stdout. Carries no secret, so it is safe to print/log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Whether the (dry-run) operation succeeded.
    pub ok: bool,
    /// Human-readable simulated steps.
    pub log: Vec<String>,
    /// Error message when `ok` is false.
    pub error: Option<String>,
}

impl Response {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            log: Vec::new(),
            error: Some(message.into()),
        }
    }
}

/// Root-side execution: assert we are actually root, **re-validate the target via
/// the core** (never trust the caller), then run `prepare_windows` in a **forced
/// dry-run**. This is the entire privileged surface; every decision is a
/// `ferrus-core` call.
#[must_use]
pub fn run_dry_run(request: &Request) -> Response {
    // 1. Fail closed unless actually elevated.
    match ferrus_core::platform::effective_uid() {
        Ok(0) => {}
        Ok(uid) => return Response::failed(format!("helper is not root (euid {uid})")),
        Err(e) => return Response::failed(format!("cannot read effective uid: {e}")),
    }

    // 2. Re-validate the target ON THE ROOT SIDE. The GUI proposes; we dispose:
    //    re-enumerate, then run the single safety checkpoint ourselves.
    let target_path = Path::new(&request.target);
    let device = match list_all_devices() {
        Ok(devices) => devices.into_iter().find(|d| d.path == target_path),
        Err(e) => return Response::failed(format!("enumeration failed: {e}")),
    };
    let Some(device) = device else {
        return Response::failed(format!(
            "{} is not a block device on this host",
            request.target
        ));
    };
    let target = match SafeTarget::acquire(device, target_path, true) {
        Ok(target) => target,
        Err(e) => {
            return Response::failed(format!("target rejected by the safety checkpoint: {e}"));
        }
    };

    // 3. Validate the image, if any.
    let image = match &request.image {
        Some(path) => match RawImage::open(Path::new(path)) {
            Ok(img) => Some(img),
            Err(e) => return Response::failed(format!("cannot use image {path}: {e}")),
        },
        None => None,
    };

    // 4. Forced dry-run — a real write is structurally impossible in 5b-1.
    let tweaks = request.tweaks.to_tweaks();
    let tweaks_opt = if tweaks.any() { Some(&tweaks) } else { None };
    let mut sink = LogSink::default();
    match prepare_windows(&target, image.as_ref(), tweaks_opt, false, &mut sink) {
        Ok(()) => Response {
            ok: true,
            log: sink.lines,
            error: None,
        },
        Err(e) => Response {
            ok: false,
            log: sink.lines,
            error: Some(e.to_string()),
        },
    }
}

/// A [`ProgressSink`] collecting the simulated steps as text.
#[derive(Default)]
struct LogSink {
    lines: Vec<String>,
}

impl ProgressSink for LogSink {
    fn stage(&mut self, stage: Stage) {
        self.lines.push(format!("[{stage:?}]"));
    }
    fn advance(&mut self, _done: u64, _total: Option<u64>) {}
    fn message(&mut self, text: &str) {
        self.lines.push(text.to_owned());
    }
}

// --- client side (used by the unprivileged GUI) ---------------------------

/// Resolve the helper binary: `$FERRUS_HELPER`, else `/usr/libexec/ferrus-helper`,
/// else a sibling of the current executable (dev layout).
#[must_use]
pub fn resolve_helper_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FERRUS_HELPER") {
        return Some(PathBuf::from(path));
    }
    let installed = PathBuf::from("/usr/libexec/ferrus-helper");
    if installed.exists() {
        return Some(installed);
    }
    let sibling = std::env::current_exe()
        .ok()?
        .parent()?
        .join("ferrus-helper");
    sibling.exists().then_some(sibling)
}

/// Spawn the helper **elevated via `pkexec`**, sending `request` on stdin (so the
/// password never touches argv or the environment) and parsing the JSON
/// [`Response`] from stdout.
///
/// # Errors
///
/// Returns a message if the helper cannot be spawned, if authentication is
/// dismissed/denied, or if the reply cannot be parsed.
pub fn run_elevated(helper: &Path, request: &Request) -> Result<Response, String> {
    let payload = serde_json::to_vec(request).map_err(|e| format!("encode request: {e}"))?;

    let mut child = Command::new("pkexec")
        .arg(helper)
        .arg(SUBCOMMAND)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot spawn pkexec: {e}"))?;

    // Write and close stdin so the helper sees EOF, then collect the output.
    child
        .stdin
        .take()
        .ok_or_else(|| "no stdin pipe to the helper".to_owned())?
        .write_all(&payload)
        .map_err(|e| format!("write request: {e}"))?;

    let output = child
        .wait_with_output()
        .map_err(|e| format!("helper did not complete: {e}"))?;

    if output.stdout.is_empty() {
        // No JSON body → pkexec dismissed (126) / not authorized (127), or a crash.
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "elevation failed ({}): {}",
            output.status,
            stderr.trim()
        ));
    }
    serde_json::from_slice::<Response>(&output.stdout).map_err(|e| format!("bad helper reply: {e}"))
}

#[cfg(test)]
mod tests;
