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
            return Err(Error::InvalidSource(format!("{} is empty", path.display())));
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

/// A preliminary, **non-authoritative** guess at what an image is — made without
/// mounting and without privileges (see SPEC-0007).
///
/// This is only a UI hint. The authority is [`detect_windows_install`] on the
/// *mounted* ISO at write time. The two can disagree on an edge case (Windows
/// structure markers present but no `install.wim`); that is acceptable — the
/// write arbitrates with [`Error::NotWindowsMedia`](crate::Error::NotWindowsMedia).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaKind {
    /// Windows install-media structure markers were found.
    Windows,
    /// The image was readable but is not Windows install media.
    Generic,
    /// Could not be determined (unreadable, not UDF, or an I/O error). This is
    /// **not** a synonym for `Generic`: we simply do not know.
    #[default]
    Unknown,
}

/// Classify a UDF root directory from its entry names. Pure, unit-tested.
///
/// Windows install media carries `bootmgr`, an `efi` directory and a `sources`
/// directory at the root — structure markers that live in the **UDF** layer of
/// modern Windows ISOs. `install.wim` is deliberately **not** part of the
/// criterion: it is UDF-only and huge, and it is a *copy* concern (Phase 3b), not
/// a *detection* one — keying on it would false-negative the main case (SPEC-0007).
fn classify_root_entries(names: &[String]) -> MediaKind {
    let has = |marker: &str| names.iter().any(|n| n.eq_ignore_ascii_case(marker));
    if has("bootmgr") && has("sources") && has("efi") {
        MediaKind::Windows
    } else {
        MediaKind::Generic
    }
}

/// Preliminary, unprivileged, no-mount guess at whether `path` is Windows install
/// media — a **hint** for the GUI (SPEC-0007), never the authority.
///
/// Modern Windows ISOs ship their real tree in **UDF** (their ISO9660 layer is a
/// stub), so this reads the UDF layer read-only, lists the root directory, and
/// classifies by structure markers. Any failure to read (not UDF, not an image,
/// I/O error) yields [`MediaKind::Unknown`] — never a false `Generic`.
#[must_use]
pub fn inspect_iso_kind(path: &Path) -> MediaKind {
    let Ok(file) = File::open(path) else {
        return MediaKind::Unknown;
    };
    let Ok(udf) = hadris_udf::UdfFs::open(std::io::BufReader::new(file)) else {
        return MediaKind::Unknown;
    };
    let Ok(root) = udf.root_dir() else {
        return MediaKind::Unknown;
    };
    let names: Vec<String> = root
        .entries()
        .map(|entry| entry.name().to_string())
        .collect();
    classify_root_entries(&names)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{MediaKind, classify_root_entries, inspect_iso_kind};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn windows_structure_markers_classify_as_windows() {
        let root = names(&[
            "bootmgr",
            "bootmgr.efi",
            "efi",
            "sources",
            "setup.exe",
            "boot",
        ]);
        assert_eq!(classify_root_entries(&root), MediaKind::Windows);
    }

    #[test]
    fn classification_is_case_insensitive() {
        let root = names(&["BOOTMGR", "EFI", "SOURCES"]);
        assert_eq!(classify_root_entries(&root), MediaKind::Windows);
    }

    #[test]
    fn missing_a_marker_is_generic() {
        // A generic bootable image: EFI + boot dir, but no bootmgr/sources.
        let root = names(&["efi", "boot", "casper", "isolinux", "pool"]);
        assert_eq!(classify_root_entries(&root), MediaKind::Generic);
        // install.wim alone must NOT drive detection (UDF-trap guard).
        assert_eq!(
            classify_root_entries(&names(&["sources"])),
            MediaKind::Generic
        );
    }

    #[test]
    fn unreadable_or_non_udf_is_unknown_not_generic() {
        // Non-existent path.
        assert_eq!(
            inspect_iso_kind(std::path::Path::new("/no/such/file.iso")),
            MediaKind::Unknown
        );
        // A real file that is not a UDF image → Unknown, never a false Generic.
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"definitely not a UDF filesystem, just some bytes")
            .unwrap();
        assert_eq!(inspect_iso_kind(f.path()), MediaKind::Unknown);
    }
}
