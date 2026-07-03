# SPEC-0002: raw image write

- **Status:** Implemented
- **Module:** `crate::copy` (orchestration + stream) + `crate::source` (image) +
  Linux backend in `crate::platform::linux`
- **Linked ADRs:** ADR-0003 (privilege elevation)
- **Linked specs:** SPEC-0001 (device / `SafeTarget`)

## Role

Copy an already-bootable disk image **byte-for-byte** onto a whole target
device, the way `dd` does. This is the first code path in Ferrus that destroys
data, so every guard below is load-bearing.

In scope:

- Raw copy of a directly-bootable image (Linux isohybrid ISOs and images that
  are already bootable as-is) to the **whole** device.
- Unmounting the target's mounted partitions, exclusive open, block-by-block
  copy with progress, and a full `fsync` before reporting success.
- Optional post-write read-back verification (`--verify`).

NOT in scope (do not pretend otherwise):

- **Windows install media.** A Windows ISO carries `install.wim` > 4 GB and does
  **not** boot from a raw copy; it needs partitioning + file extraction +
  UEFI:NTFS. That is Phase 3. Raw-copying a Windows ISO here would produce
  media that does not boot — the tool must never imply it works.
- Partitioning, formatting, bootloader install (Phases 3–4).
- ISO9660/UDF parsing or content inspection (Phase 3). Phase 2 validates the
  image only as an opaque byte stream (exists, readable, size > 0).
- GUI.

Windows detection/warning is a *possible bonus*, not a phase goal.

## Invariants

1. **The destructive entry point takes `&SafeTarget`, never a raw path or a bare
   `Device`.** It is impossible to reach the write path without having passed the
   SPEC-0001 checkpoint (removable transport, not system/critical, exact
   confirmed path, live TOCTOU re-check). *Tested:* the signature; orchestration
   tests build a `SafeTarget` through a test-only constructor.
2. **Dry-run touches nothing.** When `SafeTarget::is_dry_run()`, the write path
   opens nothing for writing, unmounts nothing, and reports no bytes written —
   it only describes the plan. *Tested:* dry-run test asserts the backend's
   unmount/open/sync are never called.
3. **Writing requires root.** The effective UID must be 0 (device writes need
   privilege; enumeration deliberately did not — SPEC-0001 / ADR-0003). A clear
   error otherwise. *Tested:* pure EUID parse + a non-root refusal.
4. **The target's mounted partitions are unmounted before writing.** Writing
   under a live mount corrupts data and/or fails `EBUSY`. *Tested:* the unmount
   op is invoked for each mounted partition of the target.
5. **Defense in depth before unmounting:** re-check the target disk is not
   system/critical; if it is, **abort** without touching anything. This must
   never trigger given `SafeTarget`, but we do not gamble. *Tested:* fixture with
   a critical target aborts and unmounts nothing.
6. **Exclusive open.** The device is opened `O_WRONLY | O_EXCL`. On Linux 2.6+,
   `O_EXCL` without `O_CREAT` on a block device makes `open()` fail with `EBUSY`
   if the device is in use (verified against `man 2 open`), blocking concurrent
   openers and auto-remounts during the write.
7. **Never report success before a full `fsync`.** "Written" without `fsync`
   means data is still in the page cache — pull the stick and it is corrupt.
   Success is returned only after the final device sync succeeds. *Tested:* the
   sink's `sync` is called before `Ok`, and a failing sync fails the operation.
8. **Size guard.** Refuse when `image_size > device_size`, before any write.
   *Tested:* size-refusal test.
9. Failures are typed `Error` variants (`PrivilegeRequired`, `ImageTooLarge`,
   `DeviceBusy`, `UnsafeTarget`, `Io`), not generic panics. No `unwrap`/`expect`
   in the library.

## Behavior

### Algorithm (real write)

1. Resolve `device = target.device()`; read `image_size` from `source`.
2. **Size guard** (invariant 8).
3. If dry-run: describe the plan (device, image size, block size) via `progress`
   and return — nothing else runs (invariant 2).
4. **EUID gate** (invariant 3).
5. **Defense-in-depth** system/critical re-check on the target disk (invariant 5).
6. **Unmount** every mounted partition of the target (invariant 4).
7. **Open** the device `O_WRONLY | O_EXCL` (invariant 6); map `EBUSY` to
   `DeviceBusy`.
8. **Copy loop:** read the image in fixed blocks (4 MiB) and write each block to
   the sink, reporting cumulative bytes / total through `progress` (stage
   `Copying`). Progress reaches 100 % of `image_size`.
9. **fsync** the device (stage `Finalizing`); only then report success
   (invariant 7).
10. If `--verify`: re-open the device read-only and compare the first
    `image_size` bytes against the image; mismatch is an error.

### Testability seam

The copy loop is written against a `WriteSink` trait (`write_chunk` + `sync`),
not a concrete file, so it runs against an in-memory sink in tests. All
privileged/OS actions (EUID, mounted-partition discovery, unmount, exclusive
open, read-back) sit behind a `WriteBackend` trait; the real implementation is
Linux/sysfs/`umount(8)`-based, and a fake drives the orchestration tests with no
hardware. This mirrors the `BlockFs` seam used for SPEC-0001.

### Dry-run

Reports device, image size, and block size, and returns `Ok` having opened,
unmounted, and written nothing.

## Known pitfalls

- **Unmount first.** A mounted target → corruption or `EBUSY`. Discover mounts by
  resolving `/proc/mounts` sources to the target disk (reusing the SPEC-0001
  slaves-walking resolver), then `umount` each mountpoint.
- **`fsync` is mandatory.** Buffered writes to a block device return long before
  the data hits the flash. `File::sync_all()` (fsync) before success — verified
  via std, no `unsafe`.
- **`O_EXCL` semantics.** Only meaningful on a block device without `O_CREAT`
  (Linux 2.6+); do not rely on it as a create-exclusive lock.
- **TOCTOU.** A partition can be re-mounted between enumeration and open; the
  exclusive open is the final gate (it fails `EBUSY` if anything grabbed it).
- **No `O_DIRECT`.** We use buffered writes, so arbitrary write lengths are fine
  (no sector-alignment requirement). The trade-off is the mandatory final fsync.
- **Kernel partition-table cache.** After a raw write the in-kernel partition
  table is stale until a re-read (`BLKRRPART` / `partprobe`). Not done in Phase 2
  (needs an ioctl / external tool) — TODO(phase2.1).
- **`umount(8)` dependency.** Unmount shells out to util-linux `umount` because
  the `umount2` syscall would require `unsafe`, which the library forbids.

## Out of scope

- Everything under "NOT in scope" in Role.
- Re-reading the partition table after write — TODO(phase2.1).
- Progress ETA/throughput smoothing (cosmetic; CLI concern).
