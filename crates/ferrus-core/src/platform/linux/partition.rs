//! Linux implementation of the partition + format path (SPEC-0003).
//!
//! Partitioning uses `sfdisk` (ADR-0004), re-read uses `partprobe`, and
//! formatting uses `mkfs.ntfs` / `mkfs.vfat`. The privileged/mount helpers are
//! reused from [`super::write`]. The pure `mkfs` command builder is unit tested
//! in `tests.rs`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use super::disk_is_system_or_critical;
use super::write::{list_mounted_partitions, read_effective_uid, run_umount};
use super::LinuxBackend;
use crate::partition::{FsKind, partition_path};
use crate::platform::PartitionBackend;
use crate::{Error, Result};

/// Whether an executable named `name` is found on `PATH` or in the common
/// sbin/bin locations (mkfs/sfdisk usually live in `/usr/sbin`, which is not
/// always on a non-login `PATH`).
fn tool_exists(name: &str) -> bool {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join(name).is_file() {
                return true;
            }
        }
    }
    ["/usr/sbin", "/sbin", "/usr/local/sbin", "/usr/bin", "/bin"]
        .iter()
        .any(|dir| Path::new(dir).join(name).is_file())
}

/// Build the `mkfs` command (program + args) for a filesystem kind. Pure, so it
/// is unit tested. NTFS is quick-formatted; FAT gets a volume label.
fn mkfs_command(fs: FsKind, partition_path: &Path, label: &str) -> (&'static str, Vec<String>) {
    let path = partition_path.to_string_lossy().into_owned();
    match fs {
        FsKind::Ntfs => (
            "mkfs.ntfs",
            vec![
                "--quick".to_owned(),
                "--label".to_owned(),
                label.to_owned(),
                path,
            ],
        ),
        FsKind::Fat => (
            "mkfs.vfat",
            vec!["-n".to_owned(), label.to_owned(), path],
        ),
    }
}

/// Run a command to completion, mapping a spawn/exec failure or a non-zero exit
/// to [`Error::Tool`].
fn run_tool(program: &str, args: &[String]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| Error::Tool {
            tool: program.to_owned(),
            reason: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(Error::Tool {
            tool: program.to_owned(),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

impl PartitionBackend for LinuxBackend {
    fn effective_uid(&self) -> Result<u32> {
        read_effective_uid()
    }

    fn is_system_or_critical(&self, device_path: &Path) -> Result<bool> {
        Ok(disk_is_system_or_critical(device_path))
    }

    fn mounted_partitions(&self, device_path: &Path) -> Result<Vec<PathBuf>> {
        list_mounted_partitions(device_path)
    }

    fn unmount(&self, mountpoint: &Path) -> Result<()> {
        run_umount(mountpoint)
    }

    fn ensure_tools(&self, tools: &[&str]) -> Result<()> {
        for tool in tools {
            if !tool_exists(tool) {
                return Err(Error::MissingTool {
                    name: (*tool).to_owned(),
                });
            }
        }
        Ok(())
    }

    fn write_partition_table(&self, device_path: &Path, script: &str) -> Result<()> {
        // `--wipe always` erases any existing filesystem/partition signatures on
        // the device before writing the new GPT.
        let mut child = Command::new("sfdisk")
            .arg("--wipe")
            .arg("always")
            .arg(device_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Tool {
                tool: "sfdisk".to_owned(),
                reason: e.to_string(),
            })?;

        // Feed the script on stdin, then close it so sfdisk can proceed.
        {
            let stdin = child.stdin.take().ok_or_else(|| Error::Tool {
                tool: "sfdisk".to_owned(),
                reason: "could not open stdin".to_owned(),
            })?;
            let mut stdin = stdin;
            stdin.write_all(script.as_bytes())?;
        }

        let output = child.wait_with_output().map_err(|e| Error::Tool {
            tool: "sfdisk".to_owned(),
            reason: e.to_string(),
        })?;
        if !output.status.success() {
            return Err(Error::Tool {
                tool: "sfdisk".to_owned(),
                reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(())
    }

    fn reread_partition_table(&self, device_path: &Path) -> Result<()> {
        run_tool("partprobe", &[device_path.to_string_lossy().into_owned()])
    }

    fn wait_for_partitions(&self, device_path: &Path, count: usize) -> Result<Vec<PathBuf>> {
        let expected: Vec<PathBuf> = (1..=count).map(|i| partition_path(device_path, i)).collect();

        // Nudge udev, then poll for the nodes (bounded ~5 s).
        let _ = Command::new("udevadm")
            .arg("settle")
            .arg("--timeout=5")
            .status();
        for _ in 0..50 {
            if expected.iter().all(|p| p.exists()) {
                return Ok(expected);
            }
            sleep(Duration::from_millis(100));
        }
        Err(Error::PartitionNodesMissing {
            device: device_path.to_path_buf(),
            expected: count,
        })
    }

    fn make_filesystem(&self, partition_path: &Path, fs: FsKind, label: &str) -> Result<()> {
        let (program, args) = mkfs_command(fs, partition_path, label);
        run_tool(program, &args)
    }
}

#[cfg(test)]
mod tests {
    use super::mkfs_command;
    use crate::partition::FsKind;
    use std::path::Path;

    #[test]
    fn ntfs_command_is_quick_with_label() {
        let (prog, args) = mkfs_command(FsKind::Ntfs, Path::new("/dev/sda1"), "Windows");
        assert_eq!(prog, "mkfs.ntfs");
        assert_eq!(args, ["--quick", "--label", "Windows", "/dev/sda1"]);
    }

    #[test]
    fn fat_command_sets_label() {
        let (prog, args) = mkfs_command(FsKind::Fat, Path::new("/dev/sda2"), "UEFI_NTFS");
        assert_eq!(prog, "mkfs.vfat");
        assert_eq!(args, ["-n", "UEFI_NTFS", "/dev/sda2"]);
    }
}
