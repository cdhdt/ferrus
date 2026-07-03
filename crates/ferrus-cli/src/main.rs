//! Ferrus command-line front-end.
//!
//! Phase 0 is a real skeleton: argument parsing is complete, `--dry-run`
//! genuinely works (it resolves and prints the plan, exercising the implemented
//! parts of `ferrus-core`), and destructive operations are refused with a clear
//! message rather than silently doing nothing. As the engine phases land, the
//! `write` command is fleshed out behind the same interface.

use std::io::Write;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use ferrus_core::copy::raw_copy;
use ferrus_core::device::{
    LARGE_TARGET_THRESHOLD_BYTES, SafeTarget, format_size, list_all_devices,
    list_writable_candidates,
};
use ferrus_core::partition::prepare_windows;
use ferrus_core::progress::{ProgressSink, Stage};
use ferrus_core::source::RawImage;
use ferrus_core::windows::{LocalAccountSpec, WindowsTweaks};

/// Cross-platform bootable USB creator.
#[derive(Debug, Parser)]
#[command(name = "ferrus", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List removable devices that are plausible write targets.
    List(ListArgs),
    /// Raw byte-for-byte write of an already-bootable image to a USB device.
    Write(WriteArgs),
    /// Build a Windows install stick: partition + format, copy the ISO, install
    /// the UEFI:NTFS bootloader, and (opt-in) drop autounattend.xml tweaks.
    PrepareWindows(PrepareArgs),
}

#[derive(Debug, Args)]
struct PrepareArgs {
    /// Target device path (e.g. /dev/sdb). Must be typed exactly.
    #[arg(long, value_name = "DEV")]
    target: std::path::PathBuf,

    /// Windows install ISO to copy onto the NTFS partition. Omit to only
    /// partition + format (3a).
    #[arg(long, value_name = "FILE")]
    image: Option<std::path::PathBuf>,

    /// After copying, verify the install image size on the stick.
    #[arg(long)]
    verify: bool,

    /// Bypass the Windows 11 hardware checks (TPM / Secure Boot / RAM / storage
    /// / CPU) via autounattend.xml. Requires --image.
    #[arg(long)]
    bypass_hardware: bool,

    /// Create a local account with this name (no Microsoft account). Requires
    /// --image.
    #[arg(long, value_name = "NAME")]
    local_account: Option<String>,

    /// Password for the local account (optional). Never logged; stored only,
    /// obfuscated, inside autounattend.xml.
    #[arg(long, value_name = "PASS")]
    local_password: Option<String>,

    /// Describe the plan without touching any device.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct ListArgs {
    /// Also show large transport-removable volumes (e.g. USB SSDs / backup
    /// disks) that are hidden by default.
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Args)]
struct WriteArgs {
    /// Path to the source ISO image.
    #[arg(long, value_name = "FILE")]
    image: std::path::PathBuf,

    /// Target device path (e.g. /dev/sdb). Must be typed exactly; Ferrus never
    /// guesses a target.
    #[arg(long, value_name = "DEV")]
    target: std::path::PathBuf,

    /// Describe what would happen without touching any device.
    #[arg(long)]
    dry_run: bool,

    /// After writing, read the device back and compare it to the image.
    #[arg(long)]
    verify: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List(args) => cmd_list(&args),
        Command::Write(args) => cmd_write(&args),
        Command::PrepareWindows(args) => cmd_prepare_windows(&args),
    }
}

