//! Source image handling.
//!
//! Two concerns: a [`RawImage`] treats the image as an opaque byte stream for
//! the raw-copy path (validate it exists, is readable, non-empty; expose size +
//! reader), and [`detect_windows_install`] classifies a *mounted* ISO as Windows
//! install media by its real marker files (used by the Phase 3b copy).

use std::fs::File;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// A validated source image, treated as an opaque byte stream for raw copy.
///
/// Construct with [`RawImage::open`]; it guarantees the file exists, is
/// readable, and has a non-zero size.
#[derive(Debug, Clone)]
pub struct RawImage {
    path: PathBuf,
    size_bytes: u64,
}

impl RawImage {
    /// Validate and describe an image for raw copy.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSource`] if the path is not a regular file or is
    /// empty, or [`Error::Io`] if its metadata cannot be read.
    pub fn open(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(Error::InvalidSource(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
        let size_bytes = meta.len();
        if size_bytes == 0 {
            return Err(Error::InvalidSource(format!(
                "{} is empty",
                path.display()
            )));
        }
        Ok(Self {
            path: path.to_path_buf(),
            size_bytes,
        })
    }

    /// Path to the image.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Size of the image in bytes.
    #[must_use]
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Open a fresh reader over the image contents.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be opened.
    pub fn open_reader(&self) -> Result<File> {
        Ok(File::open(&self.path)?)
    }
}

/// The install image found in a recognized Windows ISO tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsInstall {
    /// Relative path (lowercase, `/`-separated) of `install.wim` or
    /// `install.esd` within the ISO.
    pub install_image: String,
    /// Size of that install image in bytes.
    pub install_image_bytes: u64,
}

/// Decide whether a mounted image is a Windows install ISO, from the set of its
/// relative file paths (lowercased, `/`-separated) mapped to their sizes.
///
/// Recognition is by **real markers**, never by file name/extension:
/// `sources/install.wim` or `sources/install.esd`, plus `bootmgr` and
/// `efi/boot/bootx64.efi` (matched case-insensitively — callers lowercase the
/// keys). Returns the install image when recognized, `None` otherwise.
///
/// Pure and unit tested.
#[must_use]
pub fn detect_windows_install(
    files: &std::collections::BTreeMap<String, u64>,
) -> Option<WindowsInstall> {
    let install = if files.contains_key("sources/install.wim") {
        "sources/install.wim"
    } else if files.contains_key("sources/install.esd") {
        "sources/install.esd"
    } else {
        return None;
    };
    if files.contains_key("bootmgr") && files.contains_key("efi/boot/bootx64.efi") {
        Some(WindowsInstall {
            install_image: install.to_owned(),
            install_image_bytes: files[install],
        })
    } else {
        None
    }
}

