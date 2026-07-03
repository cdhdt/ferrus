# Changelog

All notable changes to Ferrus are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project aims to adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Proof levels: **[real]** = exercised on real hardware; **[unit]** = covered by
unit tests only.

## [Unreleased]

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
