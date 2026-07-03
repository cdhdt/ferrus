# SPEC-0003: partition + format for Windows install media (Phase 3a)

- **Status:** Implemented
- **Module:** `crate::partition` (+ `crate::format`) + Linux backend in
  `crate::platform::linux`
- **Linked ADRs:** ADR-0002 (UEFI:NTFS vendoring), ADR-0004 (sfdisk vs GPT crate)
- **Linked specs:** SPEC-0001 (device / `SafeTarget`), SPEC-0002 (write path —
  EUID gate, unmount, reread reused here)

## Role

Turn a blank USB device into the **partition + filesystem skeleton** of a
Windows install stick: a GPT with a large NTFS partition and a small FAT helper
partition at the end. This is destructive (it wipes the existing partition
table).

In scope (3a): erase the existing table, write the GPT below, and format P1 as
NTFS. Nothing else.

> **Amended by SPEC-0005:** P2 is **no longer formatted** by 3a. The UEFI:NTFS
> image written to P2 in 3c is itself a complete FAT12 filesystem, so an
> `mkfs.vfat` here would be redundant — 3a leaves P2 raw. (Original text below
> kept for context but superseded on this point.)

NOT in scope: mounting, copying Windows files (3b), writing the UEFI:NTFS
bootloader onto P2 (3c), generating `autounattend.xml` (Phase 4). **A stick
produced by 3a does not boot yet** — P2 is a raw, empty partition; the
bootloader arrives in 3c. Do not imply otherwise.

## Layout (verified, no invented GUIDs)

GPT, two partitions. Verified against `pbatard/rufus` `src/drive.c`
(`CreatePartition`) and canonical GPT type GUIDs (Wikipedia GPT article), and
the script was dry-checked against `sfdisk` on a throwaway image.

| # | Role | Position | Size | FS | GPT type GUID |
|---|------|----------|------|----|----|
| 1 | Windows files (3b) | offset 1 MiB → start of P2 | rest of disk | NTFS | `EBD0A0A2-B9E5-4433-87C0-68B6B72699C7` (Microsoft basic data) |
| 2 | UEFI:NTFS helper (3c) | the last aligned 1 MiB before the backup GPT | 1 MiB | FAT | `EBD0A0A2-B9E5-4433-87C0-68B6B72699C7` (Microsoft basic data) |

**P2 is Microsoft basic data, NOT an EFI System Partition.** This is deliberate
and matches Rufus, whose source carries the explicit comment *"Boy do you NOT
want the ESP of a GPT bootable drive to be declared as ESP"*: typing the helper
as ESP triggers unwanted OS/firmware special-casing. UEFI removable-media boot
still finds `/EFI/BOOT/BOOTX64.EFI` on a plain FAT partition, so ESP typing is
unnecessary and harmful here. (Canonical ESP GUID
`C12A7328-F81F-11D2-BA4B-00A0C93EC93B` is recorded only to document why it is
*not* used.)

### Geometry (all 1 MiB-aligned)

