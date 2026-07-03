# ADR-0006: UDF reader for unprivileged Windows-media detection

- **Status:** Accepted
- **Date:** 2026-07-03
- **Deciders:** project maintainer

## Context

The GUI wants a **preliminary hint** at ISO-selection time — is this Windows
install media? — to gate the Windows tweaks (SPEC-0007). It must run
**unprivileged, without mounting**.

Empirical finding on a real `Win11_25H2_French_x64_v2.iso` (verified with
`isoinfo`): modern Windows ISOs are **UDF Bridge** discs whose **ISO9660 layer is
a stub** — its root holds only `README.TXT`. There is **no Joliet**. Every real
structure marker (`bootmgr`, `efi/`, `sources/`, and the UDF-only
`sources/install.wim`) lives in the **UDF** layer. So a pure-ISO9660 scan — the
first-guess approach — would **false-negative every real Windows ISO**. Reliable
detection therefore requires reading **UDF**.

Constraints: read-only, unprivileged, no mount, `#![forbid(unsafe_code)]`
maintained in `ferrus-core` (dependencies may use `unsafe` internally; our code
may not).

## Options

- **Pure-ISO9660 crate** (`cdfs`, `iso9660-rs`) — rejected: markers are UDF-only,
  so this cannot see them.
- **Shell out to `7z`/`isoinfo`** — rejected: adds a runtime tool dependency and
  fragile output parsing; not a library-clean solution.
- **Native `libudfread` binding** — a C dependency + build/link surface; heavier
  than warranted for a non-authoritative hint.
- **`hadris-udf`** (pure-Rust UDF filesystem, MIT) — reads an existing UDF image
  via `Read + Seek` (seeks descriptors; does not load the 7.9 GB image), lists the
  root directory. **Verified working** on the real 25H2 ISO: it returned
  `bootmgr, efi, sources, setup.exe, …`.

## Decision

Use **`hadris-udf`** to read the UDF root directory and classify by structure
markers (`bootmgr` + `sources` + `efi`).

Rationale, and why its immaturity is acceptable here:

- It is the only pure-Rust, read-capable UDF library, it is MIT (GPL-compatible),
  and it demonstrably reads real Windows media.
- It is young (low download count). **But its blast radius is bounded to a
  non-authoritative GUI hint.** A bug in it can only produce a wrong *hint*, which
  is further contained: read failure → `MediaKind::Unknown` (permissive, tweaks
  still shown), and the **authoritative** decision remains
  `detect_windows_install` on the mounted ISO at write time
  (`Error::NotWindowsMedia`). A UDF-reader defect **cannot** cause a wrong
  destructive action.

## Consequences

- `ferrus-core` gains a `hadris-udf` dependency, used only by
  `source::inspect_iso_kind`.
- `install.wim` is deliberately **not** part of the detection criterion (it is
  UDF-only and huge — a copy concern, not a detection one), avoiding the
  false-negative trap.
- If `hadris-udf` proves unreliable on some ISOs, the fallback is `Unknown`
  (never a wrong write), and the reader can be swapped without touching the
  detection criterion or the write path.
