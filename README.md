# Ferrus

Cross-platform bootable USB creator — write any ISO, with Rufus-style tweaks for
Windows installs (skip TPM, Secure Boot, RAM checks and the Microsoft account
requirement). Written in Rust.

> **Status:** the engine works end-to-end on Linux and is CLI-only for now. No
> GUI yet; Windows and macOS ports are planned.

## Status

**What Ferrus does today (Linux, CLI):**

- Writes any already-bootable ISO raw to a USB device (`ferrus write`) — verified
  on real hardware (an Alpine ISO written and booted).
- Builds a **bootable Windows 11 install stick** (`ferrus prepare-windows`):
  GPT + NTFS/FAT layout, copies the ISO, and installs the signed **UEFI:NTFS**
  bootloader — verified on real hardware with a Windows 11 25H2 ISO.
- Generates and drops an **`autounattend.xml`** to **bypass the Windows 11
  hardware checks** (TPM / Secure Boot / RAM / storage / CPU) and **create a
  local account without a Microsoft account** — verified on a real Windows 11
  **25H2** install (no requirement wall, local account created, no MSA).
- Safety first: refuses non-removable / system disks, requires an exact target
  path, real `--dry-run`, and a single tested checkpoint before any write.

Example — build a Windows 11 install stick with the tweaks:

```sh
ferrus list
sudo ferrus prepare-windows --target /dev/sdX --image Win11.iso \
     --bypass-hardware --local-account myname --local-password 'secret'
```

**What does NOT exist yet:**

- No GUI (planned, iced — see `docs/adr/0001-gui-framework.md`).
- Linux only; UEFI/GPT only (no legacy BIOS boot).
- No telemetry / BitLocker / regional-settings tweaks yet.

## How it works

Ferrus writes any Linux/utility ISO to a USB stick, and — its real value — sets
up **Windows install media the Rufus way**:

- The Windows install image (`install.wim`) exceeds 4 GB, so the main partition
  is **NTFS**. A tiny FAT helper partition carries **UEFI:NTFS**, a small GPLv3
  EFI bootloader that lets UEFI firmware boot the NTFS partition.
- The install tweaks are **file drops, not binary patching**: an
  `autounattend.xml` at the USB root plus **LabConfig** registry keys. Together
  they can bypass the TPM / Secure Boot / RAM / storage / CPU checks, skip the
  Microsoft-account requirement, create a local account, and turn off telemetry.

The `autounattend.xml` generator is the heart of the project.

## Build

Requires a Rust toolchain with **edition 2024** support (Rust ≥ 1.85).

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Run the CLI (a real `--dry-run` is available from the start):

```sh
cargo run -p ferrus-cli -- --help
```

## Installation

A plain install target (not distro packaging) sets Ferrus up the way an end user
runs it — the GUI unprivileged, elevating a small **root-owned helper** through
**named polkit actions**:

```sh
sudo make install      # release build + install; sudo make uninstall to remove
```

It installs:

- `ferrus` and `ferrus-gui` → `/usr/bin/`
- `ferrus-helper` → `/usr/libexec/` (root-owned; this path is the `exec.path` of
  the polkit actions — they must match)
- the polkit actions → `/usr/share/polkit-1/actions/`
- two desktop entries → `/usr/share/applications/`: **Ferrus** and **Ferrus
  (software rendering)** (the latter forces CPU rendering — see Troubleshooting)

Once installed, `ferrus-gui` uses the installed helper (and its named polkit
action) — `$FERRUS_HELPER` is ignored for safety. For development, don't install;
point the GUI at your build with `FERRUS_HELPER=target/debug/ferrus-helper
cargo run -p ferrus-gui`.

## Safety

Ferrus erases block devices. It refuses to touch non-removable devices or the
system disk, requires explicit target confirmation, and supports `--dry-run`.
See the safety rules in [`CLAUDE.md`](CLAUDE.md).

## Troubleshooting — display / text artifacts

The GUI (`ferrus-gui`) renders on the GPU via iced's `wgpu` backend by default.
On some GPUs/drivers, wgpu **initializes successfully but renders corrupted text**
(garbled glyphs, e.g. on hover) — this is a driver/GPU issue, not a Ferrus bug,
and Ferrus cannot reliably auto-detect it.

The fix is to switch to the **CPU (software) renderer**, `tiny-skia`, which is
always compiled in. Run:

```sh
ICED_BACKEND=tiny-skia ferrus-gui
```

Rendering will be slower but pixel-correct. On startup `ferrus-gui` prints one
line noting the active backend and this workaround. (iced 0.14 offers no
programmatic backend switch, so this is set via the environment variable.)

## License

[GPL-3.0-or-later](LICENSE). Ferrus embeds [UEFI:NTFS](https://github.com/pbatard/uefi-ntfs)
(also GPLv3); see [`res/uefi/NOTICE`](res/uefi/NOTICE).

## Suggested GitHub topics

`rust` · `bootable-usb` · `usb` · `iso` · `windows` · `linux` · `macos` ·
`cross-platform` · `rufus`
