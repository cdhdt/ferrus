# SPEC-0004: copy Windows ISO contents onto the NTFS partition (Phase 3b)

- **Status:** Accepted
- **Module:** `crate::copy` (+ `crate::source`) + Linux backend in
  `crate::platform::linux`
- **Linked ADRs:** ADR-0002 (UEFI:NTFS), ADR-0005 (NTFS write driver)
- **Linked specs:** SPEC-0002 (block copy loop, size guard, sync reused),
  SPEC-0003 (the P1 NTFS partition this fills)

## Role

After Phase 3a has created P1 (NTFS), copy the **entire contents** of a Windows
install ISO onto it: mount the ISO read-only, mount P1 read-write, stream every
file across (including `install.wim`/`.esd` > 4 GB — the whole reason P1 is
NTFS), `sync`, and unmount. Runs as the second half of the same
`prepare-windows` command.

In scope (3b): validate the image is a Windows install ISO, mount ISO (ro) + P1
(rw), recursively copy the tree preserving names/case/structure, space-guard,
sync, and reliably unmount.

NOT in scope: the UEFI:NTFS bootloader on P2 (3c), `autounattend.xml` (Phase 4).
**After 3b the stick still does NOT boot** — P2 is an empty FAT filesystem. Say
so; do not imply otherwise.

## Image validation (verified, not guessed)

A Windows install ISO is recognized only by **real markers present once mounted**
— never by file extension or name. Required (case-insensitive, since ISO9660 may
upper-case names while UDF preserves them):

- `sources/install.wim` **or** `sources/install.esd`, **and**
- `bootmgr`, **and**
- `efi/boot/bootx64.efi`

Missing any → `Error::NotWindowsMedia`, whose message points the user at the raw
path (Phase 2, `ferrus write`) for generic ISOs. Detection is a **pure** function
over the scanned relative path set (unit tested); the scan itself is I/O.

## NTFS write driver (see ADR-0005)

Mounting NTFS read-write needs a driver. Order: **`ntfs3` (in-kernel, fast)
first, fall back to `ntfs-3g` (FUSE)**. ntfs3 avoids FUSE overhead — it matters
for a multi-GB `install.wim`. If neither mounts rw → `Error::MissingTool` naming
the NTFS driver. The ISO is mounted `-o loop,ro` with **no forced fstype** (the
kernel picks iso9660 or UDF; modern Windows ISOs are UDF because of the >4 GB
`install.wim`).

## Invariants

1. **`&SafeTarget` only** (inherited: 3b runs inside `prepare-windows`, which is
   `SafeTarget`-gated). *Tested* via the 3a signature; 3b orchestration uses a
   test constructor.
2. **Dry-run mounts and copies nothing** — it reports the plan and the ISO size
   only. *Tested:* the mount backend is never called in dry-run.
3. **Windows-only.** A non-Windows tree is refused (`NotWindowsMedia`) before any
   NTFS mount/copy. *Tested* (pure detection + orchestration).
4. **Space guard.** Total ISO content bytes ≤ P1 capacity, checked **before**
   mounting P1 rw / copying; else `Error::InsufficientSpace`. *Tested.*
5. **Streaming.** Files are copied with the SPEC-0002 block loop (4 MiB); no file
   is ever read whole into memory — `install.wim` flows in blocks. *Tested* via
   the sink trait.
6. **Exact tree.** Names, case, and directory structure are preserved verbatim
   (no normalization). *Tested* against an in-memory tree.
7. **Reliable cleanup (RAII).** Both mounts are temporary (`mktemp -d`) and are
   unmounted **even if the copy fails mid-way**, via a `Drop` guard — there is
   never a lingering mount after an error. *Tested:* a forced mid-copy error
   still unmounts both.
8. **Sync before unmount.** The NTFS filesystem is synced before it is unmounted
   (cached data = corruption otherwise), same rule as SPEC-0002's fsync.
   *Tested:* sync is invoked before success.

## Behavior — sequence (real run, after 3a)

mount ISO (ro, loop) → scan (total bytes + relative file set) → **detect Windows**
(else `NotWindowsMedia`) → **space guard** (else `InsufficientSpace`) → mount P1
(rw, ntfs3→ntfs-3g) → **recursive streaming copy** ISO→P1 → **sync** P1 →
(guards drop → unmount both) → optional `--verify`.

Dry-run stops after reporting the plan; nothing is mounted.

### `--verify` (light)

After sync, compare the size of `install.wim`/`.esd` on the destination against
the source; a mismatch is `Error::VerificationFailed`. A full content hash is
deferred — TODO(phase3b.1).

### Testability seam

Detection and the space guard are pure. The recursive copy runs against a
`TreeIo` trait (list/read/create/write), tested with an in-memory tree. Mounting
+ sync sit behind a `MountBackend` trait returning RAII `Mount` guards; the
orchestration is tested with a fake that records mounts/unmounts and can fail the
copy, proving cleanup. Same philosophy as SPEC-0002/0003.

## Known pitfalls

- **UDF, not iso9660.** Modern Windows ISOs are UDF (they contain >4 GB files).
  Do not force `-t iso9660`; let the kernel autodetect.
- **NTFS driver availability.** ntfs3 needs kernel ≥ 5.15; fall back to ntfs-3g;
  fail clearly if neither mounts rw.
- **Cleanup on failure.** A half-done copy must still unmount both filesystems
  and remove the temp mountpoints — RAII `Drop`, not manual unwinding.
- **Sync is mandatory** before unmounting the NTFS mount.
- **NTFS overhead.** The space guard compares against the *partition* size;
  filesystem metadata means usable space is a little less, so a near-exact fit
  can still `ENOSPC` at copy time (surfaced as an I/O error). Acceptable — real
  sticks have ample margin.
- **No symlink following.** Use the entry's own file type; ISO trees are plain
  files/dirs.

## Out of scope

- Everything under "NOT in scope" in Role.
- Full-content post-copy hashing — TODO(phase3b.1).
