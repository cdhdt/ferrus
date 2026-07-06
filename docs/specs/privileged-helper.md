# SPEC-0008: privileged helper + polkit elevation + type-to-confirm (Phase 5b-1 + 5b-2)

- **Status:** Implemented
- **Modules:** `ferrus-helper` (new bin + thin lib); `ferrus-gui` (confirm flow +
  client); `ferrus-core` (unchanged engine; one small public accessor added).
- **Linked ADRs:** ADR-0003 (privilege elevation — Option B, polkit + helper).
- **Linked specs:** SPEC-0001 (`SafeTarget`/EUID gate), SPEC-0006 (`WindowsTweaks`),
  SPEC-0007 (GUI).

## Scope — elevate a minimal helper to prepare a device

The privileged path end to end: (1) polkit elevates a minimal helper (`pkexec`);
(2) the (unprivileged) GUI streams the operation from it; (3) a **type-to-confirm**
gate blocks the action until the user types the exact target path. Phase 5b-1
delivered this for a forced dry-run; **Phase 5b-2 adds the real write** behind the
same guards.

**Phase 5b-2 adds the real write.** The single boolean that separates *simulate*
from *erase* is chosen by the **subcommand**, never by request data (see below):
- `dry-run` → re-enumeration + `prepare_windows(dry_run = true)` (simulate).
- `write` → re-enumeration + `prepare_windows(dry_run = false)` (**erases the
  target**).

Both paths share the exact same root-side re-validation; only the hardcoded flag
differs. Progress is **streamed** back so the GUI shows a live bar during the
minutes-long copy.

## Architecture

```
ferrus-gui (USER, never root)
   │  spawns:  pkexec  <helper>  <dry-run | write>
   │  stdin  → JSON Request  (incl. any password — never in argv/env; NO dry_run)
   │  stdout ← NDJSON events (one per line): progress… then a final result
   ▼
ferrus-helper (ROOT, minimal, audited)
   │  re-validates EVERYTHING via ferrus-core (does not trust the GUI)
   │  hardcodes dry_run per verb (true for dry-run, false for write)
   ▼
ferrus-core (the engine — unchanged, already real-world validated)
```

The helper is a **thin shell** over `ferrus-core`: it parses input, **re-validates
on the root side**, and calls the core. It contains **no new business logic** —
if it ever needs to, that logic is missing from the core and must be added there,
not in the helper.

## Threat model — GUI input is HOSTILE

The helper runs as root and must assume its input is attacker-controlled (a
compromised or buggy GUI, or a hand-fed stdin). Therefore:

- **Root-side re-validation (the core guarantee).** The helper does **not** trust
  the device the GUI names as "safe". It re-enumerates with
  `device::list_all_devices()`, finds the device by the given path, and runs
  `SafeTarget::acquire(device, &path, dry_run)` **itself** — re-checking
  removable-transport, not-system/critical, exact path match, and the live TOCTOU
  re-check. The GUI proposes; the helper disposes. A device that fails is refused.
  **Identical for both verbs** — the `write` path re-validates exactly like
  `dry-run`.
- **`dry_run` is NEVER caller data (load-bearing).** The request carries **no**
  `dry_run` field. Destructiveness is decided solely by the **subcommand**, and
  each verb passes a *literal* to the core (`serve_dry_run` → `true`,
  `serve_write` → `false`). A compromised caller cannot flip a boolean to turn a
  simulate into an erase; it would have to invoke the `write` verb, which is a
  separate polkit action requiring `auth_admin`.
- **Minimal, typed contract.** stdin carries exactly one JSON `Request`
  (`target`, optional `image`, `tweaks`); anything malformed is rejected with a
  terminal `result` event. No positional trust.
- **Subcommand allow-list.** argv must be exactly `dry-run` **or** `write` —
  those two, nothing else (`accepted_verb`). No shell, no arbitrary command
  execution, no other verbs.
- **EUID assertion.** The helper checks `platform::effective_uid() == 0` and
  refuses if not actually elevated (proves the elevation, and fails closed).
