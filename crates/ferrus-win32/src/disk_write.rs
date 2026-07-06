//! Windows disk write path (SPEC-00010, Phase 6.2a): elevation check, the ESP
//! guard's layout read, and the GPT partition-table write (lock → dismount →
//! create → set-layout → rescan). The **only** write-capable `unsafe` in the
//! project; every block carries a `// SAFETY:` note and handles are RAII.
//!
//! Geometry note: the GPT usable range is derived from the disk's **real** logical
//! sector size (`IOCTL_DISK_GET_DRIVE_GEOMETRY_EX`), not a hardcoded 512, and
//! partitions are aligned to it. The last partition is clamped to the last usable
//! LBA. This still benefits from real-hardware validation — see SPEC-00010.

use std::io;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, FALSE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FindFirstVolumeW, FindNextVolumeW,
    FindVolumeClose, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    CREATE_DISK, CREATE_DISK_GPT, DISK_GEOMETRY, DISK_GEOMETRY_EX, DRIVE_LAYOUT_INFORMATION_EX,
    DRIVE_LAYOUT_INFORMATION_GPT, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_DISK_CREATE_DISK,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
    IOCTL_DISK_SET_DRIVE_LAYOUT_EX, IOCTL_DISK_UPDATE_PROPERTIES, PARTITION_INFORMATION_EX,
    PARTITION_INFORMATION_GPT, PARTITION_STYLE_GPT, VOLUME_DISK_EXTENTS,
};
use windows_sys::Win32::System::Rpc::UuidCreate;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::core::GUID;

use crate::{
    GptGeometry, GptPartitionSpec, disk_numbers_from_extents, format_guid_fields, gpt_geometry,
    is_no_media_error, layout_buffer_holds_all, parse_guid_fields, place_partition,
};

const GPT_MAX_PARTITIONS: u32 = 128;
const GENERIC_RW: u32 = 0x8000_0000 | 0x4000_0000; // GENERIC_READ | GENERIC_WRITE

/// An owned Win32 handle, closed exactly once on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a live handle this type uniquely owns; closing once
        // is valid. (Closing a locked volume handle also releases the lock.)
        unsafe { CloseHandle(self.0) };
    }
}

fn open_wide(path: &str, access: u32) -> io::Result<Handle> {
    let wide: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is a valid NUL-terminated UTF-16 string outliving the call;
    // null security-attributes and template handle are permitted.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
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

fn ioctl(handle: HANDLE, code: u32, input: &[u8], out: &mut [u8]) -> io::Result<u32> {
    let mut returned = 0u32;
    let in_ptr = if input.is_empty() {
        std::ptr::null()
    } else {
        input.as_ptr().cast()
    };
    let (out_ptr, out_len) = if out.is_empty() {
        (std::ptr::null_mut(), 0)
    } else {
        (out.as_mut_ptr().cast(), out.len() as u32)
    };
    // SAFETY: `handle` is live; buffers and their lengths are consistent;
    // `returned` is a valid out-pointer; no overlapped I/O.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            code,
            in_ptr,
            input.len() as u32,
            out_ptr,
            out_len,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(returned)
}

fn is_grow_error(e: &io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(code)
            if code == ERROR_MORE_DATA as i32 || code == ERROR_INSUFFICIENT_BUFFER as i32
    )
}

/// Whether the current process is elevated (Administrator).
pub fn is_process_elevated() -> io::Result<bool> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess is a pseudo-handle; OpenProcessToken writes a real
    // token handle into `token` on success.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let _token = Handle(token); // closed on drop
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut ret_len = 0u32;
    // SAFETY: `elevation` is a valid `TOKEN_ELEVATION` of the size passed; the info
    // class matches; `ret_len` is a valid out-pointer.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(elevation.TokenIsElevated != 0)
}

