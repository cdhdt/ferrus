//! Per-Windows-build recipes. Build-specific content lives here (isolated from
//! the generator) so per-build drift is contained — see SPEC-0006. Each profile
//! carries its sources in the doc comments.

/// A build-specific recipe the generator assembles with [`WindowsTweaks`]
/// (`super::WindowsTweaks`).
#[derive(Debug, Clone, Copy)]
pub struct BuildProfile {
    /// Stable identifier, e.g. `win11-25h2`.
    pub id: &'static str,
    /// LabConfig value names (under `HKLM\SYSTEM\Setup\LabConfig`) to set to 1
    /// in the windowsPE pass for the hardware bypass.
    pub labconfig_keys: &'static [&'static str],
    /// Whether to add the `BypassNRO` registry complement (specialize pass) when
    /// a local account is requested.
    pub bypass_nro_complement: bool,
}

/// Windows 11 **25H2** profile.
///
/// Sources (verified 2026-07-03):
/// - LabConfig `Bypass{TPM,SecureBoot,RAM,Storage,CPU}Check` under
///   `HKLM\SYSTEM\Setup\LabConfig` — community clean-install method (elevenforum,
///   iamroot.it, woshub); unchanged across 22H2/24H2.
/// - BypassNRO registry complement — `bypassnro.cmd` removed in 24H2/25H2 but the
///   `…\CurrentVersion\OOBE\BypassNRO` DWORD still works (BleepingComputer,
///   Microsoft Community Hub, 2025).
pub const WIN11_25H2: BuildProfile = BuildProfile {
    id: "win11-25h2",
    labconfig_keys: &[
        "BypassTPMCheck",
        "BypassSecureBootCheck",
        "BypassRAMCheck",
        "BypassStorageCheck",
        "BypassCPUCheck",
    ],
    bypass_nro_complement: true,
};

/// The default build profile Ferrus targets.
#[must_use]
pub fn default_profile() -> &'static BuildProfile {
    &WIN11_25H2
}
