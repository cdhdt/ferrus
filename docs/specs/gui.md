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

## Preliminary Windows-media detection (`source::inspect_iso_kind`)

A **non-authoritative hint** — `source::inspect_iso_kind(path) -> MediaKind
{ Windows, Generic, Unknown }` — computed **unprivileged and without mounting** at
ISO-selection time, to drive the tweaks gating.

- **Two ordered passes, empirically grounded.** Windows and generic media store
  their tree in *different* layers, so detection tries both, Windows first:
  1. **UDF pass → `Windows`.** Modern Windows ISOs keep their real tree in **UDF**;
     their ISO9660 layer is a **stub** (a real 25H2 ISO: root = only `README.TXT`,
     no Joliet). A pure-ISO9660 scan would false-negative every Windows ISO, so the
     UDF root is read first (read-only, no mount) via `hadris-udf`. Criterion:
     root contains `bootmgr` + `sources` + `efi` → `Windows`. `install.wim` is
     deliberately **not** used (UDF-only, huge — a *copy* concern, not a detection
     one; keying on it would reintroduce the trap).
  2. **ISO9660 pass → `Generic`.** A generic image (e.g. Linux) has a full readable
     **ISO9660** tree (a real Ubuntu 26.04 ISO: 8+ root entries — `boot`, `casper`,
     `efi`, `pool`, `dists`, …; Joliet + Rock Ridge + El-Torito), read via
     `hadris-iso`. Criterion: **≥ 2 real root entries** (ignoring the `.`/`..`
     records, the El-Torito `boot.catalog`, and non-alphanumeric names). The
     threshold cleanly separates a real tree from a Windows ISO9660 stub (1 entry:
     `README.TXT`), so a Windows stub is **never** mistaken for generic — and the
     UDF pass has already claimed real Windows media anyway. Verified end-to-end:
     Ubuntu → `Generic`, Windows 25H2 → `Windows`.
- **`Unknown` is honest, not `Generic`.** Neither pass matched (not UDF-Windows,
  no readable ISO9660 content, or an I/O error) → `Unknown`. We never claim
  `Generic` for something we could not read, nor `Windows` without the markers.
- **Panics are contained.** `hadris-udf`/`hadris-iso` parse untrusted binary and
  are young; a panic on a malformed image must never reach the UI. The workspace
  builds with `panic = unwind` (default), so `inspect_iso_kind` wraps the parsing
  in `std::panic::catch_unwind` and degrades a panic to `Unknown`. (An owned path
  keeps the closure `UnwindSafe`.) A parser bug can therefore, at worst, produce a
  wrong or absent hint — never a crash and never a destructive decision.
- **Hint vs authority.** `inspect_iso_kind` is only a UI hint. The judge is
  `detect_windows_install` on the **mounted** ISO at write time. They can diverge on
  an edge case (structure markers present but no `install.wim`) — acceptable: the
  write arbitrates with `NotWindowsMedia`. The spec does **not** claim they always
  agree.
- **GUI gating.** `Windows` → show tweaks. `Generic` → hide. **`Unknown` →
  permissive: show the tweaks** (do not deny a feature because we could not read the
  image; the write arbitrates) with a discreet "media type undetermined" note. The
  detection runs as an async iced `Task`, so ISO selection never blocks the UI; a
  stale result for a since-replaced image is ignored (path-matched).

## Rendering backend (known issue + workaround)

iced renders on the GPU via **wgpu** by default. On some GPUs/drivers wgpu
**initializes successfully but renders corrupted text** — iced's automatic
fallback only triggers when wgpu fails to *initialize*, not when it renders
badly, so it does not help here. There is **no reliable way to auto-detect
bad rendering**; Ferrus does not attempt a heuristic (that would be unreliable
guesswork).

Mitigation: the CPU renderer **tiny-skia** is pinned as a compiled-in feature
(`iced = { features = ["wgpu", "tiny-skia"] }`) and renders correctly. It is
selected with `ICED_BACKEND=tiny-skia`.

**Why a manual environment variable and not a flag/API:** verified against the
iced 0.14 source — the public `application()` builder and `Settings` expose **no
programmatic backend selection**; the internal `iced_renderer` `backend` slot is
fed only by the `ICED_BACKEND` env var. Setting an env var from code needs
`std::env::set_var`, which is `unsafe` under edition 2024 and therefore barred by
`#![forbid(unsafe_code)]`. Ferrus does not weaken that invariant. Discoverability
is provided instead: `ferrus-gui` prints the active backend + the workaround at
startup, and the README documents it. A clean `--software-render` flag (via a
safe self-re-exec that sets the child's environment, or an external launcher)
remains an option, pending a decision — it is **not** yet implemented.

## Testability

Rendering is not unit-tested (iced views cannot be asserted reliably); this is
stated plainly, not papered over. What **is** tested (pure state logic, no runtime):
`tweaks()` maps state → `WindowsTweaks` correctly (including password → `None` when
empty); `can_run()` is false until device+ISO are set; the Generic gating hides the
tweaks; the password never appears in any `Debug`/log of the state. Manual visual
validation covers the actual rendering.