- **Bounded stdin.** The request is read with a hard cap (`MAX_REQUEST_BYTES` =
  64 KiB) via `Read::take`, not an unbounded `read_to_end`. A legitimate request
  is well under 1 KiB; anything larger is rejected cleanly (`ok: false`), never
  read into an unbounded allocation. Defense in depth on a root binary.

## Secret handling

A local-account password, if present, travels **inside the JSON Request on
stdin** — never in `argv` (visible in `/proc/*/cmdline` and `ps`) and never in the
environment (pkexec sanitizes it anyway). The wire types (`Request`, `TweaksWire`)
do **not** derive `Debug`, and the helper never logs the request, so the password
cannot leak through a log line.

## polkit integration (verified against the local polkit)

Verified on the target host — **polkit `126`** (`pkexec --version`) — using the
`pkexec(1)` man page and a real installed action
(`/usr/share/polkit-1/actions/com.ubuntu.pkexec.gdebi-gtk.policy`):

- **Invocation:** `pkexec <abs-helper-path> dry-run`. Per `pkexec(1)`: *"By
  default, the `org.freedesktop.policykit.exec` action is used"* — so pkexec runs
  an arbitrary program path after authentication; a custom action is only needed
  to customize the message/auth.
- **Environment is sanitized:** *"set to a minimal known and safe environment … to
  avoid injecting code through `LD_LIBRARY_PATH`"*, and `PKEXEC_UID` is set to the
  caller's uid. → the secret must **not** ride on env (it rides on stdin).
- **stdin/stdout are inherited** (pkexec does not close fds 0/1/2), so the JSON
  request on stdin and the NDJSON stream on stdout work.
