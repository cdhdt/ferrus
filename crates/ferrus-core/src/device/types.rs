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

    /// Whether this transport is considered removable for target eligibility.
    ///
    /// This is the **reliable** removability signal (the `removable` sysfs bit
    /// is not — see SPEC-0001). Only USB and SD/MMC transports qualify in
    /// Phase 1.
    #[must_use]
    pub fn is_removable_transport(self) -> bool {
        matches!(self, Bus::Usb | Bus::Mmc)
    }

    /// Map a Windows `STORAGE_BUS_TYPE` value to a [`Bus`]. Pure — unit tested on
    /// any host (the Windows enumeration feeds it the descriptor's `BusType`).
    ///
    /// Values are the `winioctl.h` `STORAGE_BUS_TYPE` constants (verified against
    /// Microsoft Learn, *STORAGE_BUS_TYPE* — the enum is sequential from 0):
    /// `BusTypeUsb = 7`, `BusTypeSd = 12`, `BusTypeMmc = 13`, `BusTypeSata = 11`,
    /// `BusTypeAta = 3`, `BusTypeAtapi = 2`, `BusTypeNvme = 17`, `BusTypeScsi = 1`,
    /// `BusTypeSas = 10`, `BusTypeRAID = 8`, `BusTypeiScsi = 9`. Anything not
    /// mapped — including the **virtual** buses (`BusTypeVirtual = 14`,
    /// `FileBackedVirtual = 15`, `Spaces = 16`) — is [`Bus::Unknown`], so it is
    /// **not** a removable transport and can never be an eligible target.
    #[must_use]
    pub fn from_windows_bus_type(bus_type: u32) -> Bus {
        match bus_type {
            7 => Bus::Usb,               // BusTypeUsb
            12 | 13 => Bus::Mmc,         // BusTypeSd | BusTypeMmc
            2 | 3 | 11 => Bus::Sata,     // BusTypeAtapi | BusTypeAta | BusTypeSata
            17 => Bus::Nvme,             // BusTypeNvme
            1 | 8 | 9 | 10 => Bus::Scsi, // BusTypeScsi | RAID | iScsi | Sas
            _ => Bus::Unknown,           // 1394/SSA/Fibre/Virtual/Spaces/SCM/UFS/Nvmeof/…
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
    /// Whether the OS reports this device as removable. **Display only** — this
    /// bit is unreliable (see SPEC-0001) and is never used as a gate; eligibility
    /// is decided by [`Bus::is_removable_transport`].
    pub removable: bool,
    /// Whether this device currently backs the running system or a mount the
    /// engine considers critical (swap, `/`, `/boot`, …). A `true` value is an
    /// automatic refusal.
    pub is_system_or_critical: bool,
}

impl Device {
    /// Returns `true` only if this device is, on the face of it, a plausible
    /// write target (removable *transport* and not system/critical).
    ///
    /// This is a **necessary but not sufficient** condition: it is a display
    /// filter for enumeration, not the authorization to write. Authorization
    /// only comes from [`SafeTarget::acquire`](super::SafeTarget::acquire).
    /// Note it does not consider size — the default-listing size heuristic lives
    /// in `list_writable_candidates`, not here.
    #[must_use]
    pub fn is_plausible_target(&self) -> bool {
        self.bus.is_removable_transport() && !self.is_system_or_critical
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
