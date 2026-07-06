# SPEC-0009: `device` on Windows — enumeration and safe target selection

- Status: implemented (Phase 6.1) — code compiles via cross-compilation; **real
  hardware validation is human** (see the manual procedure below).
- Related: [SPEC-0001](device.md) (the Linux original — this is its Windows
  counterpart, same guarantees, different native API), ADR-0007 (why the Win32
  `unsafe` lives in a dedicated crate).

## Role

Give the Windows port the **same** device enumeration + safe target selection as
Linux: list physical disks, classify each by transport, refuse the system disk,
and produce `Device`s that flow through the **unchanged, shared** `SafeTarget`
checkpoint. No partitioning and no writing — those are later Phase 6 steps.

The safety decision is **not** re-implemented for Windows. The Windows backend
only produces accurate `Device`s (correct transport `bus`, correct
`is_system_or_critical`); the eligibility rules and `SafeTarget::acquire` in
`crate::device` are the single, shared, already-tested authority (SPEC-0001).

## Invariants

Identical to SPEC-0001, restated for Windows:

- A raw device is **never** writable; only `SafeTarget::acquire` produces a
  writable target, and only when transport is removable **and** the disk is not
  system/critical **and** the confirmed path matches exactly.
- Eligibility is decided on the **transport bus**, never on a removable flag.
- The system/boot/Windows disk is refused.

## Behavior

### Enumeration

Physical disks are addressed as `\\.\PhysicalDriveN`. The backend scans
`N = 0..64`, opening each with `CreateFileW` using **zero desired access** —
enough for the query IOCTLs and requiring **no elevation**, so `ferrus list`
works as a normal user. Indices that fail to open (gaps) are skipped; the 64
ceiling is a generous, documented bound.

For each disk that opens:

- **Transport + model + removable flag**: `IOCTL_STORAGE_QUERY_PROPERTY`
  (`StorageDeviceProperty`, `PropertyStandardQuery`) → `STORAGE_DEVICE_DESCRIPTOR`.
  Its `BusType` (a `STORAGE_BUS_TYPE`), `RemovableMedia`, and `ProductIdOffset`
  (offset of an embedded ANSI model string) are read.
- **Size**: `IOCTL_DISK_GET_LENGTH_INFO` → `GET_LENGTH_INFORMATION.Length`. A disk
  reporting size 0 (no media) is skipped.

### Transport classification (the reliable criterion)

`Bus::from_windows_bus_type` maps the descriptor's `BusType` to Ferrus's `Bus`.
Values are the `winioctl.h` `STORAGE_BUS_TYPE` constants (verified against
Microsoft Learn, *STORAGE_BUS_TYPE*; the enum is sequential from 0):

| `STORAGE_BUS_TYPE`                         | value | `Bus`     | removable? |
| ----------------------------------------- | ----- | --------- | ---------- |
| `BusTypeUsb`                              | 7     | `Usb`     | **yes**    |
| `BusTypeSd` / `BusTypeMmc`                | 12/13 | `Mmc`     | **yes**    |
| `BusTypeAtapi` / `BusTypeAta` / `BusTypeSata` | 2/3/11 | `Sata` | no         |
| `BusTypeNvme`                            | 17    | `Nvme`    | no         |
| `BusTypeScsi`/`RAID`/`iScsi`/`Sas`        | 1/8/9/10 | `Scsi` | no        |
| everything else — incl. **virtual** (`BusTypeVirtual`=14, `FileBackedVirtual`=15, `Spaces`=16), SCM/UFS/Nvmeof, unknown | — | `Unknown` | no |

Only USB and SD/MMC are removable transports, hence the only eligible targets.
Crucially, **virtual disks** (VHD, Storage Spaces) map to `Unknown` → never
eligible. This is the Windows analogue of Linux excluding `dm/loop/md/zram`.

### System / critical-disk detection

