//! Source (ISO) inspection.
//!
//! Before writing anything, Ferrus inspects the source image to decide the
//! strategy: a generic ISO can often be copied as-is, whereas a Windows install
//! ISO needs the NTFS + UEFI:NTFS treatment and enables the Windows tweaks.

use std::path::{Path, PathBuf};

use crate::Result;

/// What kind of source image Ferrus is dealing with.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceKind {
    /// A Windows installation ISO (contains `sources/install.wim` or `.esd`).
    Windows,
    /// Any other bootable image; typically written with a generic strategy.
    Generic,
}

/// The result of inspecting a source image.
#[derive(Debug, Clone)]
pub struct SourceInfo {
    /// Path to the inspected image.
    pub path: PathBuf,
    /// Detected kind of the image.
    pub kind: SourceKind,
    /// For Windows images, the size in bytes of the largest install image
    /// (`install.wim`/`install.esd`). This is what forces NTFS when it exceeds
    /// the FAT32 4 GB per-file limit.
    pub install_image_bytes: Option<u64>,
}

impl SourceInfo {
    /// Whether the main partition must be NTFS because a single file exceeds the
    /// FAT32 4 GiB per-file limit.
    #[must_use]
    pub fn requires_ntfs(&self) -> bool {
        const FAT32_MAX_FILE: u64 = 4 * 1024 * 1024 * 1024 - 1;
        self.install_image_bytes
            .is_some_and(|bytes| bytes > FAT32_MAX_FILE)
    }
}

/// Inspect an image file and classify it.
///
/// # Errors
///
/// Returns an error if the image cannot be opened or its layout cannot be read.
pub fn inspect(image: &Path) -> Result<SourceInfo> {
    // TODO(phase2/3): mount or parse the ISO9660/UDF filesystem, detect Windows
    // layout (`sources/install.wim` or `install.esd`), and measure the install
    // image size. Do not guess from the file name.
    let _ = image;
    todo!("source inspection lands in Phase 2/3")
}
