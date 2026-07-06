# SPEC-00010: partition + format on Windows (Phase 6.2a)

- Status: implemented — cross-compiles (`x86_64-pc-windows-gnu`), safety logic
  unit-tested on any host; **the real write is validated by a human on a throwaway
  disk** (procedure below). No ISO copy (6.2b) or bootloader (6.2c) here.
- Related: [SPEC-0003](partition-format.md) (the Linux model + the layout &
  verified type GUIDs — reused verbatim), [SPEC-0009](device-windows.md) (device +
  SafeTarget), [ADR-0004](../adr/0004-partitioning-tool.md) (sfdisk),
  [ADR-0007](../adr/0007-windows-unsafe-isolation.md) (unsafe isolation),
  [ADR-0008](../adr/0008-windows-gpt-write.md) (native GPT write).

## Role

The Windows counterpart of Phase 3a: turn a `SafeTarget` USB disk into the skeleton
of a Windows install stick — GPT + NTFS P1 + a raw FAT-helper P2. The **layout is
SPEC-0003's**, unchanged: P1 = Microsoft basic-data NTFS spanning almost the whole
disk, P2 = a ~1 MiB helper at the end. As on Linux, **P2 is left raw** here — the
UEFI:NTFS image written in 6.2c carries its own filesystem.

## Layout

Reused from SPEC-0003 via the shared `compute_windows_layout` / `GptLayout`. Both
partitions use the **Microsoft basic-data** type GUID
(`EBD0A0A2-B9E5-4433-87C0-68B6B72699C7`, verified against Microsoft Learn,
*PARTITION_INFORMATION_GPT*).

## Sequence (orchestration)

`partition::windows::prepare_partition`, testable with a fake backend:

1. Compute the layout and print the plan (P1/P2 offsets, sizes, type GUID).
2. **Dry-run stops here** — nothing is locked, dismounted, written, or formatted.
3. Require **Administrator** (`is_elevated`) — the Windows analogue of the Linux
   `EUID == 0` gate; fail closed otherwise. GUI-side UAC elevation is a later phase.
4. Re-check the target is not system/critical (defense in depth; `SafeTarget`
   already guarantees it).
5. **ESP guard** (see below) — refuse a disk that already carries a boot/system
   partition.
6. `write_gpt_layout`: **lock + dismount** the disk's volumes → `CREATE_DISK`
   (GPT) → `SET_DRIVE_LAYOUT_EX` → `UPDATE_PROPERTIES` (rescan).
7. Format **P1** as NTFS via PowerShell `Format-Volume`. P2 left raw.

## APIs + sources (Microsoft Learn; `windows-sys` provides the verified structs)

| Step | API | Source note |
| --- | --- | --- |
| GPT table write | `IOCTL_DISK_SET_DRIVE_LAYOUT_EX` (`DRIVE_LAYOUT_INFORMATION_EX` + 2× `PARTITION_INFORMATION_EX`) | *IOCTL_DISK_SET_DRIVE_LAYOUT_EX* — "write access required"; for GPT, `CREATE_DISK` first |
| Init disk | `IOCTL_DISK_CREATE_DISK` (`CREATE_DISK_GPT { DiskId, MaxPartitionCount }`) | same page's Remarks |
| Rescan (partprobe eq.) | `IOCTL_DISK_UPDATE_PROPERTIES` | — |
| Read current layout (ESP guard) | `IOCTL_DISK_GET_DRIVE_LAYOUT_EX` | variable-length (see below) |
| Unmount eq. | `FSCTL_LOCK_VOLUME` then `FSCTL_DISMOUNT_VOLUME` on each volume of the disk | |
| Find volumes on the disk | `FindFirstVolumeW`/`FindNextVolumeW` + `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` | match `DiskNumber` |
| Partition GUIDs | `UuidCreate` | fresh DiskId + PartitionId |
| Admin gate | `OpenProcessToken` + `GetTokenInformation(TokenElevation)` | `TOKEN_ELEVATION.TokenIsElevated` |
| Type GUIDs | ESP `C12A7328-…`, MSR `E3C9E316-…`, Recovery `DE94BBA4-…`, basic-data `EBD0A0A2-…` | *PARTITION_INFORMATION_GPT* |

## The ESP / system-partition guard (closes SPEC-0009's 6.1.1 TODO)

