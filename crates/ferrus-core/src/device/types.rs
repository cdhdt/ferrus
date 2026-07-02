//! Device data types: [`Device`], its [`Bus`], and human-readable size
//! formatting. Pure data — no I/O, no platform code.

use std::fmt;
use std::path::PathBuf;

/// The hardware bus a device sits on, inferred from its sysfs path.
///
/// Used purely to help the user disambiguate targets; it is never a safety
/// gate on its own (see SPEC-0001 — the `removable` bit is unreliable and so is
/// any bus heuristic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Bus {
    /// USB (the common case for target sticks).
    Usb,
    /// SATA / ATA.
    Sata,
    /// NVMe.
    Nvme,
    /// SD / MMC card.
    Mmc,
    /// SCSI or another physical bus not otherwise classified.
    Scsi,
    /// Bus could not be determined.
    Unknown,
}

impl Bus {
    /// Short, stable lowercase label for display.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Bus::Usb => "usb",
            Bus::Sata => "sata",
            Bus::Nvme => "nvme",
            Bus::Mmc => "mmc",
            Bus::Scsi => "scsi",
            Bus::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Bus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A block device discovered on the host.
///
/// This describes a device; it does **not** grant permission to write to it.
/// Obtain a [`SafeTarget`](super::SafeTarget) via
/// [`SafeTarget::acquire`](super::SafeTarget::acquire) for that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Operational OS path of the whole device (e.g. `/dev/sdb`). This is what
    /// gets written to; note it is not stable across reboots (see `stable_id`).
    pub path: PathBuf,
    /// A stable `/dev/disk/by-id/*` name when one could be resolved. Shown to
    /// disambiguate; not used as the write path.
    pub stable_id: Option<String>,
    /// Human-readable model string, when the OS exposes one.
    pub model: Option<String>,
    /// Bus the device sits on, for disambiguation.
    pub bus: Bus,
    /// Total capacity in bytes.
    pub size_bytes: u64,
    /// Whether the OS reports this device as removable.
    pub removable: bool,
    /// Whether this device currently backs the running system or a mount the
    /// engine considers critical (swap, `/`, `/boot`, …). A `true` value is an
    /// automatic refusal.
    pub is_system_or_critical: bool,
}

impl Device {
    /// Returns `true` only if this device is, on the face of it, a plausible
    /// write target (removable and not system/critical).
    ///
    /// This is a **necessary but not sufficient** condition: it is a display
    /// filter for enumeration, not the authorization to write. Authorization
    /// only comes from [`SafeTarget::acquire`](super::SafeTarget::acquire).
    #[must_use]
    pub fn is_plausible_target(&self) -> bool {
        self.removable && !self.is_system_or_critical
    }
}

/// Format a byte count using decimal units (kB/MB/GB/TB), matching how storage
/// capacity is labeled on the device itself.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
