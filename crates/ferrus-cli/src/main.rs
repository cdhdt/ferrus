//! Ferrus command-line front-end.
//!
//! Phase 0 is a real skeleton: argument parsing is complete, `--dry-run`
//! genuinely works (it resolves and prints the plan, exercising the implemented
//! parts of `ferrus-core`), and destructive operations are refused with a clear
//! message rather than silently doing nothing. As the engine phases land, the
//! `write` command is fleshed out behind the same interface.

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use ferrus_core::device::{SafeTarget, format_size, list_all_devices, list_writable_candidates};
use ferrus_core::windows::{TweakOptions, labconfig_keys};

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
    List,
    /// Write an image to a USB device (optionally with Windows tweaks).
    Write(WriteArgs),
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

    #[command(flatten)]
    tweaks: TweakArgs,
}

/// Windows install tweaks (ignored for non-Windows images).
#[derive(Debug, Args)]
struct TweakArgs {
    /// Bypass the TPM 2.0 check.
    #[arg(long)]
    bypass_tpm: bool,
    /// Bypass the Secure Boot check.
    #[arg(long)]
    bypass_secure_boot: bool,
    /// Bypass the minimum RAM check.
    #[arg(long)]
    bypass_ram: bool,
    /// Bypass the storage check.
    #[arg(long)]
    bypass_storage: bool,
    /// Bypass the supported-CPU check.
    #[arg(long)]
    bypass_cpu: bool,
    /// Skip the Microsoft-account requirement during setup.
    #[arg(long)]
    skip_msa: bool,
    /// Create a local account with this name (implies --skip-msa).
    #[arg(long, value_name = "NAME")]
    local_account: Option<String>,
    /// Disable telemetry where the answer file allows it.
    #[arg(long)]
    disable_telemetry: bool,
    /// Disable automatic BitLocker device encryption.
    #[arg(long)]
    disable_bitlocker: bool,
}

impl From<&TweakArgs> for TweakOptions {
    fn from(a: &TweakArgs) -> Self {
        Self {
            bypass_tpm: a.bypass_tpm,
            bypass_secure_boot: a.bypass_secure_boot,
            bypass_ram: a.bypass_ram,
            bypass_storage: a.bypass_storage,
            bypass_cpu: a.bypass_cpu,
            // A local account requires skipping the MS-account step.
            skip_msa: a.skip_msa || a.local_account.is_some(),
            local_account: a.local_account.clone(),
            disable_telemetry: a.disable_telemetry,
            disable_bitlocker: a.disable_bitlocker,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => cmd_list(),
        Command::Write(args) => cmd_write(&args),
    }
}

fn cmd_list() -> Result<()> {
    let devices = list_writable_candidates().context("failed to enumerate devices")?;
    if devices.is_empty() {
        println!("No removable target devices found.");
        return Ok(());
    }

    println!("{:<14} {:>9}  {:<6} {}", "DEVICE", "SIZE", "BUS", "MODEL");
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
    Ok(())
}

fn cmd_write(args: &WriteArgs) -> Result<()> {
    let opts = TweakOptions::from(&args.tweaks);

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

    println!("Ferrus write plan");
    println!("  source : {}", args.image.display());
    println!(
        "  target : {} ({}, {})",
        device.path.display(),
        format_size(device.size_bytes),
        device.model.as_deref().unwrap_or("unknown model"),
    );
    println!(
        "  mode   : {}",
        if target.is_dry_run() {
            "dry-run"
        } else {
            "REAL WRITE"
        }
    );

    let keys = labconfig_keys(&opts);
    if keys.is_empty() {
        println!("  tweaks : none");
    } else {
        println!("  tweaks : LabConfig bypass keys:");
        for key in &keys {
            println!("           {} = {}", key.name, key.dword);
        }
    }
    if let Some(name) = &opts.local_account {
        println!("           local account: {name}");
    }

    if target.is_dry_run() {
        println!("\nDry-run: no device was touched.");
        return Ok(());
    }

    // Guard: real writes are not implemented, so we must not pretend to work.
    // This keeps the interface honest until the engine phases land.
    bail!(
        "real writes are not implemented yet (Phases 2-4). Re-run with --dry-run \
         to preview the plan."
    );
}
