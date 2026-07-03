//! System/critical-disk detection.
//!
//! Given `/proc/mounts` and `/proc/swaps`, compute the set of physical disks
//! that back the running system, so enumeration can flag them. The tricky part
//! is resolving a mount source to its physical disk(s): a source may be a
//! device-mapper / LUKS / LVM / md-raid node whose real backing store is found
//! by walking `slaves/`, not by stripping partition digits (see SPEC-0001).
//!
//! The [`BlockFs`] trait abstracts the filesystem lookups so the resolution
//! logic is unit tested with a fake (see `../tests.rs`); [`RealBlockFs`] is the
//! production implementation over sysfs.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Mountpoints whose backing disk is treated as system/critical. Conservative
/// on purpose: over-marking only ever protects an internal disk — a removable
/// stick is never a critical backing store.
pub(super) const CRITICAL_MOUNTPOINTS: [&str; 7] =
    ["/", "/boot", "/boot/efi", "/usr", "/var", "/etc", "/home"];

/// Filesystem lookups needed to resolve a mount source to its physical disks.
/// Abstracted for testability.
pub(super) trait BlockFs {
    /// Canonicalize a `/dev` path to its block device name
    /// (e.g. `/dev/mapper/luks-x` → `dm-0`).
    fn dev_to_block_name(&self, dev_path: &str) -> Option<String>;
    /// Entries of `/sys/class/block/<name>/slaves` (empty if none).
    fn slaves(&self, name: &str) -> Vec<String>;
    /// Whether `/sys/class/block/<name>/partition` exists.
    fn is_partition(&self, name: &str) -> bool;
    /// Parent disk name of a partition.
    fn parent_disk(&self, name: &str) -> Option<String>;
}

/// Production [`BlockFs`] backed by sysfs and `/dev`.
pub(super) struct RealBlockFs;

impl BlockFs for RealBlockFs {
    fn dev_to_block_name(&self, dev_path: &str) -> Option<String> {
        let real = fs::canonicalize(dev_path).ok()?;
        Some(real.file_name()?.to_string_lossy().into_owned())
    }

    fn slaves(&self, name: &str) -> Vec<String> {
        let dir = format!("/sys/class/block/{name}/slaves");
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn is_partition(&self, name: &str) -> bool {
        Path::new(&format!("/sys/class/block/{name}/partition")).exists()
    }

    fn parent_disk(&self, name: &str) -> Option<String> {
        let real = fs::canonicalize(format!("/sys/class/block/{name}")).ok()?;
        Some(real.parent()?.file_name()?.to_string_lossy().into_owned())
    }
}

/// Recursion depth guard against a pathological device stack (cycles should be
/// impossible in sysfs, but never trust unbounded recursion on external state).
const MAX_STACK_DEPTH: u32 = 16;

/// Collect the physical disk(s) backing a block device into `out`, walking
/// through device-mapper / md slaves down to real partitions/disks.
fn collect_backing_disks(name: &str, fs: &dyn BlockFs, out: &mut BTreeSet<String>, depth: u32) {
    if depth > MAX_STACK_DEPTH {
        return;
    }
    let slaves = fs.slaves(name);
    if !slaves.is_empty() {
        for slave in slaves {
            collect_backing_disks(&slave, fs, out, depth + 1);
        }
        return;
    }
    if fs.is_partition(name) {
        if let Some(disk) = fs.parent_disk(name) {
            out.insert(disk);
        }
        return;
    }
    // Already a whole disk.
    out.insert(name.to_owned());
}

/// Build the set of physical disk names that back the system, from the contents
/// of `/proc/mounts` and `/proc/swaps`.
pub(super) fn build_system_disk_set(
    mounts: &str,
    swaps: &str,
    fs: &dyn BlockFs,
) -> BTreeSet<String> {
    let mut disks = BTreeSet::new();

    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(source), Some(mountpoint)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !source.starts_with("/dev/") || !CRITICAL_MOUNTPOINTS.contains(&mountpoint) {
            continue;
        }
        if let Some(name) = fs.dev_to_block_name(source) {
            collect_backing_disks(&name, fs, &mut disks, 0);
        }
    }

    // `/proc/swaps` has a header line; every subsequent line's first field is
    // the swap source. Any swap device belongs to the system.
    for line in swaps.lines().skip(1) {
        let Some(source) = line.split_whitespace().next() else {
            continue;
        };
        if !source.starts_with("/dev/") {
            continue;
        }
        if let Some(name) = fs.dev_to_block_name(source) {
            collect_backing_disks(&name, fs, &mut disks, 0);
        }
    }

    disks
}

/// Mountpoints of every `/proc/mounts` entry whose source physically resides on
/// `disk` (following the same slaves-walking resolution). Used to unmount a
/// target's partitions before writing.
pub(super) fn mountpoints_backed_by(mounts: &str, disk: &str, fs: &dyn BlockFs) -> Vec<String> {
    let mut out = Vec::new();
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(source), Some(mountpoint)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !source.starts_with("/dev/") {
            continue;
        }
        if let Some(name) = fs.dev_to_block_name(source) {
            let mut disks = BTreeSet::new();
            collect_backing_disks(&name, fs, &mut disks, 0);
            if disks.contains(disk) {
                out.push(unescape_octal(mountpoint));
            }
        }
    }
    out
}

/// Decode the octal escapes `/proc/mounts` uses in path fields (`\040` = space,
/// `\011` = tab, `\012` = newline, `\134` = backslash). Without this, a stick
/// mounted at a path containing a space would fail to unmount.
pub(super) fn unescape_octal(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        // Try to read exactly three octal digits after the backslash.
        let rest = chars.as_str();
        let octal: String = rest.chars().take(3).collect();
        if octal.len() == 3
            && octal.bytes().all(|b| b.is_ascii_digit() && b < b'8')
            && let Ok(code) = u8::from_str_radix(&octal, 8)
        {
            out.push(code as char);
            // Consume the three digits we just decoded.
            for _ in 0..3 {
                chars.next();
            }
        } else {
            out.push('\\');
        }
    }
    out
}
