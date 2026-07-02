# ADR-0002: How to obtain UEFI:NTFS

- **Status:** Accepted
- **Date:** 2026-07-03
- **Deciders:** project maintainer

## Context

Ferrus boots NTFS media the Rufus way, using the **UEFI:NTFS** bootloader
(pbatard, GPLv3). We need the binary image on the produced USB's FAT helper
partition. There are two ways to get that image into a build: vendor it in the
repository, or fetch it at build time.

This is a supply-chain, reproducibility, and GPL-compliance decision.

## Options

### A. Vendor the blob under `res/uefi/` (recommended)
- The exact image ships in the repo, pinned by a checked-in SHA-256.
- Builds are reproducible and work offline; no build-time network.
- Requires GPL housekeeping: keep the license and a source reference / written
  offer alongside the binary (`res/uefi/NOTICE`).
- Cost: a binary lives in git; must be refreshed deliberately on upstream
  updates.

### B. Fetch at build time
- Repo stays free of binaries.
- Adds a network dependency and a trust/verification step to every build;
  hurts reproducibility and offline builds.
- Still requires shipping the license/source pointer wherever the binary is
  redistributed, so it does not remove the GPL housekeeping — only moves it.

## Decision

**Option A (vendoring)**, under `res/uefi/`. Robustness, reproducibility, and
offline builds outweigh the cost of a small tracked binary, and it matches how
Rufus ships the signed binaries.

The vendored asset MUST satisfy all of the following:

1. **Pinned version + SHA-256** — the exact upstream release/commit recorded in
   `res/uefi/NOTICE`, and the binary's SHA-256 checked in alongside it.
2. **Secure Boot-signed binaries** — use the signed UEFI:NTFS images (as shipped
   by Rufus). An unsigned image would force the user to disable Secure Boot in
   firmware just to boot the stick, defeating the purpose.
3. **NOTICE** carrying provenance (upstream URL + pinned commit), the GPLv3
   license, and a written offer of source.

## Consequences

- `res/uefi/` holds the image plus `NOTICE`. `.gitignore` already whitelists
  `res/uefi/*.img` against the global `*.img` ignore.
- `boot::install_uefi_ntfs` verifies the asset against the checked-in SHA-256
  before writing.
- Updating UEFI:NTFS is an explicit, reviewed commit that changes the binary,
  the hash, and the `NOTICE` provenance block together.
- **Not done in this session:** the actual binary is added in Phase 3. Until a
  version is chosen, the provenance fields stay as `<TODO>` in `NOTICE`; do not
  guess the asset name, version, or hash from memory.
