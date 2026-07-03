//! Recursive directory scan and copy, over a [`TreeIo`] seam.
//!
//! `TreeIo` abstracts the filesystem so the recursion is unit tested with an
//! in-memory tree; [`RealTreeIo`] is the `std::fs` implementation used over the
//! real mount points. File contents stream through the block loop
//! ([`copy_reader_to_writer`]).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::stream::copy_reader_to_writer;
use crate::progress::ProgressSink;
use crate::{Error, Result};

/// Bound on directory nesting, a backstop against pathological/looping trees.
const MAX_DEPTH: u32 = 64;

/// One directory entry.
pub(crate) struct TreeEntry {
    /// Absolute path of the entry.
    pub path: PathBuf,
    /// Whether it is a directory.
    pub is_dir: bool,
    /// File size in bytes (0 for directories).
    pub size: u64,
}

/// Filesystem operations the scan/copy need, abstracted for testability.
pub(crate) trait TreeIo {
    /// List the entries directly under `dir`.
    fn read_dir(&self, dir: &Path) -> Result<Vec<TreeEntry>>;
    /// Open `file` for reading.
    fn open_read(&self, file: &Path) -> Result<Box<dyn Read>>;
    /// Create `dir` (and parents).
    fn create_dir(&self, dir: &Path) -> Result<()>;
    /// Open `file` for writing (create/truncate).
    fn open_write(&self, file: &Path) -> Result<Box<dyn Write>>;
    /// Size of `file` in bytes.
    fn file_size(&self, file: &Path) -> Result<u64>;
}

/// The result of scanning a tree.
pub(crate) struct Scan {
    /// Total size of all files, in bytes.
    pub total_bytes: u64,
    /// Every file's relative path (lowercased, `/`-separated) → size, for
    /// Windows-marker detection.
    pub files: BTreeMap<String, u64>,
    /// Lowercased key → the file's original-case relative path, so callers can
    /// locate the real file (mounts are case-sensitive on Linux).
    pub original_of: BTreeMap<String, PathBuf>,
}

/// Recursively scan `root`, returning the total byte size and the relative file
/// set (lowercased) for detection.
///
/// # Errors
///
/// Returns an error if a directory cannot be read, or nesting exceeds the bound.
pub(crate) fn scan(io: &dyn TreeIo, root: &Path) -> Result<Scan> {
    let mut scan = Scan {
        total_bytes: 0,
        files: BTreeMap::new(),
        original_of: BTreeMap::new(),
    };
    scan_dir(io, root, root, &mut scan, 0)?;
    Ok(scan)
}

fn scan_dir(io: &dyn TreeIo, root: &Path, dir: &Path, scan: &mut Scan, depth: u32) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::InvalidSource(
            "directory nesting too deep".to_owned(),
        ));
    }
    for entry in io.read_dir(dir)? {
        if entry.is_dir {
            scan_dir(io, root, &entry.path, scan, depth + 1)?;
        } else {
            let rel = entry
                .path
                .strip_prefix(root)
                .unwrap_or(entry.path.as_path());
            let key = rel_key(rel);
            scan.original_of.insert(key.clone(), rel.to_path_buf());
            scan.files.insert(key, entry.size);
            scan.total_bytes = scan.total_bytes.saturating_add(entry.size);
        }
    }
    Ok(())
}

/// Relative path → a lowercased, `/`-joined key for marker matching.
fn rel_key(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

/// Recursively copy every file and directory under `src_root` into `dst_root`,
/// preserving names, case, and structure. Streams file contents in blocks and
/// reports cumulative progress against `total`. Returns bytes copied.
///
/// # Errors
///
/// Returns an error on the first read/write failure (the caller's RAII guards
/// still unmount).
pub(crate) fn copy_tree(
    io: &dyn TreeIo,
    src_root: &Path,
    dst_root: &Path,
    total: u64,
    progress: &mut dyn ProgressSink,
) -> Result<u64> {
    let mut copied: u64 = 0;
    copy_dir(io, src_root, dst_root, &mut copied, total, progress, 0)?;
    Ok(copied)
}

fn copy_dir(
    io: &dyn TreeIo,
    src_dir: &Path,
    dst_dir: &Path,
    copied: &mut u64,
    total: u64,
    progress: &mut dyn ProgressSink,
    depth: u32,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::InvalidSource(
            "directory nesting too deep".to_owned(),
        ));
    }
    io.create_dir(dst_dir)?;
    for entry in io.read_dir(src_dir)? {
        let name = entry.path.file_name().ok_or_else(|| {
            Error::InvalidSource(format!("entry without a name: {}", entry.path.display()))
        })?;
        let dst = dst_dir.join(name);
        if entry.is_dir {
            copy_dir(io, &entry.path, &dst, copied, total, progress, depth + 1)?;
        } else {
            let mut reader = io.open_read(&entry.path)?;
            let mut writer = io.open_write(&dst)?;
            copy_reader_to_writer(reader.as_mut(), writer.as_mut(), copied, total, progress)?;
        }
    }
    Ok(())
}

/// `std::fs`-backed [`TreeIo`], used over the real mount points.
pub(crate) struct RealTreeIo;

impl TreeIo for RealTreeIo {
    fn read_dir(&self, dir: &Path) -> Result<Vec<TreeEntry>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let (is_dir, size) = if file_type.is_dir() {
                (true, 0)
            } else {
                (false, entry.metadata()?.len())
            };
            entries.push(TreeEntry {
                path: entry.path(),
                is_dir,
                size,
            });
        }
        Ok(entries)
    }

    fn open_read(&self, file: &Path) -> Result<Box<dyn Read>> {
        Ok(Box::new(File::open(file)?))
    }

    fn create_dir(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        Ok(())
    }

    fn open_write(&self, file: &Path) -> Result<Box<dyn Write>> {
        Ok(Box::new(File::create(file)?))
    }

    fn file_size(&self, file: &Path) -> Result<u64> {
        Ok(fs::metadata(file)?.len())
    }
}