- **Two actions, one per verb** (`res/polkit/io.github.cdhdt.ferrus.policy`).
  Per `pkexec(1)`: *"If the `org.freedesktop.policykit.exec.argv1` annotation is
  present, the action will only be picked if the first argument to the program
  matches the value"*. So an action can bind **one** argv1 → **one action per
  subcommand** (Option A) is the correct structure: `io.github.cdhdt.ferrus.dryrun`
  (argv1 = `dry-run`) and `io.github.cdhdt.ferrus.write` (argv1 = `write`), same
  `exec.path` (`/usr/libexec/ferrus-helper`), both `auth_admin`, with distinct
  messages (the write action's message states it **erases all data**). Install path
  `/usr/libexec/ferrus-helper` (root-owned).

### Helper resolution — installed path first (why the named action wins in prod)

`resolve_helper_path()` returns the helper in a deliberate order:
**`/usr/libexec/ferrus-helper` first**, then `$FERRUS_HELPER`, then a sibling of the
current exe. In production the installed, root-owned helper exists, so it wins and
`$FERRUS_HELPER` is **ignored** — a compromised environment cannot redirect the GUI
to an arbitrary binary. Because the GUI then runs exactly
`/usr/libexec/ferrus-helper` — the `exec.path` of the named actions — pkexec
matches the **named** action (with its `auth_admin` and its "erases all data"
message), not the permissive default action. In dev nothing is installed, so
`/usr/libexec/ferrus-helper` is absent and `$FERRUS_HELPER` (the local build) is
used; `make install` on a dev box would shadow it (secure), and `make uninstall`
restores the dev workflow.

`make install` installs the helper to precisely that path and the `.policy` next to
it, so **install and policy `exec.path` stay in lockstep** — otherwise the named
action would never match and pkexec would fall back to the default action.

## Progress streaming (NDJSON)

The write is long (multi-GB), so the helper→GUI channel is **line-oriented
NDJSON**: the helper writes one JSON `HelperEvent` per line to stdout and flushes
it immediately. The helper's `ProgressSink` maps directly to events:

```
{"type":"stage","stage":"Copying"}
{"type":"advance","done":123,"total":456}
{"type":"message","text":"…"}
{"type":"result","ok":true,"error":null}     ← always the final line
```

No secret ever appears in these lines (re-confirmed: only stage labels, byte
counts, and the engine's own status strings). The client (`run_streaming`) reads
stdout **line by line** on a background thread and forwards each parsed event into
an async channel; the GUI consumes it as an iced `Task::run` stream — no
`wait_with_output`, so the UI updates live and never blocks. No deadlock: the GUI
writes and closes the small stdin first, then only reads stdout.

### Manual test procedure (dev, no install)

`pkexec` runs any absolute path with the **default** action, so a clean install is
not required in dev. **Dry-run** (safe — streams NDJSON, writes nothing):

```sh
cargo build
echo '{"target":"/dev/sdX","image":null,"tweaks":{"bypass_hardware":false,
      "account_name":null,"account_password":null,"minimize_telemetry":false,
      "disable_auto_bitlocker":false,"region":null}}' \
  | pkexec "$PWD/target/debug/ferrus-helper" dry-run
```

**Write** (⚠ DESTRUCTIVE — only on a scratch device, e.g. the USB gadget):
substitute `write` for `dry-run` and a real `target`/`image`. It streams
`stage`/`advance`/`message` lines then a final `result`.

From the GUI (dev): `FERRUS_HELPER=$PWD/target/debug/ferrus-helper ferrus-gui`, in
a desktop session with a **polkit agent** running (it shows the password dialog).

### Testing the NAMED polkit action (after a real install)

This is the production path and needs root + a graphical session:

```sh
sudo make install
ferrus-gui            # launched WITHOUT FERRUS_HELPER (installed helper wins)
```

Select a device + ISO, type the exact path to unlock, click **Write**, and read
the polkit dialog. **How to tell which action fired:**

- **Named action (correct):** the dialog shows Ferrus's own message — *"Authentication
  is required for Ferrus to write to a device — THIS ERASES ALL DATA on it"* (or the
  French translation). This proves `/usr/libexec/ferrus-helper` matched the
  `io.github.cdhdt.ferrus.write` action via its `exec.argv1 = write`.
- **Default action (wrong / not installed):** the dialog shows the generic pkexec
  message — *"Authentication is required to run `/usr/libexec/ferrus-helper` as the
  super user"* (action `org.freedesktop.policykit.exec`), with no Ferrus-specific
  wording. Seeing this means the named action did not match — check that the helper
  is installed at exactly the `.policy`'s `exec.path` and the `.policy` is in
  `/usr/share/polkit-1/actions/`.

`sudo make uninstall` removes everything and returns to the dev workflow.

## Type-to-confirm (GUI, before ANY elevation)

Before the GUI will spawn the helper — for **either** verb — the user must **type
the exact target device path** (e.g. `/dev/sda`) into a field. Both the Simulate
and the **Write** buttons stay disabled until the typed text matches the selected
device path **exactly**; changing the selected device clears the match. This
requirement is **not relaxed for the write** — it is the primary guard against
wiping the wrong disk. The GUI shows unambiguously what will happen — device
(path/size/model/bus), image, enabled tweaks — and the write button is danger-
styled and labelled `Write — ERASES ALL DATA on <dev>`. During a run the buttons
are disabled and a live progress bar is shown.

## Testable vs manual

Unit-tested (no rendering, no real elevation, no real write): the type-to-confirm
predicate (blocked until exact match, re-blocked on device change), the contract
(valid Request accepted; malformed/hostile rejected), the argv allow-list (exactly
`{dry-run, write}`), `dry_run` absent from the request, the NDJSON event
round-trip, both verbs streaming a terminal `result` (fail-closed on a bogus
target), and secret hygiene (password absent from any `Debug`/argv). **Real polkit
elevation, the real write, and the rendering are validated manually** — not
unit-testable; stated, not papered over.

## Done in 5b-2 / nothing deferred

The real destructive run is implemented: the `write` verb calls the engine with
`dry_run = false`, and progress is streamed as NDJSON. Everything designed in 5b-1
(re-validation, secret channel, type-to-confirm, polkit) carried over unchanged;
only the verb's hardcoded flag differs and progress is streamed. The core
destructive pipeline itself was **not** modified — it was already validated on real
hardware (Phases 2–4).
