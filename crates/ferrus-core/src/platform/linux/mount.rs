//! Linux mounting for the Windows file-copy path (SPEC-0004).
//!
//! ISO is mounted `-o loop,ro` (kernel autodetects iso9660/UDF); the NTFS
//! partition is mounted rw with `ntfs3`, falling back to `ntfs-3g` (ADR-0005).
//! [`LinuxMount`] is a RAII guard: `Drop` unmounts and removes the temp
//! mountpoint even if the copy failed mid-way.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::LinuxBackend;
use crate::platform::{Mount, MountBackend};
use crate::{Error, Result};

/// A mounted filesystem on a temporary directory, torn down on drop.
struct LinuxMount {
    dir: PathBuf,
}

impl Mount for LinuxMount {
    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for LinuxMount {
    fn drop(&mut self) {
        // Best-effort: unmount, lazily if busy, then remove the mountpoint.
        let ok = Command::new("umount")
            .arg(&self.dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = Command::new("umount").arg("-l").arg(&self.dir).status();
        }
        let _ = fs::remove_dir(&self.dir);
    }
}

/// Create a temporary mountpoint directory via `mktemp -d`.
fn make_tempdir() -> Result<PathBuf> {
    let output = Command::new("mktemp")
        .arg("-d")
        .arg("-t")
        .arg("ferrus.XXXXXX")
        .output()
        .map_err(|e| Error::Tool {
            tool: "mktemp".to_owned(),
            reason: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(Error::Tool {
            tool: "mktemp".to_owned(),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    ))
}

impl MountBackend for LinuxBackend {
    fn mount_iso_ro(&self, image: &Path) -> Result<Box<dyn Mount>> {
        let dir = make_tempdir()?;
        let output = Command::new("mount")
            .arg("-o")
            .arg("loop,ro")
            .arg(image)
            .arg(&dir)
            .output()
            .map_err(|e| Error::Tool {
                tool: "mount".to_owned(),
                reason: e.to_string(),
            })?;
        if !output.status.success() {
            let _ = fs::remove_dir(&dir);
            return Err(Error::Tool {
                tool: "mount".to_owned(),
                reason: format!(
                    "failed to mount ISO {}: {}",
                    image.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }
        Ok(Box::new(LinuxMount { dir }))
    }

    fn mount_ntfs_rw(&self, partition: &Path) -> Result<Box<dyn Mount>> {
        let dir = make_tempdir()?;
        // ntfs3 (in-kernel, fast) first, then ntfs-3g (FUSE).
        for fstype in ["ntfs3", "ntfs-3g"] {
            let ok = Command::new("mount")
                .arg("-t")
                .arg(fstype)
                .arg(partition)
                .arg(&dir)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return Ok(Box::new(LinuxMount { dir }));
            }
        }
        let _ = fs::remove_dir(&dir);
        Err(Error::MissingTool {
            name: "ntfs3 or ntfs-3g (NTFS read-write driver)".to_owned(),
        })
    }

    fn sync_path(&self, path: &Path) -> Result<()> {
        // `sync -f` flushes the filesystem containing `path`.
        let status = Command::new("sync")
            .arg("-f")
            .arg(path)
            .status()
            .map_err(|e| Error::Tool {
                tool: "sync".to_owned(),
                reason: e.to_string(),
            })?;
        if !status.success() {
            return Err(Error::Tool {
                tool: "sync".to_owned(),
                reason: format!("sync -f {} failed", path.display()),
            });
        }
        Ok(())
    }
}
