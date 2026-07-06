# ADR-0007: Isolating Win32 `unsafe` in a dedicated crate

- Status: **Accepted** (Phase 6.1)
- Date: 2026-07-06
- Related: SPEC-0009 (device enumeration on Windows), the workspace-wide
  `#![forbid(unsafe_code)]` convention.

## Context

`ferrus-core` (and the binaries) carry `#![forbid(unsafe_code)]` — a load-bearing
project invariant: the engine that erases block devices contains no `unsafe`.

The Windows device-enumeration path (SPEC-0009) needs Win32 storage APIs —
`CreateFileW`, `DeviceIoControl` with `IOCTL_STORAGE_QUERY_PROPERTY`,
`IOCTL_DISK_GET_LENGTH_INFO`, `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`,
`GetWindowsDirectoryW`. These are raw FFI: calling them is `unsafe`, and there is
no maintained safe crate that wraps this specific set of IOCTLs (the crates that
exist — `windows` / `windows-sys` — expose them as raw FFI too).

`forbid` is absolute: a local `#[allow(unsafe_code)]` inside a `forbid` crate is a
compile error. So there is no "small local exception" available *within*
`ferrus-core` — using any `unsafe` there means downgrading the crate to
`#![deny(unsafe_code)]`, which weakens the invariant for the whole engine.

## Decision

**Put all Win32 `unsafe` in a new, dedicated crate, `ferrus-win32`, and keep
`ferrus-core` at `#![forbid(unsafe_code)]` untouched.**

- `ferrus-win32` wraps the raw IOCTLs and exposes **safe** functions returning
  plain Rust data (`enumerate_physical_disks() -> Vec<RawDisk>`,
  `system_disk_numbers() -> BTreeSet<u32>`). Every `unsafe` block carries a
  `// SAFETY:` note; handles are RAII (`Drop` closes them).
- It depends on the official Microsoft `windows-sys` (raw, zero-overhead FFI),
  not hand-written `extern` blocks — so struct layouts, IOCTL codes and bus-type
  constants come from Microsoft's own metadata, not from us.
- It is **Windows-only**: `#![cfg(windows)]` makes it empty elsewhere, and
  `ferrus-core` depends on it only under `[target.'cfg(windows)'.dependencies]`.
  On Linux/macOS the crate contributes no code at all.
- `ferrus-core::platform::windows` is safe glue: it calls those functions and maps
  `RawDisk` onto `Device`, reusing the **shared** decision logic
  (`Bus::from_windows_bus_type`, `SafeTarget`) so the safety rules are not
  duplicated per platform.

## Consequences

- **The engine keeps its invariant.** `ferrus-core` is still
  `#![forbid(unsafe_code)]`; the destructive code path contains no `unsafe`.
- The workspace now contains `unsafe`, confined to one small, auditable crate
  whose entire job is the FFI boundary. This is a real change to the project's
  "no unsafe anywhere" posture and is called out for validation, not slipped in.
- The `unsafe` is compile-checked here via cross-compilation
  (`x86_64-pc-windows-gnu`) and clippy on the Windows target, but **cannot be
  executed** in the Linux CI — its behavior against real hardware is validated by
  a human (see SPEC-0009's manual procedure).
- Alternative rejected — *downgrade `ferrus-core` to `deny` + local `allow`*: less
  scaffolding, but it puts `unsafe` inside the engine crate and erodes the
  strongest guarantee the project makes. The dedicated-crate boundary is worth the
  extra crate.