Everything is expressed in whole MiB so the plan is sector-size agnostic (sfdisk
converts MiB using the device's real logical sector size):

- front reserve: 1 MiB (protective MBR + primary GPT).
- back reserve: 1 MiB (backup GPT; far more than the ~33 sectors GPT needs, but
  keeps the tail 1 MiB-aligned and is trivially safe).
- P2: the 1 MiB immediately before the back reserve.
- P1: from 1 MiB to the start of P2 (fills the middle).

So for a device of `D` MiB: `P1 = [1, D-2) MiB`, `P2 = [D-2, D-1) MiB`. Refuse
devices below a small minimum (**64 MiB**) so P1 is a sane, formattable size.

### P2 size — why 1 MiB, and what is deferred

The real UEFI:NTFS payload (an NTFS UEFI driver ~130 KiB + the UEFI:NTFS loader
~40 KiB) is well under 1 MiB, so a 1 MiB FAT partition holds it comfortably with
FAT overhead, and 1 MiB is the alignment granule (no extra waste). This is **not
hardcoded from memory**: Rufus sizes P2 from the actual `uefi-ntfs.img`, whose
exact size is only known once the asset is vendored (3c, ADR-0002). 3a reserves a
safe aligned 1 MiB; 3c re-derives/validates against the vendored asset and, if it
is larger, revisits this size. TODO(phase3c): confirm P2 ≥ vendored payload.

### Deployment style note (finalized in SPEC-0005)

3c writes the vendored `uefi-ntfs.img` **raw** onto P2. That image is a complete
FAT12 filesystem (containing the UEFI:NTFS loader + NTFS driver), so P2 needs no
`mkfs.vfat` in 3a — it is left raw and overwritten wholesale in 3c. The image is
exactly 1 MiB, so P2 stays 1 MiB.

## Invariants

1. **`&SafeTarget` only.** The whole operation takes a `SafeTarget`; unreachable
   without the SPEC-0001 checkpoint. *Tested:* signature; orchestration tests use
   the test-only constructor.
2. **Dry-run touches nothing** — no table write, no mkfs, no unmount. *Tested.*
3. **Root required** (EUID gate, reused from SPEC-0002). *Tested* (fake euid).
4. **Tools present or clean failure.** `sfdisk`, `mkfs.ntfs`, `partprobe` are
   checked *before any destructive step* (P2 is not formatted, so `mkfs.vfat` is
   no longer required); a missing one yields
   `Error::MissingTool { name }` with nothing written. *Tested.*
5. **Unmount the target's mounted partitions first** (reused from SPEC-0002),
   with the defense-in-depth system/critical re-check that aborts before
   touching anything. *Tested.*
6. **Reread before formatting.** After writing the table, force the kernel to
   re-read partitions and **wait (bounded retry) for the `/dev/…1` and `/dev/…2`
   nodes to appear** before running mkfs — otherwise mkfs fails on a missing
   node. This resolves the SPEC-0002 `TODO(phase2.1)` for this path. *Tested:*
   the orchestration calls reread then wait then mkfs, in that order.
7. **Pure geometry.** Offsets/sizes are computed by a pure function and are all
   1 MiB-aligned, P1 bounds P2, P2 is last, all sizes > 0. *Tested.*

## Behavior — sequence (real run)

`SafeTarget` → compute layout (pure; `DeviceTooSmall` if below minimum) → **EUID
gate** → **ensure tools** → system/critical re-check → **unmount** mounted
partitions → **write GPT** (sfdisk) → **reread** (partprobe) → **wait for nodes**
→ **mkfs.ntfs P1** (P2 left raw for 3c) → done.

Dry-run stops after computing the layout and reporting the plan.

### Testability seam

Geometry and the sfdisk script are pure (unit tested). All privileged/OS actions
sit behind a `PartitionBackend` trait (EUID, unmount, tool check, table write,
reread, node wait, mkfs); the real impl is sfdisk/partprobe/mkfs-based, and a
fake drives the orchestration tests (asserting ordering and no-op-on-failure)
with no hardware. Same pattern as SPEC-0002's `WriteBackend`.

## Known pitfalls

- **Reread + node settle.** `sfdisk` issues `BLKRRPART`, but the `/dev/…N` nodes
  appear asynchronously (udev). mkfs must wait for them (bounded retry), else it
  fails on ENOENT. `partprobe` + poll is used.
- **P2 must NOT be ESP** — see above; typing it ESP breaks the Rufus scheme.
- **Alignment.** Misaligned partitions hurt flash performance and some
  firmwares; everything is 1 MiB-aligned.
- **Sector size.** Expressing the script in MiB (not raw 512-byte sectors) lets
  sfdisk handle 512 vs 4Kn devices correctly.
- **P2 size is confirmed** at 1 MiB: the vendored `uefi-ntfs.img` is exactly
  1 MiB (SPEC-0005), so it fits P2 precisely.
- **mkfs.ntfs is slow** unless quick-formatted (`--quick`); we quick-format.

## Out of scope

- Everything under "NOT in scope" in Role.
- Legacy/MBR + BIOS boot (UEFI/GPT only for Windows media in this phase).
