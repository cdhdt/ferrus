//! Pure sysfs parsing helpers.
//!
//! Everything here is a pure function of its string/slice inputs, so it is unit
//! tested directly with fixtures (see `../tests.rs`). Actual file reads live in
//! the parent module's orchestration.

use crate::device::Bus;

/// Parse a `/sys/block/<dev>/size` value into bytes.
///
/// sysfs reports size in **512-byte sectors** regardless of the device's
/// logical block size, so the byte count is always `sectors * 512`.
pub(super) fn size_from_sectors(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok().map(|sectors| sectors * 512)
}

/// Interpret a sysfs boolean flag file (e.g. `removable`): `"1"` is true.
pub(super) fn flag_is_true(raw: &str) -> bool {
    raw.trim() == "1"
}

/// Infer the [`Bus`] from the canonical sysfs path of a block device
/// (the realpath of `/sys/block/<dev>`).
///
/// The device-tree path contains a bus marker: `usb`, `nvme`, `mmc`, `ata`.
/// Anything under `virtual` is not a physical bus; other physical devices fall
/// back to SCSI.
pub(super) fn bus_from_syspath(syspath: &str) -> Bus {
    if syspath.contains("/usb") {
        Bus::Usb
    } else if syspath.contains("/nvme") {
        Bus::Nvme
    } else if syspath.contains("/mmc") {
        Bus::Mmc
    } else if syspath.contains("/ata") {
        Bus::Sata
    } else if syspath.contains("/virtual/") {
        Bus::Unknown
    } else {
        Bus::Scsi
    }
}

/// Names under `/sys/block` we never treat as candidates. Virtual/pseudo
/// devices (device-mapper, loop, md-raid, zram, optical, floppy, nbd) are
/// excluded; the parent module also requires a `device` symlink as the primary
/// signal, this is a belt-and-suspenders name filter.
pub(super) fn is_virtual_name(name: &str) -> bool {
    const PREFIXES: [&str; 8] = ["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"];
    PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// Choose a stable `/dev/disk/by-id/*` name for `dev_name` from a list of
/// `(link name, resolved target block name)` pairs.
///
/// Human-readable model-based ids are preferred over opaque `wwn-*` / `*eui.*`
/// hardware identifiers; among equals the shortest name wins.
pub(super) fn pick_stable_id(entries: &[(String, String)], dev_name: &str) -> Option<String> {
    let mut matches: Vec<&String> = entries
        .iter()
        .filter(|(_, target)| target == dev_name)
        .map(|(link, _)| link)
        .collect();
    matches.sort_by_key(|link| {
        let opaque = link.starts_with("wwn-") || link.contains("eui.");
        (opaque, link.len())
    });
    matches.first().map(|link| (*link).clone())
}
