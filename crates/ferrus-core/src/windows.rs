//! Windows install tweaks — **the core differentiator of Ferrus**.
//!
//! These tweaks are **file drops, not binary patching**:
//!
//! - an [`autounattend.xml`](generate_autounattend) placed at the USB root,
//!   which Windows Setup reads automatically, and
//! - **LabConfig** registry keys ([`labconfig_keys`]) that bypass the hardware
//!   requirement checks.
//!
//! # Correctness warning
//!
//! The `autounattend.xml` schema and the exact bypass keys **drift across
//! Windows builds** (22H2 / 24H2 / 25H2). Nothing here may be hardcoded from
//! memory: the generator must be driven by verified, per-build data, and each
//! build's handling must be isolated and unit-tested. See the `TODO(verify)`
//! markers below and `CLAUDE.md` → "Known gotchas".

use crate::Result;

/// User-selected tweaks that parameterize the generated `autounattend.xml` and
/// registry keys. Every field maps to a checkbox in the future UI.
#[derive(Debug, Clone)]
pub struct TweakOptions {
    /// Bypass the TPM 2.0 requirement.
    pub bypass_tpm: bool,
    /// Bypass the Secure Boot requirement.
    pub bypass_secure_boot: bool,
    /// Bypass the minimum RAM requirement.
    pub bypass_ram: bool,
    /// Bypass the storage requirement.
    pub bypass_storage: bool,
    /// Bypass the supported-CPU requirement.
    pub bypass_cpu: bool,
    /// Remove the Microsoft-account requirement during OOBE.
    pub skip_msa: bool,
    /// Create a local account (name is applied when `skip_msa` is set).
    pub local_account: Option<String>,
    /// Disable telemetry / data collection where the answer file allows it.
    pub disable_telemetry: bool,
    /// Disable automatic BitLocker device encryption.
    pub disable_bitlocker: bool,
}

impl Default for TweakOptions {
    /// The neutral default: no tweaks applied. Callers opt in explicitly.
    fn default() -> Self {
        Self {
            bypass_tpm: false,
            bypass_secure_boot: false,
            bypass_ram: false,
            bypass_storage: false,
            bypass_cpu: false,
            skip_msa: false,
            local_account: None,
            disable_telemetry: false,
            disable_bitlocker: false,
        }
    }
}

/// A single LabConfig registry value to inject (under
/// `HKLM\SYSTEM\Setup\LabConfig`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryValue {
    /// Value name, e.g. `BypassTPMCheck`.
    pub name: &'static str,
    /// DWORD value (LabConfig bypasses use `1` to enable).
    pub dword: u32,
}

/// Compute the LabConfig registry values implied by `opts`.
///
/// Returns only the bypasses the user enabled. This is a pure function and is
/// the natural first unit-test target for the module.
///
/// The value **names** (`BypassTPMCheck`, `BypassSecureBootCheck`,
/// `BypassRAMCheck`, `BypassStorageCheck`, `BypassCPUCheck`) are stable enough
/// to enumerate here, but whether each is honored — and how the keys are
/// delivered — depends on the build; see [`generate_autounattend`].
#[must_use]
pub fn labconfig_keys(opts: &TweakOptions) -> Vec<RegistryValue> {
    let mut keys = Vec::new();
    let mut push = |enabled: bool, name: &'static str| {
        if enabled {
            keys.push(RegistryValue { name, dword: 1 });
        }
    };
    push(opts.bypass_tpm, "BypassTPMCheck");
    push(opts.bypass_secure_boot, "BypassSecureBootCheck");
    push(opts.bypass_ram, "BypassRAMCheck");
    push(opts.bypass_storage, "BypassStorageCheck");
    push(opts.bypass_cpu, "BypassCPUCheck");
    keys
}

/// Generate the `autounattend.xml` content for the given tweaks.
///
/// # Errors
///
/// Returns an error if the options are internally inconsistent (e.g. a local
/// account is requested without `skip_msa`).
pub fn generate_autounattend(opts: &TweakOptions) -> Result<String> {
    // TODO(phase4): build the answer file from a verified, per-build template.
    //
    // NOTE(verify): the autounattend.xml passes (windowsPE / specialize /
    // oobeSystem), the exact element names, and how the LabConfig keys are
    // injected (RunSynchronous `reg add` vs offline hive edit) all DRIFT
    // between 22H2 / 24H2 / 25H2. Verify against current sources per build;
    // do NOT emit a hardcoded blob from memory. Keep one template per build,
    // each unit-tested.
    let _ = opts;
    todo!("autounattend.xml generation lands in Phase 4")
}

/// Write the generated tweak files onto the mounted target filesystem
/// (`autounattend.xml` at the root, plus any supporting scripts).
///
/// # Errors
///
/// Returns an error on write failure.
pub fn write_tweaks(dest_mount: &std::path::Path, opts: &TweakOptions) -> Result<()> {
    // TODO(phase4): render `generate_autounattend(opts)` to <root>/autounattend.xml
    // and drop any companion scripts.
    let _ = (dest_mount, opts);
    todo!("tweak file placement lands in Phase 4")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_yield_no_bypass_keys() {
        assert!(labconfig_keys(&TweakOptions::default()).is_empty());
    }

    #[test]
    fn enabled_bypasses_are_emitted_as_dword_one() {
        let opts = TweakOptions {
            bypass_tpm: true,
            bypass_secure_boot: true,
            ..TweakOptions::default()
        };
        let keys = labconfig_keys(&opts);
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().all(|k| k.dword == 1));
        assert!(keys.iter().any(|k| k.name == "BypassTPMCheck"));
        assert!(keys.iter().any(|k| k.name == "BypassSecureBootCheck"));
    }
}
