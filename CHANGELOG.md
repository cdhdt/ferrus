# Changelog

All notable changes to Ferrus are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project aims to adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Proof levels: **[real]** = exercised on real hardware; **[unit]** = covered by
unit tests only.

## [Unreleased]

### Added

- **Windows GPT partitioning + NTFS format (Phase 6.2a, SPEC-00010).** The first
  real *write* on Windows and the counterpart of Phase 3a: partition a `SafeTarget`
  USB disk with the SPEC-0003 GPT layout (P1 NTFS basic-data, P2 raw FAT helper) and
  format P1. The GPT table is written with **native IOCTLs**
  (`CREATE_DISK` → `SET_DRIVE_LAYOUT_EX` → `UPDATE_PROPERTIES`, after
  lock/dismounting the disk's volumes) rather than scripting `diskpart`
  (ADR-0008); NTFS formatting shells out to PowerShell `Format-Volume` (the `mkfs`
  analogue). A new **ESP guard** closes the SPEC-0009 6.1.1 TODO: before any
  destructive step the target's current layout is read and a disk carrying an EFI
  system / Microsoft reserved / recovery partition is refused. Writing requires
  **Administrator** (fail closed; GUI UAC is a later phase). All Win32 `unsafe` is
  in `ferrus-win32`; `ferrus-core` stays `#![forbid(unsafe_code)]`. Cross-compiles +
  clippy-clean on `x86_64-pc-windows-gnu`; the orchestration, ESP guard, format
  command, and GUID parsing are unit-tested on any host. The IOCTL write is
  compiled, **not executed** in CI — validated by hand on a throwaway disk
  (SPEC-00010). No ISO copy or bootloader yet.

- **Windows device enumeration + safe target selection (Phase 6.1, SPEC-0009).**
  The Windows counterpart of Phase 1: list physical disks (`\\.\PhysicalDriveN`),
  classify each by **transport** (USB/SD/MMC removable vs fixed) via
  `STORAGE_BUS_TYPE`, refuse the disk backing the Windows volume (resolved through
  `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`), and feed the **unchanged, shared**
  `SafeTarget` checkpoint — same safety guarantees as Linux, native API. All Win32
  `unsafe` is isolated in a new **`ferrus-win32`** crate so `ferrus-core` keeps
  `#![forbid(unsafe_code)]` (ADR-0007). Enumeration needs **no elevation** (query
  IOCTLs opened with zero access). Cross-compiles + clippy-clean on
  `x86_64-pc-windows-gnu`; the pure transport mapping is unit-tested on any host.
  Real-hardware validation is a documented manual procedure (no writing in this
  phase). No partitioning/writing yet.
- **Hardened `ferrus-win32` before it grows a write path (Phase 6.1.1).** Still
  read-only, observable behavior unchanged. The system-disk guard now handles a
  **spanned/RAID system volume**: `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` returns
  `ERROR_MORE_DATA` for multi-disk volumes (per MS docs), so the buffer is grown
  and retried and **all** `DiskNumber`s are collected (not just the first). The
  IOCTL-buffer parsing was split out of the FFI and unit-tested on any host
  (multi-extent, truncation, model-string bounds); the model read is bounded to
  the bytes actually returned; `GetWindowsDirectoryW` no longer risks an
  out-of-range slice; layout offsets are `const`-asserted against the `windows-sys`
  ABI on Windows. The ESP/"system volume" on a separate disk is documented as a
  bounded, deferred case (SPEC-0009). `ferrus-core` stays `#![forbid(unsafe_code)]`.

- **Install target + hardened elevation (packaging).** `make install` /
  `make uninstall` set Ferrus up the way an end user runs it: `ferrus` + `ferrus-gui`
  in `/usr/bin`, the root-owned helper in `/usr/libexec` (matching the polkit
  `exec.path`), the two named polkit actions in `/usr/share/polkit-1/actions`, and
  two desktop entries (normal + a software-rendering variant that forces
  `ICED_BACKEND=tiny-skia`). `resolve_helper_path` now prefers the installed
  `/usr/libexec/ferrus-helper` over `$FERRUS_HELPER`, so in production the GUI
  always elevates through the **named** polkit action and cannot be redirected by
  the environment; dev (helper not installed) keeps using `$FERRUS_HELPER`
  (SPEC-0008). Not distro packaging (no .deb/Flatpak).
- **GUI (Phase 5) — write path proven `[real]`.** The destructive path the GUI's
  *Write* button invokes (unprivileged GUI → type-to-confirm → **named** polkit
  action → root helper re-validating → live NDJSON progress) was exercised via the
  helper's `write` verb, byte-identical to what the button spawns: a real 8.5 GB
  Windows 11 25H2 write completed, streamed live progress, and the produced stick
  booted Windows Setup in QEMU. The type-to-confirm gate is unit-tested. **Not yet
  recorded:** the fully on-screen click-through (clicking *Write* in the window,
  seeing the named polkit dialog and the animated bar live).
- **Real write from the GUI + live progress streaming (Phase 5b-2).** The GUI can
  now actually write a device (still **never** as root). A new `write` helper
  subcommand runs the engine destructively (`dry_run = false`), alongside the
  existing `dry-run`. Destructiveness is chosen by the **subcommand**, never by
  request data — there is no `dry_run` field; each verb passes a literal, and
  `write` is a separate `auth_admin` polkit action. The helper streams **NDJSON**
  progress events and the GUI shows a **live progress bar** (consumed as an async
  iced `Task` stream — the UI never blocks). Root-side re-validation
  (`SafeTarget::acquire`) and type-to-confirm are unchanged and apply to the write.
  polkit ships two actions (`…ferrus.dryrun` / `…ferrus.write`), the write one
  labelled as erasing all data (SPEC-0008). No `unsafe`; the core destructive
  pipeline (validated on real hardware in Phases 2–4) is unmodified.

### Changed

- **Helper bounds its stdin read.** The privileged helper reads the JSON request
  with a 64 KiB cap (`Read::take`) instead of an unbounded `read_to_end`; a
  legitimate request is < 1 KiB, and anything larger is rejected cleanly. Defense
  in depth on a root binary (SPEC-0008).
- **Disk-parser panics are contained.** `inspect_iso_kind` wraps the
  UDF/ISO9660 parsing (hadris-udf/hadris-iso, young binary parsers) in
  `catch_unwind`, so a panic on a malformed image degrades to `MediaKind::Unknown`
  instead of crashing the GUI. The workspace builds with `panic = unwind`, which
  makes this effective. No destructive path is involved.
- **`inspect_iso_kind` now recognizes generic (e.g. Linux) media as `Generic`.**
  Previously a Linux ISO fell through to `Unknown` (tweaks shown regardless). Added
  an ISO9660 pass (via `hadris-iso`, MIT) after the UDF/Windows pass: a readable
  ISO9660 tree with ≥ 2 real root entries and no Windows markers → `Generic`, so
  the GUI hides the Windows tweaks. Verified end-to-end on real media: Ubuntu 26.04
  → `Generic`, Windows 11 25H2 → `Windows`; unreadable → `Unknown` (never a false
  `Generic`/`Windows`). Still a non-authoritative hint (SPEC-0007).

### Added

- **Privileged helper + polkit elevation + type-to-confirm** (Phase 5b-1;
  SPEC-0008). New `ferrus-helper` crate — the only privileged component — a thin,
  re-validating shell over `ferrus-core`, elevated via `pkexec`. The GUI (always
  unprivileged) sends a typed JSON request on **stdin** (so a password never
  touches argv/env), and the helper **re-validates everything as root**
  (re-enumerate + `SafeTarget::acquire` — it never trusts the GUI's proposed
  device), asserts it is actually root, and runs a **forced dry-run** (no write is
  possible in this sub-phase). Before any elevation the GUI requires a
  **type-to-confirm**: the exact target path must be typed, or the action stays
  disabled (re-blocked when the device changes). Ships a polkit `.policy`
  (`res/polkit/`, format verified against the local polkit 126). Real write +
  progress streaming are deferred to 5b-2. No `unsafe`.

- **Preliminary Windows-media detection** (`source::inspect_iso_kind`, wired into
  the GUI). At ISO selection the GUI now guesses Windows vs generic media
  **unprivileged and without mounting**, to gate the Windows tweaks. Established
  empirically that modern Windows ISOs keep their tree in **UDF** (the ISO9660
  layer is a stub), so a pure-ISO9660 scan would miss them; detection reads the UDF
  root via `hadris-udf` (MIT; ADR-0006) and keys on structure markers
  (`bootmgr` + `sources` + `efi`), never on `install.wim`. It is a **hint** —
  `detect_windows_install` on the mounted ISO stays authoritative at write time.
  `Unknown` (unreadable) is permissive (tweaks shown, write arbitrates). No
  `unsafe`; read-only.

### Changed

- **GUI rendering robustness (Phase 5a.1).** Pin both iced renderers explicitly
  (`wgpu` + `tiny-skia`) so the CPU/software fallback is always compiled. On GPUs
  where wgpu renders corrupted text, run `ICED_BACKEND=tiny-skia ferrus-gui` for
  pixel-correct CPU rendering. `ferrus-gui` now prints the active backend and this
  workaround at startup; the README has a Troubleshooting section. No automatic
  detection (bad rendering can't be reliably detected) and no `unsafe` introduced
  — iced 0.14 has no programmatic backend switch, so the lever stays the
  `ICED_BACKEND` env var. A `--software-render` flag is deferred pending a design
  decision.

### Added

- **Phase 4.x — additional Windows tweaks** (extends the `autounattend.xml`
  generator; SPEC-0006). Unit-tested, and **validated on a real Windows 11 25H2
  install** in a TPM 2.0 + Secure Boot VM: with the TPM present, automatic
  BitLocker device encryption did **not** occur (`disable_auto_bitlocker`
  **[real]**); the OOBE was silent — no language/keyboard prompt
  (`region=fr-FR` **[real]**); and the stick booted **under Secure Boot** via the
  signed UEFI:NTFS loader (bonus **[real]**). Telemetry: the OOBE privacy screens
  were pre-answered **[real]**; the effective diagnostic level is the edition
  floor (Required on Home/Pro — **[unit]/by-design**, not "off"):
  - `minimize_telemetry`: reduce Windows diagnostic data to the edition minimum
    (`AllowTelemetry=0`; floors to Required/Basic on Home/Pro — **not** fully off
    — Security/Off only on Enterprise/Education/Server) plus disable advertising
    ID, location, Find My Device and feedback notifications (machine-wide HKLM
    policies, specialize pass). Per-user (HKCU) toggles — tailored experiences,
    inking/typing — intentionally deferred (would be a SYSTEM-hive no-op).
  - `disable_auto_bitlocker`: set `PreventDeviceEncryption=1` in specialize,
    before Windows 11 24H2+ auto-encrypts partitions during setup.
  - `region` (optional): preset UI language / system / user / input locale from a
    BCP-47 tag (e.g. `fr-FR`) via `Microsoft-Windows-International-Core`.
  - New CLI flags: `--minimize-telemetry`, `--disable-auto-bitlocker`,
    `--region <TAG>`. No flag → tweak absent from the XML (Phase 3 behavior when
    no tweak at all is selected stays unchanged).

## [0.4.0] — 2026-07-03

Core milestone: Ferrus produces a bootable Windows 11 install stick with
Rufus-style tweaks, validated end-to-end on real hardware **through Windows 11
25H2** (boot + hardware bypass + local account without a Microsoft account).

### Added

- **Phase 0 — scaffolding & architecture.** Cargo workspace (edition 2024,
  resolver 3), three crates (`ferrus-core`, `ferrus-cli`, `ferrus-gui` shell),
  error types, safety-critical `device` API design, ADRs. **[unit]**
- **Phase 1 — device enumeration + safe target selection (Linux).** Removable
  targets by *transport* (USB/MMC), not the unreliable `removable` bit; system/
  critical-disk detection walking LUKS/LVM/md `slaves`; `SafeTarget::acquire`
  single checkpoint. SPEC-0001. **[unit]** + enumeration/refusal run on the real
  host.
- **Phase 2 — raw image write.** `dd`-style whole-device write behind
  `&SafeTarget`: EUID gate, unmount-first, `O_EXCL` exclusive open, 4 MiB block
  loop with progress, mandatory `fsync`, optional read-back `--verify`.
  SPEC-0002. **[real]**: Alpine ISO written to a USB device and booted in QEMU.
- **Phase 3a — partition + format.** GPT with a large NTFS partition + a 1 MiB
  FAT helper (both Microsoft basic data — verified against Rufus, P2 deliberately
  not ESP); partitioning via `sfdisk`; kernel re-read + node-settle wait;
  `mkfs.ntfs`. SPEC-0003, ADR-0004. **[real]**.
- **Phase 3b — Windows ISO file copy.** Mount ISO (UDF), validate Windows markers,
  space guard, mount NTFS (ntfs3→ntfs-3g), recursive streaming copy, sync, RAII
  unmount even on failure. SPEC-0004, ADR-0005. **[real]**: real Windows 11 25H2
  ISO, copy byte-identical to the source (1064 files, sizes match, 7.1 GB
  `install.wim`).
- **Phase 3c — UEFI:NTFS bootloader.** Vendored, **Secure Boot-signed**
  `uefi-ntfs.img` (Rufus v4.15, pinned SHA-256, verified signed) embedded via
  `include_bytes!`, integrity-checked, written raw to P2. SPEC-0005, ADR-0002.
  **[real]**: stick boots to Windows Setup via the signed loader.
- **Phase 4 — autounattend.xml tweaks (the differentiator).** `WindowsTweaks`
  model + per-build `BuildProfile` (win11-25h2). Hardware bypass (LabConfig in
  windowsPE), local account (LocalAccounts + OOBE + BypassNRO complement),
  password obfuscation, secret-safe logging. Every value verified against current
  sources. SPEC-0006. **[unit]** generator + **[real]**: Windows 11 25H2 install
  — no TPM wall, local account without MSA.

### Notes / known gaps

- Linux only; UEFI/GPT only (no legacy BIOS).
- No GUI yet (planned: iced — ADR-0001).
- Telemetry / BitLocker / regional tweaks not implemented (phase 4.x).
- `cargo clippy` / `cargo fmt` not yet run project-wide (tracked separately).
