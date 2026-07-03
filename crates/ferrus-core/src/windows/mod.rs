//! Windows install tweaks — **the core differentiator of Ferrus** (Phase 4 + 4.x).
//!
//! Generates an `autounattend.xml` from a [`WindowsTweaks`] model + a
//! [`BuildProfile`], to be dropped at the root of the install partition so
//! Windows Setup reads it. Levers (SPEC-0006): hardware bypass, local account,
//! telemetry minimization, disabling automatic BitLocker, and an optional
//! regional preset. Every value is verified against current sources (dated in
//! SPEC-0006), not hardcoded from memory — some of these drift per build.

mod profile;

pub use profile::{BuildProfile, WIN11_25H2, default_profile};

use base64::Engine as _;

use crate::{Error, Result};

/// A local account to create during OOBE.
#[derive(Clone)]
pub struct LocalAccountSpec {
    /// Account (user) name. Also used as the display name.
    pub name: String,
    /// Optional password. When present it is written obfuscated (not encrypted)
    /// into the answer file; never log it.
    pub password: Option<String>,
}

impl std::fmt::Debug for LocalAccountSpec {
    /// Redacts the password so it never leaks through `Debug` output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAccountSpec")
            .field("name", &self.name)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// A regional preset applied during OOBE (Phase 4.x).
#[derive(Clone, Debug)]
pub struct RegionSpec {
    /// BCP-47 locale tag, e.g. `fr-FR`. Applied to the UI language, system
    /// locale, user locale, and (as the default keyboard) the input locale.
    pub locale: String,
}

/// User-selected Windows install tweaks — the model the generator consumes and
/// the (future) GUI fills.
#[derive(Clone, Debug, Default)]
pub struct WindowsTweaks {
    /// Bypass the TPM / Secure Boot / RAM / Storage / CPU requirement checks.
    pub bypass_hardware: bool,
    /// Create a local account and skip the Microsoft-account/online screens.
    pub local_account: Option<LocalAccountSpec>,
    /// Reduce Windows diagnostic data to the edition minimum and disable
    /// advertising ID / location / Find My Device / feedback (machine-wide,
    /// HKLM). **Not** a full telemetry off on Home/Pro — see SPEC-0006.
    pub minimize_telemetry: bool,
    /// Prevent automatic BitLocker device encryption during setup (24H2+).
    pub disable_auto_bitlocker: bool,
    /// Optional regional preset (UI language / locales / keyboard).
    pub region: Option<RegionSpec>,
}

impl WindowsTweaks {
    /// Whether any tweak is enabled. If not, no `autounattend.xml` is generated.
    #[must_use]
    pub fn any(&self) -> bool {
        self.bypass_hardware
            || self.local_account.is_some()
            || self.minimize_telemetry
            || self.disable_auto_bitlocker
            || self.region.is_some()
    }
}

/// The file name Windows Setup looks for at the media root.
pub const AUTOUNATTEND_FILENAME: &str = "autounattend.xml";

// Fixed schema constants (stable across builds; not the drifting part).
const ARCH: &str = "amd64";
const PKT: &str = "31bf3856ad364e35";
const WCM: &str = "http://schemas.microsoft.com/WMIConfig/2002/State";

// --- Verified Phase 4.x reg-adds (HKLM, machine-wide; see SPEC-0006) --------

/// Telemetry / privacy minimization, applied in the **specialize** pass.
///
/// HKLM only, on purpose: the per-user (HKCU) toggles — tailored experiences,
/// inking/typing — are deferred, because `RunSynchronous` runs as SYSTEM here so
/// an HKCU write would land in SYSTEM's hive, not the OOBE-created user
/// (SPEC-0006). `AllowTelemetry=0` is floored to `1` (Required) on Home/Pro;
/// only Enterprise/Education/Server honor 0.
const TELEMETRY_REG_ADDS: &[&str] = &[
    r#"reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection" /v AllowTelemetry /t REG_DWORD /d 0 /f"#,
    r#"reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\AdvertisingInfo" /v DisabledByGroupPolicy /t REG_DWORD /d 1 /f"#,
    r#"reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\LocationAndSensors" /v DisableLocation /t REG_DWORD /d 1 /f"#,
    r#"reg add "HKLM\SOFTWARE\Policies\Microsoft\FindMyDevice" /v AllowFindMyDevice /t REG_DWORD /d 0 /f"#,
    r#"reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection" /v DoNotShowFeedbackNotifications /t REG_DWORD /d 1 /f"#,
];

/// Prevent automatic BitLocker device encryption. Must run in **specialize**,
/// before 24H2+ auto-encryption triggers during setup.
const BITLOCKER_REG_ADD: &str = r#"reg add "HKLM\SYSTEM\CurrentControlSet\Control\BitLocker" /v PreventDeviceEncryption /t REG_DWORD /d 1 /f"#;

/// BypassNRO offline-account complement (**specialize**), added when a local
/// account is requested and the profile calls for it.
const BYPASSNRO_REG_ADD: &str = r#"reg add "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE" /v BypassNRO /t REG_DWORD /d 1 /f"#;

/// Generate the `autounattend.xml` content for `tweaks` using `profile`.
///
/// Deterministic and well-formed. Returns the XML as a string. Emits at most one
/// `<settings>` block per pass (windowsPE, specialize, oobeSystem).
///
/// # Errors
///
/// Returns [`Error::InvalidTweaks`] if a local account has an empty name or a
/// region has an empty locale.
pub fn generate_autounattend(tweaks: &WindowsTweaks, profile: &BuildProfile) -> Result<String> {
    if let Some(account) = &tweaks.local_account
        && account.name.trim().is_empty()
    {
        return Err(Error::InvalidTweaks(
            "local account name must not be empty".to_owned(),
        ));
    }
    if let Some(region) = &tweaks.region
        && region.locale.trim().is_empty()
    {
        return Err(Error::InvalidTweaks(
            "region locale must not be empty".to_owned(),
        ));
    }

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml.push_str("<unattend xmlns=\"urn:schemas-microsoft-com:unattend\">\n");

    // windowsPE — hardware bypass (LabConfig, before the compat gate).
    if tweaks.bypass_hardware {
        settings_open(&mut xml, "windowsPE");
        component_labconfig(&mut xml, profile);
        settings_close(&mut xml);
    }

    // specialize — every machine-policy reg-add in one Deployment RunSynchronous.
    let commands = specialize_commands(tweaks, profile);
    if !commands.is_empty() {
        settings_open(&mut xml, "specialize");
        component_specialize(&mut xml, &commands);
        settings_close(&mut xml);
    }

    // oobeSystem — regional preset and/or local account.
    if tweaks.region.is_some() || tweaks.local_account.is_some() {
        settings_open(&mut xml, "oobeSystem");
        if let Some(region) = &tweaks.region {
            component_international(&mut xml, region);
        }
        if let Some(account) = &tweaks.local_account {
            component_shell_setup(&mut xml, account);
        }
        settings_close(&mut xml);
    }

    xml.push_str("</unattend>\n");
    Ok(xml)
}

/// The ordered specialize reg-add commands for these tweaks (may be empty).
/// Fixed order → deterministic output.
fn specialize_commands(tweaks: &WindowsTweaks, profile: &BuildProfile) -> Vec<String> {
    let mut cmds: Vec<String> = Vec::new();
    if tweaks.minimize_telemetry {
        cmds.extend(TELEMETRY_REG_ADDS.iter().map(|c| (*c).to_owned()));
    }
    if tweaks.disable_auto_bitlocker {
        cmds.push(BITLOCKER_REG_ADD.to_owned());
    }
    if tweaks.local_account.is_some() && profile.bypass_nro_complement {
        cmds.push(BYPASSNRO_REG_ADD.to_owned());
    }
    cmds
}

// --- XML building blocks ---------------------------------------------------

/// Open a `<settings pass="…">` block.
fn settings_open(xml: &mut String, pass: &str) {
    xml.push_str(&format!("  <settings pass=\"{pass}\">\n"));
}

/// Close a `<settings>` block.
fn settings_close(xml: &mut String) {
    xml.push_str("  </settings>\n");
}

/// Open a `<component>` block.
fn component_open(xml: &mut String, component: &str) {
    xml.push_str(&format!(
        "    <component name=\"{component}\" processorArchitecture=\"{ARCH}\" \
         publicKeyToken=\"{PKT}\" language=\"neutral\" versionScope=\"nonSxS\" \
         xmlns:wcm=\"{WCM}\">\n"
    ));
}

/// Close a `<component>` block.
fn component_close(xml: &mut String) {
    xml.push_str("    </component>\n");
}

/// Emit one `<RunSynchronousCommand>` (Order + Path).
fn push_runsync_command(xml: &mut String, order: usize, command: &str) {
    xml.push_str("        <RunSynchronousCommand wcm:action=\"add\">\n");
    xml.push_str(&format!("          <Order>{order}</Order>\n"));
    xml.push_str(&format!(
        "          <Path>{}</Path>\n",
        escape_text(command)
    ));
    xml.push_str("        </RunSynchronousCommand>\n");
}

/// windowsPE: LabConfig hardware-bypass reg adds (before the compat gate).
fn component_labconfig(xml: &mut String, profile: &BuildProfile) {
    component_open(xml, "Microsoft-Windows-Setup");
    xml.push_str("      <RunSynchronous>\n");
    for (index, key) in profile.labconfig_keys.iter().enumerate() {
        let command =
            format!(r#"reg add "HKLM\SYSTEM\Setup\LabConfig" /v {key} /t REG_DWORD /d 1 /f"#);
        push_runsync_command(xml, index + 1, &command);
    }
    xml.push_str("      </RunSynchronous>\n");
    component_close(xml);
}

/// specialize: one `Microsoft-Windows-Deployment` `RunSynchronous` carrying every
/// machine-policy reg-add (telemetry, BitLocker, BypassNRO), in a fixed order.
fn component_specialize(xml: &mut String, commands: &[String]) {
    component_open(xml, "Microsoft-Windows-Deployment");
    xml.push_str("      <RunSynchronous>\n");
    for (index, command) in commands.iter().enumerate() {
        push_runsync_command(xml, index + 1, command);
    }
    xml.push_str("      </RunSynchronous>\n");
    component_close(xml);
}

/// oobeSystem: regional preset (UI language / locales / default keyboard).
fn component_international(xml: &mut String, region: &RegionSpec) {
    let locale = escape_text(&region.locale);
    component_open(xml, "Microsoft-Windows-International-Core");
    xml.push_str(&format!("      <InputLocale>{locale}</InputLocale>\n"));
    xml.push_str(&format!("      <SystemLocale>{locale}</SystemLocale>\n"));
    xml.push_str(&format!("      <UILanguage>{locale}</UILanguage>\n"));
    xml.push_str(&format!("      <UserLocale>{locale}</UserLocale>\n"));
    component_close(xml);
}

/// oobeSystem: local account + hide the online/MSA screens.
fn component_shell_setup(xml: &mut String, account: &LocalAccountSpec) {
    component_open(xml, "Microsoft-Windows-Shell-Setup");
    xml.push_str("      <OOBE>\n");
    xml.push_str("        <HideEULAPage>true</HideEULAPage>\n");
    xml.push_str("        <HideOnlineAccountScreens>true</HideOnlineAccountScreens>\n");
    xml.push_str("        <HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE>\n");
    xml.push_str("        <ProtectYourPC>3</ProtectYourPC>\n");
    xml.push_str("        <NetworkLocation>Other</NetworkLocation>\n");
    xml.push_str("      </OOBE>\n");

    let name = escape_text(&account.name);
    xml.push_str("      <UserAccounts>\n");
    xml.push_str("        <LocalAccounts>\n");
    xml.push_str("          <LocalAccount wcm:action=\"add\">\n");
    xml.push_str(&format!("            <Name>{name}</Name>\n"));
    xml.push_str("            <Group>Administrators</Group>\n");
    xml.push_str(&format!("            <DisplayName>{name}</DisplayName>\n"));
    if let Some(password) = &account.password {
        xml.push_str("            <Password>\n");
        xml.push_str(&format!(
            "              <Value>{}</Value>\n",
            obfuscate_password(password)
        ));
        xml.push_str("              <PlainText>false</PlainText>\n");
        xml.push_str("            </Password>\n");
    }
    xml.push_str("          </LocalAccount>\n");
    xml.push_str("        </LocalAccounts>\n");
    xml.push_str("      </UserAccounts>\n");
    component_close(xml);
}

/// Windows answer-file password obfuscation for `PlainText=false`:
/// `base64( UTF-16LE( password + "Password" ) )`. Obfuscation, not encryption.
fn obfuscate_password(password: &str) -> String {
    let combined = format!("{password}Password");
    let utf16le: Vec<u8> = combined.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(utf16le)
}

/// Escape XML text content (`&`, `<`, `>`).
fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests;