`is_system_or_critical` marks the physical disk(s) backing the **Windows
installation volume**:

- `GetWindowsDirectoryW` → e.g. `C:\Windows` → its drive letter (`C`).
- Open `\\.\C:` (zero access) and issue `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` →
  `VOLUME_DISK_EXTENTS` → each `DISK_EXTENT.DiskNumber`. Those `PhysicalDriveN`
  are system/critical (a spanned/mirrored volume can name several).

This is **defense in depth**, not the primary guard. The transport gate already
refuses every fixed disk (SATA/NVMe/SCSI), so an internal system or EFI disk is
refused *by transport* regardless. The extent resolution additionally covers the
rare case of Windows installed on a **USB** disk (which would otherwise pass the
transport gate). If it cannot be resolved, enumeration fails closed rather than
listing disks with no system guard.

Accepted scope: only the disk(s) backing `%SystemDrive%` are marked, not, say, a
separate data volume the user considers precious — exactly as SPEC-0001 marks the
OS/boot backing store, not arbitrary data disks.

### `SafeTarget::acquire` contract

Unchanged from SPEC-0001 — the same shared code, fed Windows `Device`s. The live
re-check calls `Backend::is_system_or_critical(path)`, which on Windows parses the
`\\.\PhysicalDriveN` number and re-resolves the system extents; a path that is not
a well-formed physical-drive path fails **closed** (treated as critical).

### User-facing metadata

`path` = `\\.\PhysicalDriveN`; `size_bytes`, `model` (product id), `bus`
(transport). `removable` is carried for display only and is never a gate.

## The `unsafe` question (see ADR-0007)

Win32 storage access is raw FFI and cannot be done in safe Rust. Rather than
weaken `ferrus-core`'s `#![forbid(unsafe_code)]`, **all** `unsafe` is isolated in
the dedicated `ferrus-win32` crate (safe API out, `windows-sys` FFI in, RAII
handles, `// SAFETY:` on every block). `ferrus-core` stays `forbid`. This is a
deliberate, flagged change to the project's no-`unsafe` posture.

## Verification status

- `ferrus-win32` and `ferrus-core` **cross-compile** for `x86_64-pc-windows-gnu`
  and pass `clippy -D warnings` on that target.
- The **pure** transport mapping (`Bus::from_windows_bus_type`) is unit-tested on
  any host, including the refusal cases (virtual/unknown buses → not removable).
- The Win32 I/O itself is **not** exercisable in the Linux CI. It is validated by
  a human on real Windows (below). This split is honest: compiled, not run.

## Manual test procedure (human, on real Windows)

Prerequisites: a Windows 10/11 machine or VM, the Rust toolchain (or a
cross-built `ferrus.exe`), and a USB stick.

1. Build/obtain `ferrus.exe` and run **without** administrator rights:
   `ferrus list`.
2. **With no USB stick attached**, expect: the internal system disk
   (`\\.\PhysicalDrive0`, SATA/NVMe) is **not** listed as an eligible target (it
   is a fixed transport and/or the system disk). `ferrus list --all` may show it
   for information, still flagged system/non-eligible.
3. **Attach a USB stick.** Re-run `ferrus list`: it should appear as
   `\\.\PhysicalDriveN` with `usb` bus, its size and model.
4. Confirm the guard: attempt to target the internal disk explicitly — it must be
   **refused** (`refused for safety`), both because its transport is not removable
   and because it backs Windows.
5. Sanity-check the mapping against Windows' own view: `wmic diskdrive get
   Index,Model,Size,InterfaceType` (or `Get-Disk` in PowerShell) — the USB stick's
   index/size/model should match what `ferrus list` shows.

Report what appears. No partitioning or writing is possible in this phase, so the
procedure is read-only and safe to run against a real system.

## Out of scope (later Phase 6 steps)

Partitioning, formatting, ISO copy, bootloader install, and the privileged write
path on Windows. This spec is enumeration + safe selection only.
