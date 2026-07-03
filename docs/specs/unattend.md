# SPEC-0006: autounattend.xml generation — Windows tweaks (Phase 4 + 4.x)

- **Status:** Implemented
- **Module:** `crate::windows` (generator) + deposit inside `crate::copy` (3b mount)
- **Linked specs:** SPEC-0004 (the P1 mount this drops the file into)
- **Verification date:** all sources below checked **2026-07-03**.

## Role

Generate an `autounattend.xml` answer file, parameterized by user-selected
tweaks, and drop it at the **root of the install partition (P1)** so Windows
Setup reads it automatically. This is the project's core differentiator.

**Discipline:** every registry key, XML element and value below carries its
source and verification date. Nothing is hardcoded from memory. Windows tightens
these bypasses per build. Windows 11 **25H2 was confirmed on a real install**
(no requirement wall; local account created without a Microsoft account) — but
keep re-verifying per future build.

## Scope

DONE (Phase 4):
- (a) **Hardware bypass**: TPM / Secure Boot / RAM / Storage / CPU checks.
- (b) **Local account** without a Microsoft account.

DONE (Phase 4.x):
- (c) **Minimize telemetry / data collection** — reduce Windows diagnostic data
  to the minimum the edition allows, plus disable advertising ID, location,
  Find My Device and feedback notifications (all machine-wide / HKLM).
- (d) **Disable automatic BitLocker device encryption**.
- (e) **Regional preset** (optional) — UI language / locales / input locale.

NOT in scope (do not widen the surface): any other tweak. Per-user (HKCU)
privacy toggles are explicitly deferred — see the telemetry lever below and
pitfalls. TODO(phase4.x+1).

## Config model

```
WindowsTweaks {
    bypass_hardware: bool,
    local_account: Option<LocalAccountSpec { name, password: Option<String> }>,
    minimize_telemetry: bool,
    disable_auto_bitlocker: bool,
    region: Option<RegionSpec { locale: String }>,   // e.g. "fr-FR"
}
```

This struct is THE model; the generator consumes it. The GUI (Phase 5) fills the
same struct. **No tweak selected → no autounattend.xml is generated**, and the
stick is a plain Windows installer (Phase 3 behavior strictly unchanged).

## Build profiles (surviving per-build drift)

Build-specific content is isolated in a `BuildProfile` (which LabConfig keys,
whether to add the BypassNRO complement, which passes). The generator assembles
`profile + WindowsTweaks → XML`. Target profile: **`win11-25h2`**. Adding a
future profile is trivial and each profile carries its sources.

## Verified levers (source + date)

### Answer-file location and name

