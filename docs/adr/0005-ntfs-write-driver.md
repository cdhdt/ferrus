# ADR-0005: NTFS write driver — `ntfs3` first, `ntfs-3g` fallback

- **Status:** Accepted
- **Date:** 2026-07-03
- **Deciders:** project maintainer
- **Linked:** SPEC-0004

## Context

Phase 3b mounts the freshly-created NTFS partition **read-write** to copy the
Windows files onto it. Linux offers two NTFS write drivers:

- **`ntfs3`** — in-kernel driver, mainlined in Linux 5.15. No FUSE; native
  performance.
- **`ntfs-3g`** — the historical FUSE (userspace) driver; ubiquitous, works on
  old kernels.

The payload includes a multi-gigabyte `install.wim`/`.esd`, so write throughput
matters.

## Decision

**Try `ntfs3` first, fall back to `ntfs-3g`.** Mount with `-t ntfs3`; if that
fails (driver absent, e.g. kernel < 5.15, or mount rejected), mount with
`-t ntfs-3g`. If neither succeeds, fail with `Error::MissingTool` naming the
NTFS driver.

Rationale:

- ntfs3 is in-kernel: it avoids the FUSE context-switch overhead on a
  multi-GB copy, so it is materially faster for the `install.wim` write.
- ntfs-3g is the safety net for kernels without ntfs3 or setups where the
  ntfs3 mount is unavailable, keeping Ferrus working across environments.
- Both were verified present on the dev host (kernel 7.0.9); the runtime picks
  whichever mounts.

## Consequences

- The Linux mount backend attempts `ntfs3` then `ntfs-3g`; absence of both is a
  clear, actionable error rather than a cryptic mount failure.
- `mkfs.ntfs` (Phase 3a) already comes from ntfs-3g/ntfsprogs, so ntfs-3g’s
  tooling is typically present anyway.
- Revisit if a future need (e.g. specific mount options) makes one driver
  clearly preferable.