/// Read the current GPT partition **type** GUIDs on a disk (for the ESP guard).
/// An MBR/raw disk yields an empty list.
pub fn read_partition_type_guids(disk_number: u32) -> io::Result<Vec<String>> {
    let disk = open_wide(&format!(r"\\.\PhysicalDrive{disk_number}"), GENERIC_RW)?;
    let header = std::mem::size_of::<DRIVE_LAYOUT_INFORMATION_EX>();
    let entry = std::mem::size_of::<PARTITION_INFORMATION_EX>();
    let entries_off = std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionEntry);

    let mut capacity = 8usize;
    for _ in 0..6 {
        let mut buf = vec![0u8; header + capacity * entry];
        match ioctl(disk.0, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, &[], &mut buf) {
            Ok(_) => {
                // SAFETY: the header fits (buffer >= one DRIVE_LAYOUT_INFORMATION_EX);
                // read style/count unaligned from a byte buffer.
                let style = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<i32>()) };
                if style != PARTITION_STYLE_GPT {
                    return Ok(Vec::new()); // not GPT → no GPT type GUIDs
                }
                let count =
                    unsafe { std::ptr::read_unaligned(buf.as_ptr().add(4).cast::<u32>()) } as usize;
                // Fail CLOSED: a security guard must never read a partial table. If
                // the buffer does not hold every entry, error out (the IOCTL only
                // succeeds once the buffer is large enough, so this is defensive).
                if !layout_buffer_holds_all(buf.len(), entries_off, entry, count) {
                    return Err(io::Error::other("partition table read truncated"));
                }
                let mut out = Vec::with_capacity(count);
                for i in 0..count {
                    let off = entries_off + i * entry;
                    // SAFETY: `off..off+entry` is within the buffer (guaranteed by
                    // the check above); the entry was written by the IOCTL. Read it
                    // unaligned and, since the layout is GPT, read the Gpt union arm.
                    let info = unsafe {
                        std::ptr::read_unaligned(
                            buf.as_ptr().add(off).cast::<PARTITION_INFORMATION_EX>(),
                        )
                    };
                    let gpt: PARTITION_INFORMATION_GPT = unsafe { info.Anonymous.Gpt };
                    out.push(guid_to_string(&gpt.PartitionType));
                }
                return Ok(out);
            }
            Err(e) if is_grow_error(&e) => capacity = capacity.saturating_mul(2).max(2),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("disk has too many partitions to read"))
}

fn guid_to_string(g: &GUID) -> String {
    format_guid_fields(g.data1, g.data2, g.data3, g.data4)
}

fn guid_from_str(s: &str) -> io::Result<GUID> {
    let (d1, d2, d3, d4) =
        parse_guid_fields(s).ok_or_else(|| io::Error::other("invalid GUID string"))?;
    Ok(GUID {
        data1: d1,
        data2: d2,
        data3: d3,
        data4: d4,
    })
}

fn new_guid() -> io::Result<GUID> {
    let mut g = GUID {
        data1: 0,
        data2: 0,
        data3: 0,
        data4: [0; 8],
    };
    // SAFETY: `g` is a valid GUID out-parameter; UuidCreate fills it.
    let status = unsafe { UuidCreate(&mut g) };
    // RPC_S_OK == 0; RPC_S_UUID_LOCAL_ONLY (1824) still yields a usable UUID.
    if status != 0 && status != 1824 {
        return Err(io::Error::other("UuidCreate failed"));
    }
    Ok(g)
}

/// Lock and dismount every volume that lives on `disk_number`. The returned
/// handles keep the volumes locked until dropped (after the layout write).
fn lock_and_dismount_disk_volumes(disk_number: u32) -> io::Result<Vec<Handle>> {
    let mut locked = Vec::new();
    let mut name = [0u16; 260];
    // SAFETY: buffer/len describe `name`; FindFirstVolumeW fills it or fails.
    let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if find == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // Ensure the find handle is always closed.
    let mut result = Ok(());
    loop {
        if let Some(volume_path) = wide_to_string(&name) {
            if volume_backs_disk(&volume_path, disk_number).unwrap_or(false) {
                match lock_dismount_one(&volume_path) {
                    Ok(handle) => locked.push(handle),
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
        }
        // SAFETY: `find` is a live enumeration handle; `name` is a valid buffer.
        let more = unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) };
        if more == 0 {
            break;
        }
    }
    // SAFETY: `find` came from FindFirstVolumeW and is closed exactly once here.
    unsafe { FindVolumeClose(find) };
    result?;
    Ok(locked)
}

