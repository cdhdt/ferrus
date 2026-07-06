# SPEC-0008: privileged helper + polkit elevation + type-to-confirm (Phase 5b-1)

- **Status:** Implemented
- **Modules:** `ferrus-helper` (new bin + thin lib); `ferrus-gui` (confirm flow +
  client); `ferrus-core` (unchanged engine; one small public accessor added).
- **Linked ADRs:** ADR-0003 (privilege elevation — Option B, polkit + helper).
- **Linked specs:** SPEC-0001 (`SafeTarget`/EUID gate), SPEC-0006 (`WindowsTweaks`),
  SPEC-0007 (GUI).

## Scope (5b-1) — elevation plumbing only, NO destruction

Prove the privileged path end to end **without ever risking a disk**:
1. polkit elevates a minimal helper (`pkexec`).
2. the (unprivileged) GUI talks to it and gets the result back,
3. a **type-to-confirm** gate blocks the action until the user types the exact
   target path.

The helper here runs **only** a non-destructive op as root: re-enumeration +
`prepare_windows` in **dry-run** (`dry_run = true`, forced). No write, format,
partition or unmount. The **real write is Phase 5b-2** and is out of scope.

## Architecture

```
ferrus-gui (USER, never root)
   │  spawns:  pkexec  <helper>  dry-run
   │  stdin  → JSON Request  (incl. any password — never in argv/env)
   │  stdout ← JSON Response (ok, log lines, error)
   ▼
ferrus-helper (ROOT, minimal, audited)
   │  re-validates EVERYTHING via ferrus-core (does not trust the GUI)
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
  `SafeTarget::acquire(device, &path, dry_run = true)` **itself** — re-checking
  removable-transport, not-system/critical, exact path match, and the live TOCTOU
  re-check. The GUI proposes; the helper disposes. A device that fails is refused.
- **Minimal, typed contract.** stdin carries exactly one JSON `Request`
  (`target`, optional `image`, `tweaks`); anything malformed is rejected with a
  `Response { ok: false, error }`. No positional trust.
- **Subcommand allow-list.** argv must be exactly `dry-run` — the only accepted
  subcommand. No shell, no arbitrary command execution, no other verbs.
- **EUID assertion.** The helper checks `platform::effective_uid() == 0` and
  refuses if not actually elevated (proves the elevation, and fails closed).
- **Dry-run is forced.** The helper always calls the engine with `dry_run = true`
  in 5b-1; a real write is structurally impossible here regardless of input.
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
  Request/Response over stdin/stdout works.
- **Shipped policy** (`res/polkit/io.github.cdhdt.ferrus.prepare.policy`),
  following the format of the real system action above:
  action id `io.github.cdhdt.ferrus.prepare`, `<defaults>` all `auth_admin`,
  `<annotate key="org.freedesktop.policykit.exec.path">/usr/libexec/ferrus-helper</annotate>`,
  and `<annotate key="org.freedesktop.policykit.exec.argv1">dry-run</annotate>`
  to bind the action to that one subcommand. Install path
  `/usr/libexec/ferrus-helper` (root-owned).

### Manual test procedure (dev, no install)

`pkexec` runs any absolute path with the **default** action, so a clean install is
not required to prove elevation in dev:

```sh
cargo build
# feed a JSON request on stdin; pkexec prompts (GUI agent or textual on a tty):
echo '{"target":"/dev/sdX","image":null,"tweaks":{"bypass_hardware":false,
      "account_name":null,"account_password":null,"minimize_telemetry":false,
      "disable_auto_bitlocker":false,"region":null}}' \
  | pkexec "$PWD/target/debug/ferrus-helper" dry-run
```

From the GUI: launch `ferrus-gui` in a desktop session with a **polkit agent
running** (the agent shows the password dialog). Point it at the helper via
`FERRUS_HELPER=$PWD/target/debug/ferrus-helper ferrus-gui` for dev. A **fully
clean** test (named action, custom message, argv1 binding) additionally requires
installing the `.policy` to `/usr/share/polkit-1/actions/` and the helper to
`/usr/libexec/ferrus-helper` (root-owned) — this is an explicit install step, not
needed for the basic elevation proof.

## Type-to-confirm (GUI, before ANY elevation)

Before the GUI will spawn the helper, the user must **type the exact target
device path** (e.g. `/dev/sda`) into a field. The action button stays disabled
until the typed text matches the selected device path **exactly**; changing the
selected device clears/invalidates the match. The GUI shows unambiguously what
will happen — device (path/size/model/bus), image, enabled tweaks — and, in 5b-1,
labels it clearly as a **test elevation / dry run (nothing will be written)**.

## Testable vs manual

Unit-tested (no rendering, no real elevation): the type-to-confirm predicate
(blocked until exact match, re-blocked on device change), the contract
(valid Request accepted; malformed/hostile rejected), the helper's re-validation
(a device that fails `SafeTarget` is refused even if "proposed"), and secret
hygiene (password absent from any `Debug`/argv). **Real polkit elevation and the
rendering are validated manually** — not unit-testable; this is stated, not
papered over.

## Deferred to 5b-2

The real destructive run: the helper gains a `write`/`prepare` subcommand that
calls the engine with `dry_run = false`, plus progress streaming back to the GUI
(a line protocol / subscription) for the minutes-long copy. Everything in this
spec (re-validation, secret channel, type-to-confirm, polkit) is designed to carry
over unchanged; only `dry_run` flips and progress is streamed.
