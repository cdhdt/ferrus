//! Audited Win32 FFI for Ferrus device enumeration (SPEC-0009, ADR-0007).
//!
//! **This is the only crate in the workspace that uses `unsafe`.** It wraps the
//! Win32 storage IOCTLs and exposes *safe* functions returning plain Rust data,
//! so `ferrus-core` keeps `#![forbid(unsafe_code)]`. Every `unsafe` block carries
//! a `// SAFETY:` note; handles are RAII.
//!
//! The **pure parsing** (turning raw IOCTL buffers into disk numbers / model
//! strings) is separated from the FFI so it compiles and is unit-tested on any
//! host — the Windows-only I/O lives in [`imp`]. All handles are opened with
//! **zero access rights**: enough for the query IOCTLs, requiring **no
//! elevation** (so `ferrus list` works unprivileged).

use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// Plain data + pure parsing — compiled and tested on every host.
// ---------------------------------------------------------------------------

/// A physical disk described by its Win32 storage descriptor. Plain data — no
/// handles, safe to pass to `ferrus-core`.
#[derive(Debug, Clone)]
pub struct RawDisk {
    /// The `N` in `\\.\PhysicalDriveN`.
    pub number: u32,
    /// Total capacity in bytes.
    pub size_bytes: u64,
    /// Model string (product id), when the device exposes one.
    pub model: Option<String>,
    /// `STORAGE_BUS_TYPE` value (map with `Bus::from_windows_bus_type`).
    pub bus_type: u32,
    /// Whether the device reports removable media (display only — the transport
    /// bus is the reliable eligibility signal).
    pub removable_media: bool,
}

// `VOLUME_DISK_EXTENTS` / `DISK_EXTENT` byte layout (winioctl.h). Hardcoded so the
// parser is host-independent; a `const _` assertion under `cfg(windows)` (in
// `imp`) verifies these equal the real `windows-sys` ABI at compile time.
//
//   VOLUME_DISK_EXTENTS { DWORD NumberOfDiskExtents; DISK_EXTENT Extents[]; }
//   DISK_EXTENT         { DWORD DiskNumber; LARGE_INTEGER Start; Length; }  // 8-aligned
const VDE_COUNT_OFFSET: usize = 0; // NumberOfDiskExtents: u32
const VDE_EXTENTS_OFFSET: usize = 8; // Extents[]: u32 + 4 pad (DISK_EXTENT is 8-aligned)
const DISK_EXTENT_STRIDE: usize = 24; // u32 + 4 pad + i64 + i64
const DISK_EXTENT_DISKNUM_OFFSET: usize = 0; // DiskNumber: u32

/// `NumberOfDiskExtents` from a `VOLUME_DISK_EXTENTS` buffer (0 if too short).
/// Native-endian: the buffer is filled by the kernel on the same machine.
#[allow(dead_code)] // used by `imp` (Windows) and the tests
fn disk_extent_count(buf: &[u8]) -> u32 {
    buf.get(VDE_COUNT_OFFSET..VDE_COUNT_OFFSET + 4)
        .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
        .unwrap_or(0)
}

/// Collect **every** `DiskNumber` from a `VOLUME_DISK_EXTENTS` buffer.
///
/// A volume can span several disks (spanned / mirrored / RAID) — the system
/// volume most dangerously so — hence *all* extents are collected, not just the
/// first. Reads are bounds-checked: a truncated buffer stops early rather than
/// reading past the end (the FFI layer grows the buffer and retries on
/// `ERROR_MORE_DATA`, so a complete buffer is what normally reaches here).
#[allow(dead_code)] // used by `imp` (Windows) and the tests
fn disk_numbers_from_extents(buf: &[u8]) -> BTreeSet<u32> {
    let mut set = BTreeSet::new();
    for i in 0..disk_extent_count(buf) as usize {
        let off = VDE_EXTENTS_OFFSET + i * DISK_EXTENT_STRIDE + DISK_EXTENT_DISKNUM_OFFSET;
        let Some(bytes) = buf.get(off..off + 4) else {
            break; // truncated buffer — stop without over-reading
        };
        set.insert(u32::from_ne_bytes(bytes.try_into().unwrap()));
    }
    set
}

