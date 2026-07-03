# Ferrus

Cross-platform bootable USB creator in Rust. Spiritual successor to Rufus
(not a git fork — clean rewrite). Writes any ISO, plus Rufus-style Windows
install tweaks.

## Locked decisions

These are settled. Document them, do not relitigate them without an explicit
request.

- License: **GPL-3.0-or-later** (Ferrus embeds UEFI:NTFS, which is GPLv3, and
  follows in the Rufus lineage — GPLv3 is the path of least resistance here).
- NTFS boot via **UEFI:NTFS**, not the FAT32/split-WIM strategy. Modern Windows
  ISOs ship an `install.wim` larger than 4 GB, so FAT32 is ruled out for the
  main partition.
- **Rust edition 2024**, Cargo workspace, resolver 3.
- **Platform order: Linux → Windows → macOS.** Design cross-platform from day
  one (clean abstractions), even while only Linux is implemented.
- **All repo content in English** (README, this file, code comments, commit
  messages). International open-source project.
- **`Cargo.lock` is committed** (Ferrus is an application, not a library).

## Working method

This process applies to Phase 1 and every phase after it.

- **One phase at a time.** A phase is done only when `cargo build` and
  `cargo test` are green. Then: write a short summary and STOP. Never start the
  next phase without an explicit instruction.
- **Spec-first for domain-heavy modules** (`device`, `partition`, `boot`,
  `windows`): the spec under `docs/specs/` is written BEFORE the code and is the
  contract the code must satisfy. A spec captures the WHY and the BEHAVIOR
  (invariants, refusal cases, pitfalls), not a line-by-line paraphrase of the
  code — that only rots and lies.
- **Safety invariants are encoded in the type system where possible, and are
  ALWAYS covered by tests** — including the refusal cases, not just the happy
  path.
- **Never hardcode system values or mechanisms from memory** (sysfs layouts,
  Windows registry keys, `autounattend.xml` schema, dependency versions).
  Verify against a current source; when unsure, leave an explicit `TODO` and
  flag it in the end-of-phase report rather than inventing something.
- **Conventions are non-negotiable** (see "Code conventions"): `thiserror` in
  the library, `anyhow` in the binaries, no `unwrap`/`expect` in the library,
  `clippy --all-targets -- -D warnings`, `fmt`, doc-comments on every public
  item, platform abstraction via traits + `cfg`.
- **Atomic, descriptive commits.**

## How the Windows "magic" works

- Tweaks are **file drops, NOT binary patching**:
  - `autounattend.xml` at the USB root — Windows Setup reads it automatically
    (Microsoft-account bypass, local account, telemetry off, OOBE skip,
    regional settings).
  - **LabConfig** registry keys (`BypassTPMCheck`, `BypassSecureBootCheck`,
    `BypassRAMCheck`, `BypassStorageCheck`, `BypassCPUCheck`) for hardware
    bypass.
- The **`autounattend.xml` generator is the core differentiator** of the
  project — it is parameterized by the options the user ticks.
- Why NTFS: `install.wim` > 4 GB does not fit on FAT32.
- **UEFI:NTFS** (pbatard) is a small GPLv3 EFI bootloader written to a FAT
  helper partition at the end of the disk; it lets a UEFI firmware boot the
  main NTFS partition. Secure Boot-signed binaries exist (shipped by Rufus).

## Architecture

Cargo workspace, three crates:

```
ferrus/
├── Cargo.toml            # workspace (resolver 3, edition 2024)
├── crates/
│   ├── ferrus-core/      # lib: the engine, platform-abstracted
│   ├── ferrus-cli/       # bin: clap CLI, real --dry-run from day one
│   └── ferrus-gui/       # bin: minimal stub, framework TBD (see ADR-0001)
├── res/uefi/             # vendored UEFI:NTFS blob (+ NOTICE: license/source)
└── docs/adr/             # Architecture Decision Records
```

`ferrus-core` module map (all modules are documented stubs in Phase 0):

- `error`     — error types (`thiserror`).
- `device`    — device enumeration + **safe target selection** (removable only,
                excludes the system disk and critical mounts). Safety-critical.
- `source`    — ISO inspection: detect Windows vs generic, locate
                `install.wim`, measure its size.
