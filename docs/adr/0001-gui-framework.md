# ADR-0001: GUI framework

- **Status:** Accepted
- **Date:** 2026-07-03
- **Deciders:** project maintainer

## Context

Ferrus needs a graphical front-end (`ferrus-gui`) in addition to the CLI
(Phase 5 on the roadmap). It must be cross-platform (Linux → Windows → macOS),
work well for a small operational UI (device picker, ISO chooser, a set of
tweak checkboxes, a progress bar), and integrate cleanly with a Rust core that
runs long, cancellable operations off the UI thread.

The choice is **not locked**. This ADR records the candidates and trade-offs so
it can be decided deliberately rather than by whatever gets imported first. Until
this ADR is accepted, `ferrus-gui` carries no GUI dependency.

## Options

### iced
- Elm-like, retained/reactive; pure-Rust; good async/command model that fits a
  core doing background work.
- Cross-platform via wgpu. Theming is decent; native look is not a goal.
- Larger dependency graph; API still evolves between releases.

### slint
- Dedicated `.slint` markup + Rust logic; polished widgets and tooling.
- Small runtime, embedded-friendly, strong cross-platform story.
- License nuance: royalty-free/GPL options exist. **Must confirm the chosen
  license is compatible with GPL-3.0-or-later** before adopting.

### egui
- Immediate-mode; simplest to prototype and to reason about; very small API
  surface.
- Cross-platform via `eframe`. Look is functional rather than native.
- Immediate-mode redraws and state handling can get awkward for a larger,
  form-heavy UI.

## Decision

**egui** (via `eframe`).

Rationale:

- Ferrus is a small desktop utility. **Immediate mode** maps naturally onto the
  write flow: a background worker performs the burn while the UI polls and
  redraws progress each frame — no retained-widget state to keep in sync with a
  long-running operation.
- Widest adoption of the candidates, so the largest pool of examples and
  contributors.
- **MIT/Apache-2.0** licensing, cleanly compatible with GPL-3.0-or-later and,
  crucially, non-viral — it does not constrain a future relicensing.

Rejected:

- **slint** — its licensing requires a commercial license for any closed-source
  use, which would foreclose a future relicensing of Ferrus. Ruled out on that
  basis despite its polish.

Fallback:

- **iced** — kept as the alternative to reach for if a need for a more polished,
  retained-mode UX emerges later.

## Consequences

- `ferrus-gui` depends on `eframe`/`egui`; GUI-specific code stays inside that
  crate, and all engine logic remains in `ferrus-core` so the front-end can be
  swapped (e.g. for iced) with limited blast radius.
- Immediate-mode redraw means the burn worker must run off the UI thread and
  communicate progress over a channel; the UI reads it each frame. This aligns
  with the `progress::ProgressSink` design in `ferrus-core`.