/// A volume path from FindFirstVolume is `\\?\Volume{GUID}\`; strip the trailing
/// backslash to open it, query its disk extents, and check for `disk_number`.
///
/// Error handling is fail-**closed**: only a genuine "no media present" error
/// (`ERROR_NOT_READY` / `ERROR_NO_MEDIA_IN_DRIVE` — a legitimate reason a volume,
/// e.g. an empty card reader, has no disk extents) is treated as "does not back
/// this disk". Any other error (access, buffer, …) is **propagated**, so a mounted
/// volume is never silently skipped from the lock/dismount — that would risk
/// filesystem corruption when the table is rewritten.
fn volume_backs_disk(volume_path: &str, disk_number: u32) -> io::Result<bool> {
    let trimmed = volume_path.strip_suffix('\\').unwrap_or(volume_path);
    let handle = open_wide(trimmed, 0)?; // query only, no access
    let base = std::mem::size_of::<VOLUME_DISK_EXTENTS>();
    let mut buf = vec![0u8; base + 32 * std::mem::size_of::<[u8; 24]>()];
    if let Err(e) = ioctl(
        handle.0,
        windows_sys::Win32::Storage::FileSystem::IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
        &[],
        &mut buf,
    ) {
        if is_no_media_error(e.raw_os_error()) {
            return Ok(false); // no media → legitimately no extents
        }
        return Err(e); // real failure → fail closed
    }
    Ok(disk_numbers_from_extents(&buf).contains(&disk_number))
}

fn lock_dismount_one(volume_path: &str) -> io::Result<Handle> {
    let trimmed = volume_path.strip_suffix('\\').unwrap_or(volume_path);
    let handle = open_wide(trimmed, GENERIC_RW)?;
    ioctl(handle.0, FSCTL_LOCK_VOLUME, &[], &mut [])?;
    ioctl(handle.0, FSCTL_DISMOUNT_VOLUME, &[], &mut [])?;
    Ok(handle)
}

fn wide_to_string(buf: &[u16]) -> Option<String> {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Write the GPT layout described by `specs` onto `disk_number` (a disk of
/// `disk_size_bytes`). Locks/dismounts the disk's volumes first and re-reads the
/// table afterward. **Destructive.**
pub fn write_gpt_layout(
    disk_number: u32,
    disk_size_bytes: u64,
    specs: &[GptPartitionSpec],
) -> io::Result<()> {
    // Hold the volume locks for the whole operation.
    let _locks = lock_and_dismount_disk_volumes(disk_number)?;

    let disk = open_wide(&format!(r"\\.\PhysicalDrive{disk_number}"), GENERIC_RW)?;
    let disk_id = new_guid()?;

    // Real logical sector size — fail closed if it cannot be read (never assume 512).
    let geom = gpt_geometry(disk_size_bytes, disk_bytes_per_sector(disk.0)?);

    // 1. Initialize the disk as GPT.
    create_gpt_disk(disk.0, disk_id)?;
    // Let the MSR the driver may create settle before setting our own layout.
    let _ = ioctl(disk.0, IOCTL_DISK_UPDATE_PROPERTIES, &[], &mut []);

    // 2. Write our explicit layout.
    set_gpt_layout(disk.0, disk_id, &geom, specs)?;

    // 3. Make the system re-read the new table (partprobe equivalent).
    ioctl(disk.0, IOCTL_DISK_UPDATE_PROPERTIES, &[], &mut [])?;
    Ok(())
    // `_locks` drop here, unlocking the (now stale) old volumes.
}

fn create_gpt_disk(disk: HANDLE, disk_id: GUID) -> io::Result<()> {
    let create = CREATE_DISK {
        PartitionStyle: PARTITION_STYLE_GPT,
        Anonymous: windows_sys::Win32::System::Ioctl::CREATE_DISK_0 {
            Gpt: CREATE_DISK_GPT {
                DiskId: disk_id,
                MaxPartitionCount: GPT_MAX_PARTITIONS,
            },
        },
    };
    // SAFETY: `create` is a fully-initialized `CREATE_DISK`; view it as its bytes
    // for the input buffer (read-only).
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&create as *const CREATE_DISK).cast::<u8>(),
            std::mem::size_of::<CREATE_DISK>(),
        )
    };
    ioctl(disk, IOCTL_DISK_CREATE_DISK, bytes, &mut [])?;
    Ok(())
}

