//! Tests for the UEFI:NTFS bootloader install (Phase 3c).
//!
//! No real device is touched: the write runs against a fake `WriteBackend` with
//! an in-memory sink. A small fake image drives the write/dry-run/integrity/size
//! paths; a separate test enforces that the real embedded asset matches its
//! pinned hash and size (keeping code and asset in sync).

use std::cell::{Cell, RefCell};
use std::io::{Cursor, Read};
use std::path::Path;
use std::rc::Rc;

use super::{
    UEFI_NTFS_IMG, UEFI_NTFS_IMG_SHA256, ensure_fits, install_uefi_ntfs_with, sha256_hex,
    verify_integrity,
};
use crate::Error;
use crate::Result;
use crate::platform::{WriteBackend, WriteSink};
use crate::progress::NullProgress;

// --- asset / pure checks --------------------------------------------------

#[test]
fn embedded_asset_matches_pinned_hash_and_size() {
    assert_eq!(sha256_hex(UEFI_NTFS_IMG), UEFI_NTFS_IMG_SHA256);
    assert_eq!(UEFI_NTFS_IMG.len(), 1_048_576); // exactly 1 MiB
}

#[test]
fn verify_integrity_accepts_correct_and_rejects_mismatch() {
    assert!(verify_integrity(UEFI_NTFS_IMG, UEFI_NTFS_IMG_SHA256).is_ok());
    let err = verify_integrity(b"not the image", UEFI_NTFS_IMG_SHA256).unwrap_err();
    assert!(matches!(err, Error::BootloaderIntegrity { .. }));
}

#[test]
fn ensure_fits_bounds() {
    assert!(ensure_fits(100, 100).is_ok());
    assert!(matches!(
        ensure_fits(101, 100),
        Err(Error::ImageExceedsPartition { .. })
    ));
}

// --- write orchestration --------------------------------------------------

struct MemSink {
    data: Rc<RefCell<Vec<u8>>>,
    synced: Rc<Cell<bool>>,
}

impl WriteSink for MemSink {
    fn write_chunk(&mut self, buf: &[u8]) -> Result<()> {
        self.data.borrow_mut().extend_from_slice(buf);
        Ok(())
    }
    fn sync(&mut self) -> Result<()> {
        self.synced.set(true);
        Ok(())
    }
}

struct FakeWriteBackend {
    written: Rc<RefCell<Vec<u8>>>,
    synced: Rc<Cell<bool>>,
    opened: Cell<bool>,
}

impl FakeWriteBackend {
    fn new() -> Self {
        Self {
            written: Rc::new(RefCell::new(Vec::new())),
            synced: Rc::new(Cell::new(false)),
            opened: Cell::new(false),
        }
    }
}

impl WriteBackend for FakeWriteBackend {
    fn effective_uid(&self) -> Result<u32> {
        Ok(0)
    }
    fn is_system_or_critical(&self, _device_path: &Path) -> Result<bool> {
        Ok(false)
    }
    fn mounted_partitions(&self, _device_path: &Path) -> Result<Vec<std::path::PathBuf>> {
        Ok(Vec::new())
    }
    fn unmount(&self, _mountpoint: &Path) -> Result<()> {
        Ok(())
    }
    fn open_exclusive_writer(&self, _device_path: &Path) -> Result<Box<dyn WriteSink>> {
        self.opened.set(true);
        Ok(Box::new(MemSink {
            data: self.written.clone(),
            synced: self.synced.clone(),
        }))
    }
    fn open_reader(&self, _device_path: &Path) -> Result<Box<dyn Read>> {
        Ok(Box::new(Cursor::new(self.written.borrow().clone())))
    }
}

#[test]
fn install_writes_image_and_fsyncs() {
    let img = b"UEFI-NTFS-FAKE-IMAGE".as_slice();
    let sha = sha256_hex(img);
    let backend = FakeWriteBackend::new();
    let mut progress = NullProgress;

    install_uefi_ntfs_with(
        img,
        &sha,
        Path::new("/dev/sda2"),
        1_000_000,
        false,
        &mut progress,
        &backend,
    )
    .unwrap();

    assert_eq!(*backend.written.borrow(), img);
    assert!(backend.synced.get(), "must fsync before success");
}

#[test]
fn install_dry_run_writes_nothing() {
    let img = b"UEFI-NTFS-FAKE-IMAGE".as_slice();
    let sha = sha256_hex(img);
    let backend = FakeWriteBackend::new();
    let mut progress = NullProgress;

    install_uefi_ntfs_with(
        img,
        &sha,
        Path::new("/dev/sda2"),
        1_000_000,
        true,
        &mut progress,
        &backend,
    )
    .unwrap();

    assert!(!backend.opened.get());
    assert!(backend.written.borrow().is_empty());
    assert!(!backend.synced.get());
}

#[test]
fn install_rejects_tampered_hash_without_writing() {
    let img = b"UEFI-NTFS-FAKE-IMAGE".as_slice();
    let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
    let backend = FakeWriteBackend::new();
    let mut progress = NullProgress;

    let err = install_uefi_ntfs_with(
        img,
        wrong,
        Path::new("/dev/sda2"),
        1_000_000,
        false,
        &mut progress,
        &backend,
    )
    .unwrap_err();

    assert!(matches!(err, Error::BootloaderIntegrity { .. }));
    assert!(
        !backend.opened.get(),
        "must not open the device on a hash mismatch"
    );
    assert!(backend.written.borrow().is_empty());
}

#[test]
fn install_rejects_oversized_image_without_writing() {
    let img = b"UEFI-NTFS-FAKE-IMAGE".as_slice(); // 20 bytes
    let sha = sha256_hex(img);
    let backend = FakeWriteBackend::new();
    let mut progress = NullProgress;

    let err = install_uefi_ntfs_with(
        img,
        &sha,
        Path::new("/dev/sda2"),
        3, // partition smaller than the image
        false,
        &mut progress,
        &backend,
    )
    .unwrap_err();

    assert!(matches!(err, Error::ImageExceedsPartition { .. }));
    assert!(!backend.opened.get());
}
