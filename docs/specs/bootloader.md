# SPEC-0005: UEFI:NTFS bootloader — make the Windows stick bootable (Phase 3c)

- **Status:** Implemented
- **Module:** `crate::boot` + Linux backend in `crate::platform::linux`
- **Linked ADRs:** ADR-0002 (UEFI:NTFS vendoring — this spec resolves it)
- **Linked specs:** SPEC-0002 (block write + O_EXCL + fsync reused), SPEC-0003
  (the P2 helper partition), SPEC-0004 (P1 Windows files)

## Role

Write the vendored **UEFI:NTFS** image raw onto the small FAT helper partition
(P2), the final step that makes a Windows install stick bootable under UEFI.
After 3a → 3b → 3c (with a real Windows ISO), the stick **boots**. This ends the
"not bootable yet" caveat of the earlier phases.

In scope (3c): verify the embedded image's integrity, then `dd` it onto P2.
Nothing else.

NOT in scope: Windows tweaks (`autounattend.xml` / LabConfig — Phase 4). Those do
not affect bootability.

## Mechanism (verified against pbatard, not assumed)

UEFI firmware cannot read NTFS, so it ignores P1 and boots the FAT partition P2.
The vendored `uefi-ntfs.img` is itself a FAT12 filesystem containing
`/EFI/Boot/bootx64.efi` (the UEFI:NTFS loader) and `/EFI/Rufus/ntfs_x64.efi` (the
NTFS driver) — confirmed by mounting the image. The firmware runs
`/EFI/Boot/bootx64.efi`; it loads the NTFS driver, mounts P1, and chain-loads
P1's `/efi/boot/bootx64.efi` — the Windows boot manager already copied to P1 by
Phase 3b. **3c therefore only deposits `uefi-ntfs.img` on P2; it never
regenerates the Windows loader.**

## Vendored asset (resolves ADR-0002)

- Source: Rufus repo, tag **v4.15**, `res/uefi/uefi-ntfs.img`
  (https://raw.githubusercontent.com/pbatard/rufus/v4.15/res/uefi/uefi-ntfs.img).
- Size: **1048576 bytes (exactly 1 MiB)**.
- SHA-256: `72683fa1250eeea772d3399277b434d4e55ba8dd0dc926e52d817e701fc2eb9e`.
- **Secure Boot-signed**: verified by byte comparison — the image's
  `/EFI/Boot/bootx64.efi` is identical to the official `bootx64_signed.efi` of
  pbatard/uefi-ntfs v2.8 (and differs from the unsigned one). See `res/uefi/NOTICE`.

### Consequences for the P2 partition (amends SPEC-0003)

- The image is exactly 1 MiB, so **P2 stays 1 MiB** — it fits exactly. No resize.
- The image carries its **own FAT12 filesystem**, so the `mkfs.vfat` that
  SPEC-0003 ran on P2 is **redundant and removed**: 3a no longer formats P2, it
  leaves it raw, and 3c writes the image over it. (P1 is still NTFS-formatted by
  3a.) A `prepare-windows` run without an image therefore leaves P2 raw — that is
  fine, P2 is only meaningful once the bootloader is written.

### How the asset is embedded

`include_bytes!` — the 1 MiB image is compiled into the Ferrus binary. Rationale:
a single self-contained binary (no external data file to locate/ship/keep in
sync at runtime), robust against a missing/misplaced asset, and negligible size.
A GPLv3 blob inside a GPLv3 binary is compliant (see NOTICE). The pinned SHA-256
is still checked at runtime (below) as a code/asset consistency guard.

## Invariants

1. **Integrity before writing.** Before any write, the embedded image's SHA-256
   is compared to the pinned `UEFI_NTFS_IMG_SHA256`; a mismatch is
   `Error::BootloaderIntegrity` and **nothing is written** — the bootloader is a
   chain-of-trust component. *Tested:* correct hash passes; a tampered byte
   fails with no write. A separate test asserts the embedded asset matches the
   constant (keeps code and asset in sync).
2. **Fits the partition.** If the image were larger than P2, refuse with
   `Error::ImageExceedsPartition` before writing. *Tested* via the pure check.
3. **`&SafeTarget` only.** 3c runs inside `prepare-windows`, gated by the
   SPEC-0001 checkpoint; the P2 write is a sub-operation authorized by holding
   the checkpoint. Honors dry-run (writes nothing). *Tested.*
4. **Reuse the Phase 2 write path.** The image is written with the same block
   loop + exclusive `O_EXCL` open + final `fsync`, targeted at the P2 partition
   node (not the whole device). *Tested* via the injected write backend.

## Behavior — full sequence (real run)

`prepare-windows` now runs, in one command, from-scratch:

3a partition + format P1 (NTFS); P2 left raw →
3b mount ISO, validate Windows, copy tree to P1, sync →
3c verify image integrity → `dd` `uefi-ntfs.img` onto P2 (O_EXCL, fsync).

Dry-run reports the full plan including the bootloader step, writing nothing.

## Known pitfalls

- **Image vs partition size.** The image must fit P2. It is exactly 1 MiB today;
  the size check guards against a future larger asset.
- **Signed vs unsigned.** Only the Secure Boot-signed image lets the stick boot
  with Secure Boot on; the vendored one is signed (verified). An unsigned image
  would force the user to disable Secure Boot.
- **Integrity.** Never write an unverified bootloader blob; the SHA-256 gate is
  mandatory and precedes the write.
- **P2 is raw until 3c.** Because 3a no longer formats it, tooling that inspects
  P2 between 3a and 3c sees an unformatted partition — expected.

## Out of scope

- Windows tweaks (Phase 4).
- Legacy BIOS boot (UEFI/GPT only here).