/// A `DRIVE_LAYOUT_INFORMATION_EX` with room for exactly two partition entries.
#[repr(C)]
struct DriveLayoutTwo {
    info: DRIVE_LAYOUT_INFORMATION_EX, // includes PartitionEntry[0]
    entry_two: PARTITION_INFORMATION_EX,
}

fn set_gpt_layout(
    disk: HANDLE,
    disk_id: GUID,
    geom: &GptGeometry,
    specs: &[GptPartitionSpec],
) -> io::Result<()> {
    if specs.len() != 2 {
        return Err(io::Error::other("expected exactly two partitions"));
    }

    let mut entries = [gpt_entry(&specs[0], geom)?, gpt_entry(&specs[1], geom)?];
    // Number them 1..=2 in table order regardless of the spec's own value.
    entries[0].PartitionNumber = 1;
    entries[1].PartitionNumber = 2;

    let gpt_header = DRIVE_LAYOUT_INFORMATION_GPT {
        DiskId: disk_id,
        StartingUsableOffset: geom.first_usable as i64,
        UsableLength: geom.last_usable_end.saturating_sub(geom.first_usable) as i64,
        MaxPartitionCount: GPT_MAX_PARTITIONS,
    };

    let mut layout = DriveLayoutTwo {
        info: DRIVE_LAYOUT_INFORMATION_EX {
            PartitionStyle: PARTITION_STYLE_GPT as u32,
            PartitionCount: 2,
            Anonymous: windows_sys::Win32::System::Ioctl::DRIVE_LAYOUT_INFORMATION_EX_0 {
                Gpt: gpt_header,
            },
            PartitionEntry: [entries[0]],
        },
        entry_two: entries[1],
    };
    layout.info.PartitionEntry[0] = entries[0];

    // SAFETY: `layout` is a fully-initialized, `#[repr(C)]` two-entry drive layout;
    // view it as its bytes for the input buffer (read-only).
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&layout as *const DriveLayoutTwo).cast::<u8>(),
            std::mem::size_of::<DriveLayoutTwo>(),
        )
    };
    ioctl(disk, IOCTL_DISK_SET_DRIVE_LAYOUT_EX, bytes, &mut [])?;
    Ok(())
}

fn gpt_entry(spec: &GptPartitionSpec, geom: &GptGeometry) -> io::Result<PARTITION_INFORMATION_EX> {
    // Sector-aligned placement within the usable range (clamps the last partition
    // to the last usable LBA — sfdisk did this for us on Linux).
    let (start, length) = place_partition(spec.start_bytes, spec.size_bytes, geom);
    Ok(PARTITION_INFORMATION_EX {
        PartitionStyle: PARTITION_STYLE_GPT,
        StartingOffset: start as i64,
        PartitionLength: length as i64,
        PartitionNumber: spec.partition_number,
        RewritePartition: 1,
        IsServicePartition: FALSE as u8,
        Anonymous: windows_sys::Win32::System::Ioctl::PARTITION_INFORMATION_EX_0 {
            Gpt: PARTITION_INFORMATION_GPT {
                PartitionType: guid_from_str(&spec.type_guid)?,
                PartitionId: new_guid()?,
                Attributes: 0,
                Name: [0u16; 36],
            },
        },
    })
}

/// The disk's logical sector size (`BytesPerSector`) via
/// `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX`. Fails (never assumes 512) if unreadable.
fn disk_bytes_per_sector(disk: HANDLE) -> io::Result<u64> {
    // DISK_GEOMETRY_EX is variable-length (trailing partition/detection info);
    // allocate generously and read only the fixed geometry header.
    let mut buf = vec![0u8; std::mem::size_of::<DISK_GEOMETRY_EX>() + 512];
    ioctl(disk, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut buf)?;
    // SAFETY: the IOCTL succeeded, so the buffer starts with a DISK_GEOMETRY;
    // read it unaligned from the byte buffer.
    let geometry = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<DISK_GEOMETRY>()) };
    let bps = geometry.BytesPerSector;
    if bps == 0 {
        return Err(io::Error::other("disk reported a zero sector size"));
    }
    Ok(u64::from(bps))
}
