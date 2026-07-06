# ADR-0008: Writing the GPT partition table on Windows via native IOCTLs

- Status: **Accepted** (Phase 6.2a)
- Date: 2026-07-06
- Related: ADR-0004 (`sfdisk` on Linux), ADR-0007 (Win32 `unsafe` isolation),
  SPEC-00010 (Windows partition + format), SPEC-0003 (the shared layout).

## Context

Phase 6.2a is the first real *write* on Windows: it must lay down the SPEC-0003
GPT table (P1 NTFS basic-data, P2 FAT helper). On Linux this is `sfdisk` (ADR-0004).
Windows offers two routes:

1. **Shell out to `diskpart`** with a generated script (`select disk N` / `clean`
   / `convert gpt` / `create partition …`).
2. **Native `IOCTL_DISK_SET_DRIVE_LAYOUT_EX`** (with `IOCTL_DISK_CREATE_DISK` +
   `IOCTL_DISK_UPDATE_PROPERTIES`) in `ferrus-win32`.

## Decision

**Use the native IOCTLs.**

`diskpart` is brittle to script for a *destructive, safety-critical* operation: its
output is localized and unstructured, it has no per-step error codes (only a final
exit status), and it cannot be driven transactionally. Parsing localized text to
decide whether a disk wipe succeeded is exactly the kind of guesswork this project
avoids.

The IOCTLs give **precise** control over offsets, type GUIDs and disk/partition
GUIDs (matching SPEC-0003 exactly), and **real** Win32 error codes at each step
(`CreateFileW`, lock/dismount, create, set-layout, update). The `unsafe` this adds
is confined to `ferrus-win32` (ADR-0007) — `ferrus-core` stays
`#![forbid(unsafe_code)]`. This mirrors the project's Linux stance: precise,
low-level, auditable control of the dangerous step.

**Filesystem formatting is the exception** (decided separately, see SPEC-00010): it
shells out to PowerShell `Format-Volume`, the direct analogue of Linux's `mkfs`
shell-out — no benefit to reimplementing NTFS/FAT formatting natively, and no extra
`unsafe`.

## Consequences

- Precise, SPEC-0003-faithful GPT geometry and structured error handling.
- More `unsafe` in `ferrus-win32`: `CREATE_DISK`, a variable-length
  `DRIVE_LAYOUT_INFORMATION_EX` (two `PARTITION_INFORMATION_EX` entries),
  `GET_DRIVE_LAYOUT_EX` (variable buffer), volume lock/dismount, and GUID
  generation (`UuidCreate`). All cross-compiled and clippy-checked; **not**
  executed in CI (validated by hand on a throwaway disk — SPEC-00010).
- We own GPT geometry details `sfdisk` handled for free: the usable range and
  clamping the last partition to the last usable LBA (512-byte sector assumed).
  This is the piece most in need of real-hardware validation.
- Alternative rejected — `diskpart`: less `unsafe`, but unreliable to script and
  opaque on failure for a data-destroying operation.