/// Read a NUL-terminated ANSI string embedded in `buf` at `offset` (as a
/// descriptor's `ProductIdOffset` points), trimmed. `0`/out-of-range → `None`.
///
/// The caller passes only the bytes the IOCTL actually returned, so this never
/// reads uninitialized tail of an over-allocated buffer.
#[allow(dead_code)] // used by `imp` (Windows) and the tests
fn read_ansi_at(buf: &[u8], offset: usize) -> Option<String> {
    if offset == 0 || offset >= buf.len() {
        return None;
    }
    let bytes = &buf[offset..];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `VOLUME_DISK_EXTENTS` buffer holding `disks.len()` extents.
    fn make_extents(disks: &[u32]) -> Vec<u8> {
        let mut buf = vec![0u8; VDE_EXTENTS_OFFSET + disks.len().max(1) * DISK_EXTENT_STRIDE];
        buf[VDE_COUNT_OFFSET..VDE_COUNT_OFFSET + 4]
            .copy_from_slice(&(disks.len() as u32).to_ne_bytes());
        for (i, &d) in disks.iter().enumerate() {
            let off = VDE_EXTENTS_OFFSET + i * DISK_EXTENT_STRIDE;
            buf[off..off + 4].copy_from_slice(&d.to_ne_bytes());
        }
        buf
    }

    #[test]
    fn collects_all_disk_numbers_across_extents() {
        // A spanned/RAID system volume across 3 disks → all three are protected.
        // This is the Windows counterpart of the Linux LUKS/LVM slaves test.
        let set = disk_numbers_from_extents(&make_extents(&[2, 0, 5]));
        assert_eq!(set, BTreeSet::from([0, 2, 5]));
    }

    #[test]
    fn single_extent_volume() {
        assert_eq!(
            disk_numbers_from_extents(&make_extents(&[3])),
            BTreeSet::from([3])
        );
    }

    #[test]
    fn truncated_buffer_stops_without_overreading() {
        // Header claims 3 extents but the buffer only holds 2: collect the 2 that
        // fit, never reading past the end.
        let mut buf = make_extents(&[7, 9]);
        buf[VDE_COUNT_OFFSET..VDE_COUNT_OFFSET + 4].copy_from_slice(&3u32.to_ne_bytes());
        assert_eq!(disk_numbers_from_extents(&buf), BTreeSet::from([7, 9]));
    }

    #[test]
    fn empty_or_short_buffers_yield_nothing() {
        assert!(disk_numbers_from_extents(&make_extents(&[])).is_empty());
        assert!(disk_numbers_from_extents(&[]).is_empty());
        assert!(disk_numbers_from_extents(&[0u8; 3]).is_empty()); // shorter than count field
        assert_eq!(disk_extent_count(&[0u8; 2]), 0);
    }

    #[test]
    fn reads_model_within_bounds() {
        let mut buf = vec![0u8; 32];
        buf[4..13].copy_from_slice(b"USB DISK\0");
        assert_eq!(read_ansi_at(&buf, 4).as_deref(), Some("USB DISK"));
        assert_eq!(read_ansi_at(&buf, 0), None); // offset 0 means "absent"
        assert_eq!(read_ansi_at(&buf, 999), None); // out of range → None, never a panic

        let mut spaced = vec![0u8; 16];
        spaced[2..6].copy_from_slice(b"AB  ");
        assert_eq!(read_ansi_at(&spaced, 2).as_deref(), Some("AB")); // trimmed
    }
}

// ---------------------------------------------------------------------------
// Win32 FFI — Windows only; the crate's only `unsafe`.
// ---------------------------------------------------------------------------

/// Compile-time proof that the hardcoded layout constants match the real
/// `windows-sys` ABI (checked whenever the crate is built for Windows).
#[cfg(windows)]
const _: () = {
    use windows_sys::Win32::System::Ioctl::{DISK_EXTENT, VOLUME_DISK_EXTENTS};
    assert!(std::mem::offset_of!(VOLUME_DISK_EXTENTS, NumberOfDiskExtents) == VDE_COUNT_OFFSET);
    assert!(std::mem::offset_of!(VOLUME_DISK_EXTENTS, Extents) == VDE_EXTENTS_OFFSET);
    assert!(std::mem::size_of::<DISK_EXTENT>() == DISK_EXTENT_STRIDE);
    assert!(std::mem::offset_of!(DISK_EXTENT, DiskNumber) == DISK_EXTENT_DISKNUM_OFFSET);
};