fn cmd_prepare_windows(args: &PrepareArgs) -> Result<()> {
    // Validate the ISO up front, if one was given.
    let image = match &args.image {
        Some(path) => Some(
            RawImage::open(path).with_context(|| format!("cannot use image {}", path.display()))?,
        ),
        None => None,
    };

    // Build the Windows tweaks (Phase 4). The password is never echoed.
    if args.local_password.is_some() && args.local_account.is_none() {
        bail!("--local-password requires --local-account");
    }
    let tweaks = WindowsTweaks {
        bypass_hardware: args.bypass_hardware,
        local_account: args.local_account.as_ref().map(|name| LocalAccountSpec {
            name: name.clone(),
            password: args.local_password.clone(),
        }),
    };
    if tweaks.any() && image.is_none() {
        bail!("Windows tweaks (--bypass-hardware / --local-account) require --image");
    }
    let tweaks_opt = if tweaks.any() { Some(&tweaks) } else { None };

    let device = list_all_devices()
        .context("failed to enumerate devices")?
        .into_iter()
        .find(|dev| dev.path == args.target)
        .ok_or_else(|| {
            anyhow!(
                "{} is not a block device on this host (run `ferrus list`)",
                args.target.display()
            )
        })?;
    let target = SafeTarget::acquire(device, &args.target, args.dry_run)
        .context("target rejected by the safety checkpoint")?;

    println!("Ferrus prepare-windows");
    println!(
        "  target : {} ({})",
        target.device().path.display(),
        format_size(target.device().size_bytes),
    );
    match &image {
        Some(img) => println!(
            "  image  : {} ({})",
            img.path().display(),
            format_size(img.size_bytes())
        ),
        None => println!("  image  : none (partition + format only)"),
    }
    println!(
        "  mode   : {}{}",
        if target.is_dry_run() {
            "dry-run"
        } else {
            "REAL (destructive)"
        },
        if args.verify { ", verify" } else { "" },
    );
    if tweaks.any() {
        let mut parts = Vec::new();
        if tweaks.bypass_hardware {
            parts.push("hardware bypass".to_owned());
        }
        if let Some(account) = &tweaks.local_account {
            parts.push(format!(
                "local account '{}'{}",
                account.name,
                if account.password.is_some() {
                    " (with password)"
                } else {
                    ""
                },
            ));
        }
        println!("  tweaks : {}", parts.join(", "));
    } else {
        println!("  tweaks : none");
    }

    let mut progress = CliProgress;
    prepare_windows(
        &target,
        image.as_ref(),
        tweaks_opt,
        args.verify,
        &mut progress,
    )
    .context("prepare-windows failed")?;
    Ok(())
}

fn cmd_list(args: &ListArgs) -> Result<()> {
    let devices = list_writable_candidates(args.all).context("failed to enumerate devices")?;

    // How many plausible targets were hidden purely by the size heuristic.
    let hidden = if args.all {
        0
    } else {
        list_writable_candidates(true)
            .map(|all| all.len().saturating_sub(devices.len()))
            .unwrap_or(0)
    };

    if devices.is_empty() {
        println!("No removable target devices found.");
    } else {
        println!("{:<14} {:>9}  {:<6} MODEL", "DEVICE", "SIZE", "BUS");
        for dev in &devices {
            println!(
                "{:<14} {:>9}  {:<6} {}",
                dev.path.display(),
                format_size(dev.size_bytes),
                dev.bus,
                dev.model.as_deref().unwrap_or("(unknown model)"),
            );
            if let Some(id) = &dev.stable_id {
                println!("{:<14} by-id: {id}", "");
            }
        }
    }

    if hidden > 0 {
        println!(
            "\n{hidden} large volume(s) over {} hidden; pass --all to include them.",
            format_size(LARGE_TARGET_THRESHOLD_BYTES),
        );
    }
    Ok(())
}

fn cmd_write(args: &WriteArgs) -> Result<()> {
    // Validate the image up front (opaque byte stream — no ISO parsing yet).
    let image = RawImage::open(&args.image)
        .with_context(|| format!("cannot use image {}", args.image.display()))?;

    // Route the requested target through the single safety checkpoint. Even a
    // rejected device is fed to `acquire` so the user gets the precise reason.
    let device = list_all_devices()
        .context("failed to enumerate devices")?
        .into_iter()
        .find(|dev| dev.path == args.target)
        .ok_or_else(|| {
            anyhow!(
                "{} is not a block device on this host (run `ferrus list`)",
                args.target.display()
            )
        })?;
    let target = SafeTarget::acquire(device, &args.target, args.dry_run)
        .context("target rejected by the safety checkpoint")?;
    let device = target.device();

    println!("Ferrus raw write");
    println!(
        "  source : {} ({})",
        image.path().display(),
        format_size(image.size_bytes())
    );
    println!(
        "  target : {} ({}, {})",
        device.path.display(),
        format_size(device.size_bytes),
        device.model.as_deref().unwrap_or("unknown model"),
    );
    println!(
        "  mode   : {}{}",
        if target.is_dry_run() {
            "dry-run"
        } else {
            "REAL WRITE"
        },
        if args.verify { ", verify" } else { "" },
    );

    let mut progress = CliProgress;
    raw_copy(&image, &target, args.verify, &mut progress).context("write failed")?;
    Ok(())
}

/// Minimal terminal progress renderer.
#[derive(Default)]
struct CliProgress;

impl ProgressSink for CliProgress {
    fn stage(&mut self, stage: Stage) {
        println!("[{stage:?}]");
    }

    fn advance(&mut self, done: u64, total: Option<u64>) {
        let Some(total) = total.filter(|t| *t > 0) else {
            return;
        };
        let pct = done.saturating_mul(100) / total;
        print!(
            "\r  {pct:3}%  {} / {}",
            format_size(done),
            format_size(total)
        );
        let _ = std::io::stdout().flush();
        if done >= total {
            println!();
        }
    }

    fn message(&mut self, text: &str) {
        println!("  {text}");
    }
}
