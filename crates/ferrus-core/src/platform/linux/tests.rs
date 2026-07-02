//! Fixture-based unit tests for the Linux backend's parsing and resolution.
//!
//! Nothing here touches real hardware: pure sysfs helpers are exercised with
//! string fixtures, and the system-disk resolution runs against a fake
//! [`BlockFs`](super::mounts::BlockFs) modeling a real LUKS-on-NVMe host.

use super::mounts::{BlockFs, build_system_disk_set};
use super::sysfs::{
    bus_from_syspath, flag_is_true, is_virtual_name, pick_stable_id, size_from_sectors,
};
use crate::device::Bus;

// --- sysfs pure helpers ---------------------------------------------------

#[test]
fn size_is_sectors_times_512() {
    assert_eq!(size_from_sectors("1000215216"), Some(1000215216 * 512));
    assert_eq!(size_from_sectors("  512\n"), Some(262_144));
    assert_eq!(size_from_sectors("not-a-number"), None);
}

#[test]
fn flag_parsing() {
    assert!(flag_is_true("1"));
    assert!(flag_is_true("1\n"));
    assert!(!flag_is_true("0"));
    assert!(!flag_is_true(""));
}

#[test]
fn bus_inference_from_syspath() {
    // Realpaths taken from actual hosts.
    assert_eq!(
        bus_from_syspath("/sys/devices/pci0000:00/.../nvme/nvme0/nvme0n1"),
        Bus::Nvme
    );
    assert_eq!(
        bus_from_syspath("/sys/devices/pci0000:00/0000:00:14.0/usb2/2-1/.../block/sdb"),
        Bus::Usb
    );
    assert_eq!(
        bus_from_syspath("/sys/devices/pci0000:00/0000:00:17.0/ata1/host0/.../block/sda"),
        Bus::Sata
    );
    assert_eq!(
        bus_from_syspath("/sys/devices/platform/.../mmc_host/mmc0/.../block/mmcblk0"),
        Bus::Mmc
    );
    assert_eq!(
        bus_from_syspath("/sys/devices/virtual/block/dm-0"),
        Bus::Unknown
    );
    assert_eq!(bus_from_syspath("/sys/devices/scsi_host/.../sdc"), Bus::Scsi);
}

#[test]
fn virtual_names_are_excluded() {
    for name in ["dm-0", "loop3", "ram0", "zram0", "md0", "sr0", "nbd0"] {
        assert!(is_virtual_name(name), "{name} should be virtual");
    }
    for name in ["sda", "nvme0n1", "mmcblk0", "sdb"] {
        assert!(!is_virtual_name(name), "{name} should be physical");
    }
}

#[test]
fn stable_id_prefers_human_readable_over_opaque() {
    let entries = vec![
        (
            "nvme-eui.000000000000000100a07522400c59c6".to_owned(),
            "nvme0n1".to_owned(),
        ),
        (
            "nvme-Micron_3400_MTFDKBA512TFH_2251400C59C6".to_owned(),
            "nvme0n1".to_owned(),
        ),
        // A partition-level id for the same disk — must be ignored.
        (
            "nvme-Micron_3400_MTFDKBA512TFH_2251400C59C6-part1".to_owned(),
            "nvme0n1p1".to_owned(),
        ),
    ];
    assert_eq!(
        pick_stable_id(&entries, "nvme0n1").as_deref(),
        Some("nvme-Micron_3400_MTFDKBA512TFH_2251400C59C6")
    );
    assert_eq!(pick_stable_id(&entries, "sdz"), None);
}

// --- system-disk resolution ----------------------------------------------

/// Fake filesystem modeling a LUKS-on-NVMe host: `/` is on
/// `/dev/mapper/luks-x` → `dm-0` → slave `nvme0n1p2` → disk `nvme0n1`.
struct FakeBlockFs;

impl BlockFs for FakeBlockFs {
    fn dev_to_block_name(&self, dev_path: &str) -> Option<String> {
        Some(match dev_path {
            "/dev/mapper/luks-x" => "dm-0".to_owned(),
            other => other.strip_prefix("/dev/").unwrap_or(other).to_owned(),
        })
    }

    fn slaves(&self, name: &str) -> Vec<String> {
        if name == "dm-0" {
            vec!["nvme0n1p2".to_owned()]
        } else {
            Vec::new()
        }
    }

    fn is_partition(&self, name: &str) -> bool {
        matches!(name, "nvme0n1p1" | "nvme0n1p2" | "nvme0n1p3" | "sdz1")
    }

    fn parent_disk(&self, name: &str) -> Option<String> {
        Some(match name {
            "nvme0n1p1" | "nvme0n1p2" | "nvme0n1p3" => "nvme0n1".to_owned(),
            "sdz1" => "sdz".to_owned(),
            _ => return None,
        })
    }
}

#[test]
fn root_on_luks_resolves_to_physical_disk() {
    let mounts = "\
/dev/mapper/luks-x / btrfs rw,relatime 0 0
/dev/mapper/luks-x /home btrfs rw,relatime 0 0
/dev/nvme0n1p1 /boot/efi vfat rw 0 0
tmpfs /tmp tmpfs rw 0 0
/dev/sdz1 /media/usb vfat rw 0 0
";
    let swaps = "Filename\tType\tSize\tUsed\tPriority\n";
    let set = build_system_disk_set(mounts, swaps, &FakeBlockFs);

    // The physical disk behind the LUKS root and /boot/efi is flagged.
    assert!(set.contains("nvme0n1"));
    // The intermediate dm node is not itself a "disk".
    assert!(!set.contains("dm-0"));
    // A USB stick merely mounted at a non-critical path stays a candidate.
    assert!(!set.contains("sdz"));
    assert!(!set.contains("sdz1"));
}

#[test]
fn swap_device_marks_its_disk() {
    let mounts = "";
    let swaps = "Filename\tType\tSize\tUsed\tPriority\n/dev/nvme0n1p3 partition 8388604 0 -2\n";
    let set = build_system_disk_set(mounts, swaps, &FakeBlockFs);
    assert!(set.contains("nvme0n1"));
}
