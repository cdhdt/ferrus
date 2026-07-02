# SPEC-0001: `device` — enumeration and safe target selection

- **Status:** Accepted
- **Module:** `crate::device` (+ Linux backend in `crate::platform::linux`)
- **Linked ADRs:** ADR-0003 (privilege elevation)

## Role

Discover the block devices attached to the host, present only the plausible
write targets to the user with enough detail to disambiguate them, and provide
the **single checkpoint** (`SafeTarget::acquire`) through which every later
destructive operation must pass.

This module is responsible for *deciding what may be written to*. It is NOT
responsible for partitioning, formatting, copying, or any write — those are
later phases and stay stubs. Enumeration here is strictly read-only.

## Invariants

1. **A raw `Device` grants no write permission.** The only value that authorizes
   writing is a `SafeTarget`, and the only way to build one is
   `SafeTarget::acquire`. Encoded in the type system (private fields, no public
   constructor). *Tested:* the acquire refusal/accept tests.
2. **Never authorize a non-removable device.** *Tested:* refusal test.
3. **Never authorize a device that backs the running system or a critical mount**
   (`/`, `/boot`, `/boot/efi`, swap, and other essential OS mounts). *Tested:*
   refusal test with a device flagged system/critical.
4. **The confirmed path must equal the enumerated device path** — no guessed or
   mistyped `/dev/sdX` is ever accepted. *Tested:* path-mismatch refusal test.
5. **Re-check at the moment of use (TOCTOU).** Between enumeration and acquire a
   device can be mounted or become critical; `acquire` re-verifies live mount
   state via the platform backend before authorizing.
6. Refusals surface as `Error::UnsafeTarget`, a distinct, testable outcome — not
   a generic failure.

## Behavior

### What "removable" means on Linux, and why it is not enough

`/sys/block/<dev>/removable` is a single bit and is **not reliable on its own**:

- Many USB SSDs and USB-bridged drives report `removable = 0`. Trusting the bit
  alone would hide legitimate targets and, worse, could surface a data drive as
  a target if the bit were wrong the other way.
- An external USB disk may be a *precious backup*, not a scratch stick — the tool
  must still show size/model/bus so the human can tell them apart. Ferrus does
  not silently decide a big USB disk is fair game.

So Ferrus treats `removable = 1` as the primary *display* filter but layers the
system/critical-mount check underneath as an independent guard (invariant 3), so
a mislabeled device that backs the OS is still refused.

Bus is derived independently of the `removable` bit, by inspecting the sysfs
device path (the realpath of `/sys/block/<dev>`):

- contains `/usb` → USB, `/nvme/` → NVMe, `/ata` or `/ata` host → SATA/ATA,
  `/mmc` → MMC/SD, otherwise SCSI/Unknown.
- contains `/virtual/` → virtual; excluded entirely (see pitfalls).

### Exclusions (never even enumerated as candidates)

Virtual and pseudo devices are dropped: `loop*`, `ram*`, `zram*`, `dm-*`,
`md*`, `sr*` (optical), `fd*`, `nbd*`. The robust single signal used is the
absence of `/sys/block/<dev>/device` (physical block devices have it; dm/loop/
md/zram do not). Devices reporting size 0 (e.g. an empty card reader) are also
dropped.

### System / critical-disk detection

A candidate disk is marked system/critical if it *physically backs* any critical
mount or any swap. Critical mountpoints: `/`, `/boot`, `/boot/efi`, `/usr`,
`/var`, `/etc`, `/home` (conservative — over-marking only ever protects internal
disks; a removable stick is never a critical backing store).

Resolution from a mount/swap source to physical disk(s) MUST follow the storage
stack, not just strip partition digits:

- `/proc/mounts` / `/proc/swaps` give a source like `/dev/nvme0n1p1`,
  `/dev/sda2`, or `/dev/mapper/luks-…`.
- Canonicalize the source to its block name (`/dev/mapper/luks-…` →
  `/dev/dm-0` → `dm-0`).
- If the block device has non-empty `/sys/class/block/<name>/slaves/`
  (device-mapper / LUKS / LVM / md-raid), **recurse into each slave** until
  reaching real partitions/disks.
- If it is a partition (`/sys/class/block/<name>/partition` exists), its parent
  disk is the basename of the parent of `canonicalize(/sys/class/block/<name>)`.
- Otherwise it is already a whole disk.

This yields the *set* of physical disks backing a mount (a LUKS/LVM/RAID volume
can span several). Any enumerated disk in that set is `is_system_or_critical`.

### `SafeTarget::acquire` contract

Preconditions (all required, checked in this order; first failure wins):

1. `device.removable == true` — else `UnsafeTarget`.
2. `device.is_system_or_critical == false` — else `UnsafeTarget`.
3. `confirmed_path == device.path` — else `UnsafeTarget`.
4. Live re-check: backend reports the device is not currently system/critical —
   else `UnsafeTarget`.

Guarantees on success: the returned `SafeTarget` wraps the device and the
`dry_run` flag; possessing it means all four checks passed. It is the only type
downstream destructive code will accept.

### User-facing metadata (to remove ambiguity)

For each candidate the CLI shows: operational path (`/dev/<name>`), a
human-readable size (decimal units, matching how storage is labeled), model,
and bus. A **stable** `/dev/disk/by-id/*` name is resolved best-effort and shown
when available, since `/dev/sdX` ordering is not stable across reboots/hotplug;
the operational path used for writes remains `/dev/<name>`.

## Known pitfalls

- **`removable` bit lies both ways** — see above; never the sole gate.
- **Root on LUKS/LVM/RAID.** On this project's own dev host, `/` is
  `/dev/mapper/luks-…` → `dm-0` → slave `nvme0n1p2` → disk `nvme0n1`. Naïve
  "strip trailing digits" partition→disk mapping would miss the physical disk
  entirely. Always walk `slaves/`.
- **NVMe/MMC partition naming** (`nvme0n1p3`, `mmcblk0p1`) differs from SCSI
  (`sda3`); rely on sysfs (`partition` file + realpath parent), not string
  surgery.
- **`/proc/mounts` octal escaping** (`\040` for space, etc.) in mountpoints —
  matters if a critical path ever contains spaces; the essential OS mounts do
  not, but decode before comparing if the set grows.
- **TOCTOU** between enumeration and write — hence the live re-check in acquire.
- **sysfs `size` is in 512-byte sectors** regardless of physical/logical block
  size; multiply by 512 for bytes (do not read the drive's logical block size
  here).
- **Model strings carry trailing padding spaces** — trim.

## Out of scope

- Any write, unmount, partition, format, or boot operation (Phases 2+).
- Windows/macOS enumeration backends (Phases 6/7 — stubs return `Unsupported`).
- Privilege elevation (ADR-0003). Enumeration is fully unprivileged — reading
  `/sys/block`, `/proc/mounts` and `/dev/disk/by-id` needs no root — so no EUID
  gate is added in Phase 1. The EUID check belongs on the write path and lands
  with Phase 2.
- Hotplug monitoring / live device add-remove events.