Once Ferrus *writes* partition tables, the transport gate (removable-only) is no
longer the sole line of defense. **Before any destructive step**,
`read_partition_type_guids` reads the target's current layout and
`refuse_if_system_partition` rejects it if any partition's type GUID is an **EFI
System**, **Microsoft Reserved**, or **Microsoft Recovery** partition. This guard is
unit-tested (allowed: plain basic-data / empty; refused: ESP, MSR, recovery — case-
and brace-insensitive) and runs *before* elevation-gated destruction in the
orchestration (also tested: dry-run touches nothing; not-elevated → refuse; ESP
present → refuse before any write).

## Variable-length buffers

`GET_DRIVE_LAYOUT_EX` (like the volume extents in SPEC-0009) is variable-length:
the code grows the buffer and retries on `ERROR_MORE_DATA` /
`ERROR_INSUFFICIENT_BUFFER`. The write side builds a fixed **two-entry**
`DRIVE_LAYOUT_INFORMATION_EX` via a `#[repr(C)]` wrapper.

## Geometry (real sector size)

The GPT usable range is derived from the disk's **real** logical sector size,
read via `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX` (`DISK_GEOMETRY.BytesPerSector`); if it
cannot be read, the write **fails closed** (never assumes 512). First usable = 34
sectors, backup reserve = 33 sectors (the UEFI/GPT layout — 34/33 *sectors*, which
is exact on 512 B and safely over-reserves on 4Kn). Partitions are aligned to the
sector size and the last partition is clamped to the last usable LBA (something
`sfdisk` did for us on Linux). The geometry (`(disk_size, bytes_per_sector) →
first/last usable`, partition placement) is a **pure function**, unit-tested for
512 B (identical to the previous behavior) and 4096 B (offsets aligned on 4096,
first usable = 34×4096). Still worth a real-hardware pass on an actual 4Kn disk.

## `write_gpt_layout` is an unguarded primitive

`ferrus_win32::write_gpt_layout` is a low-level, **unguarded** destructive
primitive: it locks/dismounts and rewrites the table with no safety checks of its
own. It must only be called **behind `prepare_partition`**, which owns the guards
(dry-run, Administrator, not-system, and the ESP guard). The ESP guard's layout
read (`read_partition_type_guids`) is itself **fail-closed**: a partition table
that does not fit the read buffer is an error, never a partial list that could miss
a system partition. Likewise, the volume lock/dismount step only treats a genuine
"no media" error (`ERROR_NOT_READY` / `ERROR_NO_MEDIA_IN_DRIVE`) as "not on this
disk"; any other error propagates, so a mounted volume is never silently skipped.

## Elevation

Writing a partition table requires Administrator. This phase only **detects and
requires** it (fail closed); wiring GUI-side UAC elevation (a manifest / an elevated
helper) is a later phase, the analogue of the Linux polkit work.

## Verification status

- `ferrus-core`, `ferrus-win32`, `ferrus-cli` **cross-compile** for
  `x86_64-pc-windows-gnu` and pass `clippy -D warnings` on that target.
- Unit-tested on any host: the ESP guard, the orchestration ordering + dry-run
  contract (fake backend), the PowerShell command construction + result parsing,
  and GUID parse/format round-trips.
- The IOCTL write itself is **not** executed in CI — compiled, not run.

## Manual test procedure (human — throwaway disk only)

⚠️ **Destructive. Use a disposable USB stick / VM scratch disk you can lose.**

In a Windows VM (or a machine with a spare stick):

1. Build: `cargo build -p ferrus-cli` (or a cross-built `ferrus.exe`).
2. **Dry run first — nothing is touched:**
   `ferrus prepare-windows --target \\.\PhysicalDriveN --image any.iso --dry-run`
   → prints the P1/P2 plan and stops. Confirm the target number `N` with
   `Get-Disk` / `wmic diskdrive get Index,Model,Size` first.
3. **ESP guard check:** point it (dry-run is fine to reason about, but the guard
   runs on a real write) at a disk carrying an ESP → it must refuse
   (`refused for safety`). A plain data stick is accepted.
4. **Real write, elevated, on the throwaway disk:** in an **Administrator** shell,
   `ferrus prepare-windows --target \\.\PhysicalDriveN --image any.iso` (no
   `--dry-run`). Without admin it must fail closed with a privileges error.
5. **Verify the result:** `Get-Partition -DiskNumber N` should show two partitions
   (a large NTFS P1 + a ~1 MiB P2), and `Get-Volume` an NTFS volume for P1. Cross-
   check with `diskpart` → `list disk` / `select disk N` / `list partition`.

Report the plan, the guard behavior, and the resulting partition table.

## Out of scope

ISO copy (6.2b), UEFI:NTFS bootloader (6.2c), GUI UAC elevation, MBR/legacy.