- `partition` — GPT/MBR scheme, UEFI/Legacy layout.
- `format`    — `mkfs.ntfs` / `mkfs.vfat` wrappers.
- `copy`      — ISO content extraction/copy.
- `boot`      — bootloader install (UEFI:NTFS integration for NTFS; legacy
                later).
- `windows`   — **the differentiator**: `autounattend.xml` + LabConfig
                generation, parameterized by the ticked options.
- `progress`  — progress reporting (callback / channel).
- `platform`  — OS abstraction (cfg-gated: `linux` implemented,
                `windows`/`macos` stubbed) via traits, not scattered
                `if os == ...`.

## Safety rules (non-negotiable)

Ferrus wipes block devices. A mistake means destroyed data. These guards live
in the `device`/`partition` API, not as an afterthought:

- **Never** write to a non-removable device, or one hosting the system or a
  critical mount. Filter and refuse in code.
- **Explicit target confirmation** is mandatory (no guessed `/dev/sdX`).
- **`--dry-run`** must work from the very first brick.
- **Defensive enumeration**: only present plausible targets; show size / model /
  path to remove any ambiguity.
- Every destructive operation goes through a **single, tested checkpoint**.

## Code conventions

- Errors: `thiserror` in `ferrus-core`, `anyhow` in the binaries.
- **No `unwrap()` / `expect()` in the library** (except proven, commented
  invariants). Binaries may be more relaxed.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt` must pass.
- Doc-comments on every public item.
- Platform abstraction via traits + `#[cfg(...)]`, not scattered OS checks.
- Clear, atomic commits.

## Known gotchas

- Windows bypass keys / `autounattend.xml` schema **DRIFT across builds**
  (22H2 / 24H2 / 25H2). Never hardcode from memory; verify against current
  sources and keep per-build handling isolated and testable.
- GPL compliance: any vendored GPLv3 binary (UEFI:NTFS) must ship with its
  license and a source reference / written offer (see `res/uefi/NOTICE`).
- Device access needs elevated privileges (root / polkit on Linux — see
  ADR-0003).

## Build & test

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Roadmap

**Core validated on real hardware through Windows 11 25H2** (boot + hardware
bypass + local account without MSA). A Ferrus-made stick boots via the signed
UEFI:NTFS loader, gets past the Windows 11 requirement checks on a TPM-less VM,
and creates a local account without a Microsoft account — observed on a real
25H2 Setup run, not assumed.

Proof level is called out per phase: **[real]** = exercised on real hardware;
**[unit]** = covered by unit tests only.

- **Phase 0** (done): scaffolding + architecture + safety guards. — **[unit]**
- **Phase 1** (done): device enumeration + safe selection (Linux),
  `docs/specs/device.md` (SPEC-0001). — **[unit]** + enumeration/refusal
  exercised on the real host.
- **Phase 2** (done): generic raw image writing, `docs/specs/write.md`
  (SPEC-0002). — **[real]**: Alpine ISO written to a USB device and booted in
  QEMU; unmount + O_EXCL + fsync + read-back verify exercised.
- **Phase 3** (done): Windows install media — GPT + NTFS/FAT (3a),
  ISO file copy (3b), UEFI:NTFS bootloader (3c). `docs/specs/partition-format.md`
  (SPEC-0003), `docs/specs/copy-windows.md` (SPEC-0004),
  `docs/specs/bootloader.md` (SPEC-0005); ADR-0002/0004/0005. — **[real]**: a
  real Windows 11 25H2 ISO written (copy byte-identical to the ISO) and booted
  to Windows Setup via the signed UEFI:NTFS loader.
- **Phase 4** (done): the differentiator — `autounattend.xml` (hardware bypass +
  local account), `docs/specs/unattend.md` (SPEC-0006). — **[real]**: generator
  unit-tested; validated on a real Windows 11 25H2 install — no TPM wall, local
  account created without a Microsoft account.
- **Phase 5**: GUI (iced — ADR-0001).
- **Phase 6**: Windows port.
- **Phase 7**: macOS port.

Not yet done / known gaps: no GUI; Linux-only; UEFI/GPT only (no legacy BIOS);
telemetry/BitLocker/regional tweaks not implemented (phase 4.x);
`cargo clippy`/`cargo fmt` not yet run project-wide (tracked separately).