Windows Setup's *implicit answer file search order* includes **removable media,
at the root of the drive, named `Autounattend.xml`** (entries 4 and 5). Since
Setup boots from P1 (NTFS) via UEFI:NTFS, dropping the file at the **P1 root**
is found automatically.
- Source: Microsoft Learn, *Windows Setup Automation Overview* (implicit search
  order table; "Save the file as Autounattend.xml on the root of a USB flash
  drive"). Verified 2026-07-03.
- Ferrus writes it as `autounattend.xml` at the P1 root (case-insensitive lookup).

### (a) Hardware bypass — LabConfig, windowsPE pass

Set five DWORD values under `HKLM\SYSTEM\Setup\LabConfig`, each `= 1`:
`BypassTPMCheck`, `BypassSecureBootCheck`, `BypassRAMCheck`,
`BypassStorageCheck`, `BypassCPUCheck`. They must exist **before the WinPE
compatibility gate**, so they are written by a `RunSynchronousCommand` in the
**`windowsPE`** pass (`Microsoft-Windows-Setup` component) running:
`reg add "HKLM\SYSTEM\Setup\LabConfig" /v <Name> /t REG_DWORD /d 1 /f`.
- Source: long-standing community method (elevenforum "bypass Windows 11
  hardware requirements for clean installs"; iamroot.it 2021; woshub). Confirmed
  the `labconfig_keys()` names from Phase 0 are still correct. Verified 2026-07-03.
- **Confirmed on a real 25H2 install:** the LabConfig bypass gets past the
  clean-install requirement check on 25H2 (no "This PC can't run Windows 11"
  wall on a TPM-less VM). Method unchanged across 22H2/24H2/25H2.

### (b) Local account — oobeSystem pass

`Microsoft-Windows-Shell-Setup` → `UserAccounts` → `LocalAccounts` →
`LocalAccount wcm:action="add"` with `<Name>`, `<Group>Administrators</Group>`,
`<DisplayName>`, and (if a password is given) `<Password><Value>…</Value>
<PlainText>false</PlainText></Password>`. Valid pass: **oobeSystem**.
- Source: Microsoft Learn, *LocalAccount (…useraccounts-localaccounts-
  localaccount)* — child elements + "Valid Configuration Passes: auditSystem,
  oobeSystem" + XML example. Verified 2026-07-03.

**Password obfuscation** (when a password is set): `PlainText=false` expects
`Value = base64( UTF-16LE( password + "Password" ) )`. Derived by decoding the
Microsoft Learn LocalAccount example (`cAB3AF…` → UTF-16LE `"pwPassword"` for
password `"pw"`). Verified 2026-07-03. **This is obfuscation, not encryption** —
trivially reversible; see pitfalls.

### Skipping the online/MSA screens — OOBE, oobeSystem pass

`Microsoft-Windows-Shell-Setup` → `OOBE` with:
`<HideOnlineAccountScreens>true</HideOnlineAccountScreens>`,
`<ProtectYourPC>3</ProtectYourPC>`, `<HideEULAPage>true</HideEULAPage>`,
`<HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE>`,
`<NetworkLocation>Other</NetworkLocation>`.
- Source: Microsoft Learn, *OOBE (…shell-setup-oobe)* — child elements +
  "Valid Configuration Passes: oobeSystem". Verified 2026-07-03.
- Note: `HideLocalAccountScreen` is **Server-only** per Learn — not used.

### BypassNRO complement — specialize pass

The `oobe\bypassnro.cmd` script was **removed in 24H2/25H2**, but the registry
value `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE\BypassNRO` (DWORD `1`)
still re-enables the offline path. Added as a **complement** via a
`RunSynchronousCommand` (`Microsoft-Windows-Deployment`) in the **specialize**
pass — only when a local account is requested.
- Source: BleepingComputer ("Microsoft blocks more tricks to skip Microsoft
  account setup"); Microsoft Community Hub / memstechtips (2025) — script gone,
  registry key still works on 25H2. Verified 2026-07-03.
- Rationale: the local-account answer file is the primary mechanism; BypassNRO is
  belt-and-suspenders per the phase brief.

### (c) Minimize telemetry / data collection — specialize pass

Machine-wide (**HKLM**) policy reg-adds via `RunSynchronousCommand`
(`Microsoft-Windows-Deployment`) in the **specialize** pass (before OOBE), so
they are enforced for the OOBE privacy screen and the installed OS:

| Setting | Key | Value |
|---|---|---|
| Diagnostic data | `HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection\AllowTelemetry` | `0` |
| Advertising ID | `HKLM\SOFTWARE\Policies\Microsoft\Windows\AdvertisingInfo\DisabledByGroupPolicy` | `1` |
| Location | `HKLM\SOFTWARE\Policies\Microsoft\Windows\LocationAndSensors\DisableLocation` | `1` |
| Find My Device | `HKLM\SOFTWARE\Policies\Microsoft\FindMyDevice\AllowFindMyDevice` | `0` |
| Feedback notifications | `HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection\DoNotShowFeedbackNotifications` | `1` |

- Source: Microsoft Learn, *Configure Windows diagnostic data in your
  organization* (AllowTelemetry values 0/1/2/3; updated 2025-11-07) and *Manage
  connections from Windows OS components to Microsoft services* (the exact
  registry keys for advertising ID, location, Find My Device, feedback).
  Verified 2026-07-03.

- **⚠ Edition nuance — do NOT claim "telemetry off".** `AllowTelemetry=0`
  ("Diagnostic data off / Security") is **only honored on Windows Server,
  Enterprise and Education**. On **Home and Pro the effective floor is `1`
  (Required / Basic)** — Windows silently raises 0 to 1. Hence the tweak is
  named **`minimize_telemetry`** ("reduce to the edition minimum"), never
  "disable telemetry". Quoted from the source: *"Diagnostic data off … is only
  available on Windows Server, Windows Enterprise, and Windows Education
  editions."*

- **Deliberately NOT set here (would be a silent no-op):** the *per-user*
  (**HKCU**) toggles — **Tailored experiences**
  (`HKCU\…\CloudContent\DisableTailoredExperiencesWithDiagnosticData`) and
  **Inking & typing** (`HKCU\…\InputPersonalization\RestrictImplicit*Collection`).
  `RunSynchronous` in specialize runs as **SYSTEM**, so an HKCU write lands in
  SYSTEM's hive, not the account created at OOBE — it would not apply. Setting
  these correctly needs the Default-user hive or a `FirstLogonCommand`;
  deferred. TODO(phase4.x+1). This is why the tweak is scoped to HKLM only.

### (d) Disable automatic BitLocker device encryption — specialize pass

`HKLM\SYSTEM\CurrentControlSet\Control\BitLocker\PreventDeviceEncryption` DWORD
`1`, via `RunSynchronousCommand` (`Microsoft-Windows-Deployment`) in the
**specialize** pass.
- **Timing is the point:** since Windows 11 **24H2**, a clean install on a
  device with TPM + Secure Boot **auto-encrypts every partition** during setup.
  The key must exist **before** that triggers — specialize runs after image
  apply and before OOBE, which is early enough. specialize (not oobeSystem) is
  therefore required.
- Source: Microsoft Learn, *BitLocker drive encryption in Windows 11 for OEMs*
  (`PreventDeviceEncryption`); Windows OS Hub / gHacks (2024, 24H2 auto-encrypt
  behavior + this exact key). Verified 2026-07-03.

### (e) Regional preset (optional) — oobeSystem pass

`Microsoft-Windows-International-Core` in the **oobeSystem** pass, driven by a
single BCP-47 tag (`RegionSpec.locale`, e.g. `fr-FR`) applied to all four:
`<UILanguage>`, `<SystemLocale>`, `<UserLocale>`, and `<InputLocale>`.
- `InputLocale` accepts a language tag (uses that language's default keyboard) or
  the `locale:KLID` hex form; Ferrus uses the plain tag for simplicity.
- Source: Microsoft Learn, *Microsoft-Windows-International-Core* and its
  `InputLocale` child (valid passes: oobeSystem, specialize; value formats +
  examples). Verified 2026-07-03.
- Scope note: only the installed-OS locale (oobeSystem). The **Setup UI** language
  (a separate `…International-Core-WinPE` component in windowsPE) is out of scope.

## Integration

The generation + deposit happen **inside the 3b copy** (P1 still mounted), after
the tree copy and **before sync/unmount** — reusing the existing mount and sync,
no new destructive path. The file is written to `<P1 root>/autounattend.xml`.
Honors dry-run (generates nothing on disk; may report the plan).

## Invariants (tested)

1. **Opt-in only.** No tweak flag → no file generated → plain Windows stick
   (Phase 3 unchanged). *Tested.*
2. **Well-formed, deterministic.** Same `WindowsTweaks` + profile → byte-identical,
   XML-parseable output. *Tested* (parse + equality).
3. **Nominal content.** `bypass_hardware:true` + `local_account:Some` →
   parseable XML with the five LabConfig `reg add`s in windowsPE, the
   LocalAccount in oobeSystem, the OOBE nodes, and the BypassNRO complement in
   specialize — the verified 25H2 values, at the right passes. *Tested.*
4. **Phase 4.x levers, nominal.** `minimize_telemetry` → the five HKLM privacy
   reg-adds in specialize; `disable_auto_bitlocker` → `PreventDeviceEncryption`
   in specialize; `region:Some` → International-Core with the four locale
   elements in oobeSystem. Each off → absent. All specialize reg-adds share one
   `Microsoft-Windows-Deployment`/`RunSynchronous` block (single `<settings
   pass="specialize">`). *Tested.*
5. **No Phase 4 regression.** With the same inputs, the Phase 4 output (bypass +
   account) is byte-identical whether or not the new tweaks are toggled off.
   *Tested.*
6. **Secret hygiene.** A supplied password never appears in any log/stdout/error
   or `Debug` output (only, obfuscated, inside the file). *Tested.*
7. **XML-escaped** user input (account name/display name, region locale).
   *Tested.*

## Known pitfalls

- **Per-build drift.** These bypasses are the #1 drift point of the project.
  Isolated in `BuildProfile`; each value dated above; re-verify per Windows build.
- **bypassnro.cmd removed** (24H2/25H2) — do not rely on it; use the answer-file
  account + the BypassNRO *registry* complement.
- **LabConfig timing** — the keys must be written in **windowsPE** (before the
  compat gate), not later.
- **Password is reversible.** `PlainText=false` is base64 obfuscation, not
  encryption; the file persists on the stick. Never log the plaintext; treat the
  file as containing a recoverable secret.
- **25H2 local account** — **confirmed on a real 25H2 install** (account created,
  no Microsoft-account requirement). Microsoft actively tightens this path, so it
  may still break on a future build; re-verify per build.
- **Telemetry is "minimized", not "off".** `AllowTelemetry=0` floors to `1`
  (Required) on Home/Pro — only Enterprise/Education/Server honor 0. Never claim
  full telemetry disablement to the user.
- **HKCU in specialize is a trap.** `RunSynchronous` runs as SYSTEM; HKCU writes
  hit SYSTEM's hive, not the OOBE-created user. That is why tailored-experiences
  and inking/typing are deferred, not shipped as ineffective reg-adds.
- **BitLocker timing.** `PreventDeviceEncryption` must be in **specialize**
  (before OOBE auto-encryption on 24H2+), not oobeSystem.
- **Phase 4.x levers — validated on a real 25H2 install** (TPM 2.0 + Secure Boot
  VM): auto-BitLocker prevented despite the TPM being present, silent OOBE with
  the `fr-FR` preset, and boot under Secure Boot via the signed UEFI:NTFS loader.
  Telemetry: OOBE privacy screens pre-answered; the *level* remains the edition
  floor (Required on Home/Pro), as documented — confirm in Settings if it matters.

## Out of scope

- Per-user (HKCU) privacy toggles: tailored experiences, inking & typing
  personalization — need Default-hive / FirstLogonCommand. TODO(phase4.x+1).
- Setup UI (WinPE) display language; full silent install (disk/edition/key).
- Fully silent install (disk/edition/product-key automation) — only the two
  levers above are in scope.
