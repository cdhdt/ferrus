# ADR-0004: Partitioning via `sfdisk`, not a Rust GPT crate

- **Status:** Accepted
- **Date:** 2026-07-03
- **Deciders:** project maintainer
- **Linked:** SPEC-0003

## Context

Phase 3a must write a GPT to a real device. Two ways: shell out to an external
partitioner (`sfdisk`/`sgdisk`/`parted`) or write the GPT from Rust with a crate
(e.g. `gpt`).

## Options

### A. External tool — `sfdisk` (util-linux) (chosen)
- We already depend on util-linux at runtime (`umount` in Phase 2) and on other
  external tools by design (`mkfs.ntfs`, `mkfs.vfat`); an external partitioner is
  consistent, not a new class of dependency.
- `sfdisk` has a deterministic scripted dump/restore format, accepts explicit
  GPT type GUIDs and partition names, aligns to 1 MiB, and computes the GPT
  metadata (primary/backup headers, CRCs, backup at end) correctly.
- The script is a pure string we can unit-test, and we validated it against real
  `sfdisk` on a throwaway image before writing any code.
- `sgdisk` would also do, but is **not installed** here; `sfdisk` is present.
- Cost: relies on an external binary at runtime (detected up front;
  `Error::MissingTool` if absent).

### B. Rust `gpt` crate
- No external partitioner dependency.
- But we would own GPT correctness (header CRCs, backup header placement,
  protective MBR), a foot-gun on a destructive path; larger dependency surface;
  and it does nothing for the `mkfs` step, which stays external regardless.

## Decision

**Option A — `sfdisk`.** Consistency with the already-accepted external-`mkfs`
approach, correctness we don't have to own on a data-destroying path, and a
scriptable, testable interface. Its presence is checked before any destructive
step.

## Consequences

- Runtime depends on `sfdisk` (util-linux), `mkfs.ntfs` (ntfs-3g/ntfsprogs),
  `mkfs.vfat` (dosfstools), and `partprobe` (parted) — all detected at the start
  of the prepare path, with `Error::MissingTool { name }` on absence.
- The GPT layout lives as pure, tested geometry + a pure sfdisk-script builder;
  only execution is behind the platform backend.
- Revisit only if we later need a partitioner on a platform without util-linux
  (Windows/macOS ports own their native APIs anyway).
