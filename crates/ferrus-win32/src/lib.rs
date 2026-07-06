//! Audited Win32 FFI for Ferrus device enumeration (SPEC-0009, ADR-0007).
//!
//! **This is the only crate in the workspace that uses `unsafe`.** It wraps the
//! Win32 storage IOCTLs and exposes *safe* functions returning plain Rust data,
//! so `ferrus-core` keeps `#![forbid(unsafe_code)]`. Every `unsafe` block carries
//! a `// SAFETY:` note. The crate is Windows-only; on other targets it compiles
//! to nothing.
//!
//! All handles are opened with **zero access rights** — enough for the query
//! IOCTLs, and requiring **no elevation** (so `ferrus list` works unprivileged).

#![cfg(windows)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DISK_EXTENT, GET_LENGTH_INFORMATION, IOCTL_DISK_GET_LENGTH_INFO, IOCTL_STORAGE_QUERY_PROPERTY,
    PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
    StorageDeviceProperty, VOLUME_DISK_EXTENTS,
};
use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;

/// The highest `\\.\PhysicalDriveN` index scanned. Generous for a desktop; gaps
/// are skipped, so a sparse numbering is fine.
const MAX_PHYSICAL_DRIVES: u32 = 64;

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

/// An owned Win32 handle, closed on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a handle we opened and own; closing it once is valid.
        unsafe { CloseHandle(self.0) };
    }
}

/// Open a device path (`\\.\PhysicalDriveN` or `\\.\C:`) for query IOCTLs only:
/// zero desired access → no elevation required.
fn open_query(path: &str) -> io::Result<Handle> {
    let wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is a valid NUL-terminated UTF-16 string held for the call;
    // the security-attributes and template-handle pointers are null as allowed.
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

/// Issue a `DeviceIoControl` with the given input/output buffers, returning the
/// number of bytes written to `out`.
fn device_io_control(handle: HANDLE, code: u32, input: &[u8], out: &mut [u8]) -> io::Result<u32> {
    let mut returned: u32 = 0;
    let in_ptr = if input.is_empty() {
        std::ptr::null()
    } else {
        input.as_ptr().cast()
    };
    // SAFETY: `handle` is valid; the buffers and their lengths are consistent;
    // `returned` is a valid out-pointer; no overlapped I/O.
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
    // SAFETY: `STORAGE_PROPERTY_QUERY` is plain old data; reinterpreting it as its
    // bytes for the input buffer is sound.
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
    // SAFETY: the buffer holds at least a full descriptor (checked above); read it
    // unaligned since a `Vec<u8>` is only byte-aligned.
    let desc =
        unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>()) };

    let bus_type = desc.BusType as u32;
    let removable = desc.RemovableMedia != 0;
    let model = read_ansi_at(&buf, desc.ProductIdOffset as usize);
    Ok((bus_type, removable, model))
}

/// Read a NUL-terminated ANSI string embedded in `buf` at `offset` (as the
/// descriptor's `ProductIdOffset` points), trimmed. `0`/out-of-range → `None`.
fn read_ansi_at(buf: &[u8], offset: usize) -> Option<String> {
    if offset == 0 || offset >= buf.len() {
        return None;
    }
    let bytes = &buf[offset..];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Read the disk length (size in bytes) via `IOCTL_DISK_GET_LENGTH_INFO`.
fn query_length(handle: HANDLE) -> io::Result<u64> {
    let mut buf = vec![0u8; std::mem::size_of::<GET_LENGTH_INFORMATION>()];
    device_io_control(handle, IOCTL_DISK_GET_LENGTH_INFO, &[], &mut buf)?;
    // SAFETY: buffer sized for the struct; read unaligned.
    let info = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<GET_LENGTH_INFORMATION>()) };
    Ok(info.Length.max(0) as u64)
}

/// Enumerate the physical disks (`\\.\PhysicalDrive0..N`), skipping any index
/// that cannot be opened or queried.
///
/// # Errors
///
/// Currently infallible in aggregate (per-disk failures are skipped); returns a
/// `Result` for forward compatibility.
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

/// The physical-disk numbers that back the Windows installation volume — the
/// system/critical disks that must never be a write target.
///
/// Resolved via `GetWindowsDirectoryW` → its drive letter → the volume's disk
/// extents (`IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`). This is defense in depth:
/// the transport gate already refuses every fixed (non-USB/MMC) disk, so this
/// additionally guards the rare case of Windows installed on a USB disk.
///
/// # Errors
///
/// Returns an error if the Windows directory or the volume's extents cannot be
/// read.
pub fn system_disk_numbers() -> io::Result<BTreeSet<u32>> {
    // GetWindowsDirectoryW → e.g. "C:\\Windows".
    let mut win_dir = [0u16; 260];
    // SAFETY: buffer/len match; the call writes at most `len` code units.
    let len = unsafe { GetWindowsDirectoryW(win_dir.as_mut_ptr(), win_dir.len() as u32) };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    let dir = String::from_utf16_lossy(&win_dir[..len as usize]);
    let drive = dir
        .chars()
        .next()
        .ok_or_else(|| io::Error::other("empty windows dir"))?;
    if !drive.is_ascii_alphabetic() {
        return Err(io::Error::other("windows dir has no drive letter"));
    }

    let volume_path = format!(r"\\.\{}:", drive.to_ascii_uppercase());
    let handle = open_query(&volume_path)?;

    // Buffer for VOLUME_DISK_EXTENTS with room for several extents.
    let mut buf = vec![
        0u8;
        std::mem::size_of::<VOLUME_DISK_EXTENTS>()
            + 32 * std::mem::size_of::<DISK_EXTENT>()
    ];
    device_io_control(
        handle.0,
        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
        &[],
        &mut buf,
    )?;

    // SAFETY: buffer sized for the header; read the count unaligned.
    let count = unsafe {
        std::ptr::read_unaligned(buf.as_ptr().cast::<VOLUME_DISK_EXTENTS>()).NumberOfDiskExtents
    };
    let extents_base = std::mem::offset_of!(VOLUME_DISK_EXTENTS, Extents);
    let stride = std::mem::size_of::<DISK_EXTENT>();
    let disk_number_off = std::mem::offset_of!(DISK_EXTENT, DiskNumber);

    let mut set = BTreeSet::new();
    for i in 0..count as usize {
        let off = extents_base + i * stride + disk_number_off;
        if off + std::mem::size_of::<u32>() > buf.len() {
            break;
        }
        // SAFETY: `off` is within the buffer (checked); read the u32 unaligned.
        let disk_number = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off).cast::<u32>()) };
        set.insert(disk_number);
    }
    Ok(set)
}
