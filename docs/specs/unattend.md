# SPEC-0006: autounattend.xml generation — Windows tweaks (Phase 4)

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

## Scope (minimal core)

DONE (Phase 4):
- (a) **Hardware bypass**: TPM / Secure Boot / RAM / Storage / CPU checks.
- (b) **Local account** without a Microsoft account.

NOT in Phase 4 (do not widen the surface): telemetry off, auto-BitLocker off,
regional settings. TODO(phase4.x).

## Config model

```
WindowsTweaks {
    bypass_hardware: bool,
    local_account: Option<LocalAccountSpec { name, password: Option<String> }>,
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
4. **Secret hygiene.** A supplied password never appears in any log/stdout/error
   or `Debug` output (only, obfuscated, inside the file). *Tested.*
5. **XML-escaped** user input (account name/display name). *Tested.*

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

## Out of scope

- Telemetry / BitLocker / regional settings (TODO phase4.x).
- Fully silent install (disk/edition/product-key automation) — only the two
  levers above are in scope.
