# SPEC-0007: graphical front-end — skeleton + UX, dry-run (Phase 5a)

- **Status:** Implemented
- **Module:** `ferrus-gui` (binary crate). Consumes `ferrus-core`; adds no engine
  logic.
- **Linked ADRs:** ADR-0001 (GUI framework = iced).
- **Linked specs:** SPEC-0001 (device / `SafeTarget`), SPEC-0004 (Windows copy),
  SPEC-0006 (`WindowsTweaks`).
- **Framework:** **iced 0.14** (functional `application(boot, update, view)` API).
  File picker: **rfd 0.17** (default `xdg-portal` backend — no GTK build dep).

## Role & scope (5a)

Give Ferrus a graphical front-end with the **complete interface and UX**, wired to
the engine **exclusively through the core's dry-run path**. Phase 5a performs **no
destructive work**: no partitioning, no format, no mount, no write. The action
button runs the full flow with `is_dry_run = true`, so the only thing that reaches
the disk is nothing.

Deferred to **5b** (explicitly out of scope here):
- Real (destructive) execution — the same flow with `dry_run = false`.
- Privilege elevation (the real write needs root/polkit — ADR-0003). 5a runs
  **unprivileged**, like `ferrus list`.
- A mandatory confirmation dialog before a real write (type-to-confirm).
- A live progress **subscription** for the minutes-long real copy (5a's dry-run is
  instantaneous, so it collects log lines synchronously instead).

## Architecture (iced Elm loop)

- **State** (`Ferrus`): the whole UI state — device list + selection, ISO path +
  validated size, the tweak fields (1:1 with `WindowsTweaks`), and the
  action/result state. Deterministic; no hidden globals.
- **Message**: every user action and every async completion (devices loaded, ISO
  picked/validated, dry-run finished).
- **update(&mut State, Message) -> Task<Message>**: pure-ish state transitions;
  side effects (enumeration, file dialog, dry-run) are dispatched as `Task`s so the
  UI thread never blocks.
- **view(&State) -> Element<Message>**: renders the current state; disables controls
  that are not yet valid.

### GUI ↔ core boundary (non-negotiable)

Everything that is a *decision* lives in `ferrus-core` and is merely **called** by
the GUI:

| Concern | Owner | GUI call |
|---|---|---|
| Enumerate safe targets | core | `device::list_writable_candidates(show_large)` |
| Size-hiding of large USB volumes | core | the `include_large` arg (mirrors CLI `--all`) |
| Authorize a target | core | `SafeTarget::acquire(device, &path, dry_run=true)` |
| Validate the ISO | core | `source::RawImage::open(path)` |
| Run the (simulated) flow | core | `partition::prepare_windows(&target, img, tweaks, verify, sink)` |
| Generate autounattend | core | inside `prepare_windows` (GUI never builds XML) |

If a rule is missing from the core, the GUI **signals it**; it never re-implements
it. See "Known gaps".

## `WindowsTweaks` mapping (1:1, no phantom fields)

The GUI fills exactly the struct frozen in Phase 4.x:

| UI control | `WindowsTweaks` field |
|---|---|
| "Bypass Windows 11 hardware checks" | `bypass_hardware: bool` |
| "Create a local account" + name + password | `local_account: Option<LocalAccountSpec{name,password}>` |
| "Minimize telemetry / data collection" | `minimize_telemetry: bool` |
| "Disable automatic BitLocker" | `disable_auto_bitlocker: bool` |
| "Regional preset" + BCP-47 tag | `region: Option<RegionSpec{locale}>` |

- Labels are the **honest** CLI labels — "minimize telemetry", never "disable"
  (the edition-floor nuance from SPEC-0006). The password field is **masked**
  (`text_input.secure(true)`) and is **never logged** — it only flows into the
  `LocalAccountSpec` the core consumes.
- `local_account` is `Some` only when the checkbox is on; an empty password field
  maps to `password: None`. Building the struct is a pure function of state,
  unit-tested.

## Safety UX (a security objective, not decoration)

- **Unambiguous target.** Each candidate is shown with **path + size + model +
  bus** (and `by-id` when known), because in 5b a wrong pick destroys data. The
  action stays **disabled** until a device *and* an ISO are chosen
  (`on_press_maybe(None)`).
- **Dry-run is the only path in 5a.** `SafeTarget::acquire(..., dry_run=true)`; the
  UI states plainly "Dry run — no data will be written".
- **Empty state is honest.** No devices → a clear "no removable targets found"
  with a Refresh button, never a fake/placeholder entry.
- **Large volumes hidden** by default (core heuristic), revealed by a
  "Show large drives" toggle — the exact `--all` behavior, not a GUI reinvention.

## States handled

Empty (no devices), loading (enumerating / running), success (dry-run log shown),
error (enumeration / ISO / dry-run failure, each with a clear message).

## Known gaps (signalled, not faked)

- **Windows-vs-generic ISO detection is not available unprivileged.** The core only
  classifies media by scanning a **mounted** ISO (root-only, part of the real
  flow); dry-run does not mount. So in 5a the GUI cannot auto-decide "Windows vs
  generic". It shows a `MediaKind` (Unknown by default on selection) and the
  gating rule "hide Windows tweaks when Generic" is implemented and tested, but the
  **detection that would set Generic/Windows is deferred**. Proposed core follow-up:
  a non-mounting `source::inspect_iso_kind(path) -> MediaKind` (ISO9660 directory
  scan reusing the pure `detect_windows_install`). Until then, the authoritative
  Windows check remains `prepare_windows` at real-write time (returns
  `NotWindowsMedia`). This is called out rather than approximated in the GUI.

## Testability

Rendering is not unit-tested (iced views cannot be asserted reliably); this is
stated plainly, not papered over. What **is** tested (pure state logic, no runtime):
`tweaks()` maps state → `WindowsTweaks` correctly (including password → `None` when
empty); `can_run()` is false until device+ISO are set; the Generic gating hides the
tweaks; the password never appears in any `Debug`/log of the state. Manual visual
validation covers the actual rendering.
