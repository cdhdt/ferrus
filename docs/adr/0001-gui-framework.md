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

**iced.**

Rationale:

- The product goal is a **modern, good-looking** UI. iced gives direct control
  over rendering and theming (custom widgets, styling) via its wgpu renderer,
  without embedding a WebView — so we can pursue a polished look without the
  weight and attack surface of a browser runtime.
- Its Elm-like, retained/reactive model with an explicit command/subscription
  system fits a core that runs long, cancellable operations off the UI thread
  and streams progress back.
- Pure-Rust, MIT-licensed — cleanly compatible with GPL-3.0-or-later and
  non-viral, so it does not constrain a future relicensing.

Fallback:

- **egui** (via `eframe`) — kept as the alternative to reach for if an
  ultra-fast MVP becomes the priority again; immediate mode is quicker to
  prototype but gives less control over a refined visual design.

Rejected:

- **slint** — its licensing requires a commercial license for any closed-source
  use, which would foreclose a future relicensing of Ferrus. Ruled out on that
  basis despite its polish.

**To be reconfirmed at the start of Phase 5.** No GUI implementation happens
before then; `ferrus-gui` stays a minimal window shell until Phase 5.

## Consequences

- `ferrus-gui` will depend on `iced`; GUI-specific code stays inside that crate,
  and all engine logic remains in `ferrus-core` so the front-end can still be
  swapped (e.g. for egui) with limited blast radius.
- iced's command/subscription model means the burn worker runs off the UI thread
  and reports progress over a channel/subscription. This aligns with the
  `progress::ProgressSink` design in `ferrus-core`.
- The Phase 0 shell used `eframe` only as a throwaway placeholder. **Phase 5a
  executed the switch to iced** (0.14) as decided here; the eframe dependency is
  gone. Reconfirmed at the start of Phase 5: iced stands.
