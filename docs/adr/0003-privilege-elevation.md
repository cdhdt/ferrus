# ADR-0003: Privilege elevation on Linux

- **Status:** Accepted
- **Date:** 2026-07-03
- **Deciders:** project maintainer

## Context

Writing to a raw block device, repartitioning, and running `mkfs` all require
elevated privileges. Ferrus needs a story for how the Linux user grants that
access. This interacts directly with the safety model in `crate::device`: the
elevation boundary is also a good place to re-assert the "removable, not
system/critical" guard.

## Options

### A. Run the whole process as root (`sudo ferrus …`)
- Simplest to implement; no IPC, no helper.
- The entire application (including any future GUI and its dependency tree)
  runs as root — a large privileged surface for a tool that mostly does
  read-only enumeration and UI.

### B. polkit + a small privileged helper
- Only a minimal, audited helper performs privileged block-device operations;
  the CLI/GUI run unprivileged and request actions over a defined boundary.
- Integrates with desktop authorization (polkit agent prompts).
- More moving parts: a helper binary, a policy file, and a stable IPC contract.

### C. udev rules granting the invoking user access to specific devices
- No elevation at run time for the whitelisted devices.
- Coarse and system-specific; awkward to ship and reason about safely.

## Decision

**Two-stage.**

- **Now (CLI):** run privileged directly (**Option A**). The CLI checks its
  effective UID at startup and, if not root, exits with a clear message telling
  the user to re-run with elevation, rather than failing deep inside a device
  operation. Simple, no IPC, sufficient for CLI/headless/dev use.
- **Later (Phase 5, with the GUI):** move to **Option B (polkit + a minimal
  privileged helper)** so the GUI runs unprivileged and requests privileged
  actions through a defined boundary. This is deferred to Phase 5 and not
  implemented now.

## Consequences

- Phase 1 adds an EUID check to the CLI entry path with an actionable message.
- The privilege boundary is kept as an explicit seam in `crate::platform`, so
  the Phase 5 helper process can be slotted in without reworking call sites.
- The single safety checkpoint (`device::SafeTarget::acquire`) must run on the
  privileged side in both stages, so elevation can never bypass the guards.
- Revisit the helper design alongside the GUI work (ADR-0001, iced).

**Update (Phase 5b-1):** Option B is now implemented — `ferrus-helper`, elevated
via `pkexec`, re-runs `SafeTarget::acquire` on the root side and never trusts the
GUI's proposed device. See SPEC-0008. Phase 5b-1 restricts the helper to a forced
dry-run; the real write follows in 5b-2.