#[cfg(windows)]
pub use imp::{enumerate_physical_disks, system_disk_numbers};

#[cfg(windows)]
mod imp {
    use std::collections::BTreeSet;
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_MORE_DATA, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        GET_LENGTH_INFORMATION, IOCTL_DISK_GET_LENGTH_INFO, IOCTL_STORAGE_QUERY_PROPERTY,
        PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
        StorageDeviceProperty,
    };
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;

    use super::{
        DISK_EXTENT_STRIDE, RawDisk, VDE_EXTENTS_OFFSET, disk_extent_count,
        disk_numbers_from_extents, read_ansi_at,
    };

    /// The highest `\\.\PhysicalDriveN` index scanned. Generous for a desktop;
    /// gaps are skipped, so a sparse numbering is fine.
    const MAX_PHYSICAL_DRIVES: u32 = 64;

    /// An owned Win32 handle, closed exactly once on drop.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a live handle this type uniquely owns (only ever
            // constructed from a checked, non-invalid `CreateFileW` result, and
            // never copied), so closing it exactly once here is valid.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Open a device path (`\\.\PhysicalDriveN` or `\\.\C:`) for query IOCTLs
    /// only: zero desired access → no elevation required.
    fn open_query(path: &str) -> io::Result<Handle> {
        let wide: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is a valid NUL-terminated UTF-16 string that outlives the
        // call; the security-attributes and template-handle pointers are null,
        // which `CreateFileW` explicitly permits.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Handle(handle))
    }

    /// Issue a `DeviceIoControl`, returning the number of bytes written to `out`.
    fn device_io_control(
        handle: HANDLE,
        code: u32,
        input: &[u8],
        out: &mut [u8],
    ) -> io::Result<u32> {
        let mut returned: u32 = 0;
        let in_ptr = if input.is_empty() {
            std::ptr::null()
        } else {
            input.as_ptr().cast()
        };
        // SAFETY: `handle` is a live handle owned by the caller; `in_ptr`/`out`
        // are valid for the lengths passed (their own `.len()`); `returned` is a
        // valid out-pointer; no overlapped I/O (null OVERLAPPED).
        let ok = unsafe {
            DeviceIoControl(
                handle,
                code,
                in_ptr,
                input.len() as u32,
                out.as_mut_ptr().cast(),
                out.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(returned)
    }

    /// Read the `STORAGE_DEVICE_DESCRIPTOR` (bus type, removable flag, model).
    fn query_descriptor(handle: HANDLE) -> io::Result<(u32, bool, Option<String>)> {
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        // SAFETY: `STORAGE_PROPERTY_QUERY` is a `#[repr(C)]` POD; viewing the live
        // local as its own bytes for the input buffer is sound and read-only.
        let input: &[u8] = unsafe {
            std::slice::from_raw_parts(
                (&query as *const STORAGE_PROPERTY_QUERY).cast::<u8>(),
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>(),
            )
        };
        let mut buf = vec![0u8; 1024];
        let n = device_io_control(handle, IOCTL_STORAGE_QUERY_PROPERTY, input, &mut buf)? as usize;
        if n < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return Err(io::Error::other("short STORAGE_DEVICE_DESCRIPTOR"));
        }
        // SAFETY: the first `n` bytes (>= one descriptor, checked) were written by
        // the kernel; read the fixed header unaligned since a `Vec<u8>` is only
        // byte-aligned. Fields beyond the header (the model string) are read via
        // the safe `read_ansi_at`, bounded to the returned `n` bytes.
        let desc =
            unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>()) };

        let bus_type = desc.BusType as u32;
        let removable = desc.RemovableMedia != 0;
        let model = read_ansi_at(&buf[..n], desc.ProductIdOffset as usize);
        Ok((bus_type, removable, model))
    }

    /// Read the disk length in bytes via `IOCTL_DISK_GET_LENGTH_INFO`.
    fn query_length(handle: HANDLE) -> io::Result<u64> {
        let mut buf = vec![0u8; std::mem::size_of::<GET_LENGTH_INFORMATION>()];
        device_io_control(handle, IOCTL_DISK_GET_LENGTH_INFO, &[], &mut buf)?;
        // SAFETY: the buffer is exactly one `GET_LENGTH_INFORMATION` and was
        // filled by the successful IOCTL; read it unaligned.
        let info =
            unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<GET_LENGTH_INFORMATION>()) };
        Ok(info.Length.max(0) as u64)
    }

    /// Fetch a volume's `VOLUME_DISK_EXTENTS` buffer, growing on `ERROR_MORE_DATA`.
    ///
    /// Per Microsoft's docs, a volume that spans more than one disk returns
    /// `ERROR_MORE_DATA` with `NumberOfDiskExtents` filled in; the caller must
    /// retry with a buffer sized for that many extents. We start at one extent and
    /// grow (to the reported count, or by doubling as a fallback) until it fits.
    fn query_volume_disk_extents(handle: HANDLE) -> io::Result<Vec<u8>> {
        let mut capacity = 1usize;
        for _ in 0..5 {
            let mut buf = vec![0u8; VDE_EXTENTS_OFFSET + capacity * DISK_EXTENT_STRIDE];
            match device_io_control(handle, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut buf) {
                Ok(_) => return Ok(buf),
                Err(e) if e.raw_os_error() == Some(ERROR_MORE_DATA as i32) => {
                    let needed = disk_extent_count(&buf) as usize;
                    capacity = needed.max(capacity.saturating_mul(2)).max(2);
                }
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("volume spans too many disks"))
    }

    /// Enumerate the physical disks (`\\.\PhysicalDrive0..N`), skipping any index
    /// that cannot be opened or queried.
    ///
    /// # Errors
    ///
    /// Currently infallible in aggregate (per-disk failures are skipped); returns
    /// a `Result` for forward compatibility.
    pub fn enumerate_physical_disks() -> io::Result<Vec<RawDisk>> {
        let mut disks = Vec::new();
        for number in 0..MAX_PHYSICAL_DRIVES {
            let path = format!(r"\\.\PhysicalDrive{number}");
            let Ok(handle) = open_query(&path) else {
                continue;
            };
            let Ok((bus_type, removable_media, model)) = query_descriptor(handle.0) else {
                continue;
            };
            let size_bytes = query_length(handle.0).unwrap_or(0);
            if size_bytes == 0 {
                continue; // no media / unreadable geometry
            }
            disks.push(RawDisk {
                number,
                size_bytes,
                model,
                bus_type,
                removable_media,
            });
        }
        Ok(disks)
    }

    /// The physical-disk numbers backing the **Windows OS volume** (`%SystemRoot%`)
    /// — the disk(s) that must never be a write target.
    ///
    /// Resolved via `GetWindowsDirectoryW` → its drive letter → the volume's disk
    /// extents. A spanned OS volume yields several disk numbers, all collected.
    ///
    /// This is **defense in depth**: the transport gate already refuses every
    /// fixed (non-USB/MMC) disk, so a normally-installed system/EFI disk is
    /// refused regardless. This additionally covers Windows-on-USB (Windows To
    /// Go), where Setup co-locates the ESP and OS on one disk. The ESP/boot volume
    /// on a *separate* disk is not independently resolved here — see SPEC-0009 for
    /// the reasoning and the deferral to the write phase.
    ///
    /// # Errors
    ///
    /// Returns an error if the Windows directory or the volume's extents cannot be
    /// read (enumeration then fails closed rather than listing with no guard).
    pub fn system_disk_numbers() -> io::Result<BTreeSet<u32>> {
        let mut win_dir = [0u16; 260];
        // SAFETY: pointer and length describe `win_dir`; `GetWindowsDirectoryW`
        // writes at most `len` UTF-16 units and returns the count written
        // (excluding the NUL). A path longer than the buffer returns a value
        // `> len` and does not fill it — rejected below, so the later slice is
        // always in bounds.
        let len =
            unsafe { GetWindowsDirectoryW(win_dir.as_mut_ptr(), win_dir.len() as u32) } as usize;
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        if len > win_dir.len() {
            return Err(io::Error::other("windows directory path too long"));
        }
        let dir = String::from_utf16_lossy(&win_dir[..len]);
        let drive = dir
            .chars()
            .next()
            .filter(char::is_ascii_alphabetic)
            .ok_or_else(|| io::Error::other("windows dir has no drive letter"))?;

        let volume_path = format!(r"\\.\{}:", drive.to_ascii_uppercase());
        let handle = open_query(&volume_path)?;
        let buf = query_volume_disk_extents(handle.0)?;
        Ok(disk_numbers_from_extents(&buf))
    }
}
