//! Ferrus graphical front-end (iced — see `docs/adr/0001-gui-framework.md`).
//!
//! **Phase 5a**: the complete interface + UX, wired to the engine only through
//! the core's **dry-run** path (SPEC-0007). Nothing destructive happens here —
//! the action button runs `prepare_windows` with `dry_run = true`. Real writes,
//! privilege elevation and the confirmation dialog land in Phase 5b.
//!
//! All decisions (enumeration, safe-target authorization, ISO validation,
//! autounattend generation) live in `ferrus-core`; this crate is presentation +
//! state orchestration only.

#![windows_subsystem = "windows"]

use std::path::PathBuf;

use ferrus_core::device::{Device, SafeTarget, format_size, list_writable_candidates};
use ferrus_core::partition::prepare_windows;
use ferrus_core::progress::{ProgressSink, Stage};
use ferrus_core::source::{MediaKind, RawImage, inspect_iso_kind};
use ferrus_core::windows::{LocalAccountSpec, RegionSpec, WindowsTweaks};

use iced::widget::{button, checkbox, column, container, row, scrollable, space, text, text_input};
use iced::{Center, Element, Fill, Task, Theme};

fn main() -> iced::Result {
    announce_renderer();
    iced::application(Ferrus::boot, Ferrus::update, Ferrus::view)
        .title("Ferrus — bootable USB creator")
        .theme(theme)
        .run()
}

/// Print one line about the active renderer so a user who sees corrupted text (a
/// wgpu glitch on some GPUs/drivers) can escape to CPU rendering without knowing
/// iced internals.
///
/// iced 0.14 exposes **no programmatic backend selection** (verified against the
/// source): the only public lever is the `ICED_BACKEND` environment variable, and
/// `std::env::set_var` is `unsafe` under edition 2024 — which `#![forbid(unsafe_code)]`
/// rules out — so Ferrus does not set it from code. See SPEC-0007 / README.
fn announce_renderer() {
    match std::env::var("ICED_BACKEND") {
        Ok(backend) => eprintln!("ferrus-gui: renderer backend = {backend} (via ICED_BACKEND)"),
        Err(_) => eprintln!(
            "ferrus-gui: rendering with the GPU (wgpu). If text or glyphs look \
             corrupted, rerun with  ICED_BACKEND=tiny-skia  for CPU rendering."
        ),
    }
}

/// Fixed theme. A free `fn` (not a closure) so it satisfies the `for<'a>`
/// bound iced's `.theme()` requires.
fn theme(_state: &Ferrus) -> Theme {
    Theme::Dark
}

/// A password held so it never leaks through `Debug`/logs.
#[derive(Clone, Default)]
struct Password(String);

impl Password {
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() {
            "<empty>"
        } else {
            "<redacted>"
        })
    }
}

/// A validated source image (from `RawImage::open`).
#[derive(Clone, Debug)]
struct IsoInfo {
    path: PathBuf,
    size: u64,
}

/// The whole UI state.
#[derive(Debug, Default)]
struct Ferrus {
    devices: Vec<Device>,
    selected: Option<Device>,
    show_large: bool,
    loading_devices: bool,
    device_error: Option<String>,

    iso: Option<IsoInfo>,
    media: MediaKind,
    iso_error: Option<String>,

    bypass_hardware: bool,
    account_enabled: bool,
    account_name: String,
    account_password: Password,
    minimize_telemetry: bool,
    disable_auto_bitlocker: bool,
    region_enabled: bool,
    region_locale: String,

    running: bool,
    log: Vec<String>,
    run_error: Option<String>,
}

#[derive(Clone, Debug)]
enum Message {
    Refresh,
    DevicesLoaded(Result<Vec<Device>, String>),
    ToggleShowLarge(bool),
    SelectDevice(Device),
    PickIso,
    IsoChosen(Option<PathBuf>),
    IsoValidated(Result<IsoInfo, String>),
    MediaDetected(PathBuf, MediaKind),
    ToggleBypass(bool),
    ToggleAccount(bool),
    AccountName(String),
    AccountPassword(Password),
    ToggleTelemetry(bool),
    ToggleBitlocker(bool),
    ToggleRegion(bool),
    RegionLocale(String),
    Run,
    RunFinished(Result<Vec<String>, String>),
}

impl Ferrus {
    /// Initial state + kick off the first (unprivileged) enumeration.
    fn boot() -> (Self, Task<Message>) {
        let state = Self {
            loading_devices: true,
            ..Self::default()
        };
        (state, load_devices(false))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => {
                self.loading_devices = true;
                self.device_error = None;
                load_devices(self.show_large)
            }
            Message::DevicesLoaded(result) => {
                self.loading_devices = false;
                match result {
                    Ok(devices) => {
                        // Keep the selection only if it is still present.
                        if let Some(sel) = &self.selected
                            && !devices.iter().any(|d| d.path == sel.path)
                        {
                            self.selected = None;
                        }
                        self.devices = devices;
                        self.device_error = None;
                    }
                    Err(e) => {
                        self.devices.clear();
                        self.selected = None;
                        self.device_error = Some(e);
                    }
                }
                Task::none()
            }
            Message::ToggleShowLarge(on) => {
                self.show_large = on;
                self.loading_devices = true;
                load_devices(on)
            }
            Message::SelectDevice(device) => {
                self.selected = Some(device);
                Task::none()
            }
            Message::PickIso => Task::perform(pick_iso(), Message::IsoChosen),
            Message::IsoChosen(None) => Task::none(), // dialog cancelled
            Message::IsoChosen(Some(path)) => {
                self.iso_error = None;
                Task::perform(validate_iso(path), Message::IsoValidated)
            }
            Message::IsoValidated(Ok(info)) => {
                let path = info.path.clone();
                self.iso = Some(info);
                self.media = MediaKind::Unknown; // until the async hint returns
                self.iso_error = None;
                // Preliminary, unprivileged, no-mount Windows-vs-generic hint.
                Task::perform(
                    async move {
                        let kind = inspect_iso_kind(&path);
                        (path, kind)
                    },
                    |(path, kind)| Message::MediaDetected(path, kind),
                )
            }
            Message::IsoValidated(Err(e)) => {
                self.iso = None;
                self.iso_error = Some(e);
                Task::none()
            }
            Message::MediaDetected(path, kind) => {
                // Apply only if it still matches the current ISO (guards a race
                // where the user picked another image meanwhile).
                if self.iso.as_ref().is_some_and(|i| i.path == path) {
                    self.media = kind;
                }
                Task::none()
            }
            Message::ToggleBypass(v) => {
                self.bypass_hardware = v;
                Task::none()
            }
            Message::ToggleAccount(v) => {
                self.account_enabled = v;
                Task::none()
            }
            Message::AccountName(v) => {
                self.account_name = v;
                Task::none()
            }
            Message::AccountPassword(v) => {
                self.account_password = v;
                Task::none()
            }
            Message::ToggleTelemetry(v) => {
                self.minimize_telemetry = v;
                Task::none()
            }
            Message::ToggleBitlocker(v) => {
                self.disable_auto_bitlocker = v;
                Task::none()
            }
            Message::ToggleRegion(v) => {
                self.region_enabled = v;
                Task::none()
            }
            Message::RegionLocale(v) => {
                self.region_locale = v;
                Task::none()
            }
            Message::Run => {
                let (Some(device), Some(iso)) = (self.selected.clone(), self.iso.clone()) else {
                    return Task::none();
                };
                let tweaks = self.tweaks();
                self.running = true;
                self.log.clear();
                self.run_error = None;
                Task::perform(
                    async move { run_dry_run(device, iso.path, tweaks) },
                    Message::RunFinished,
                )
            }
            Message::RunFinished(result) => {
                self.running = false;
                match result {
                    Ok(lines) => {
                        self.log = lines;
                        self.run_error = None;
                    }
                    Err(e) => self.run_error = Some(e),
                }
                Task::none()
            }
        }
    }

    /// Map the UI state to the core's `WindowsTweaks`, 1:1. Pure — unit tested.
    fn tweaks(&self) -> WindowsTweaks {
        WindowsTweaks {
            bypass_hardware: self.bypass_hardware,
            local_account: self.account_enabled.then(|| LocalAccountSpec {
                name: self.account_name.clone(),
                password: {
                    let p = self.account_password.as_str();
                    (!p.is_empty()).then(|| p.to_owned())
                },
            }),
            minimize_telemetry: self.minimize_telemetry,
            disable_auto_bitlocker: self.disable_auto_bitlocker,
            region: self.region_enabled.then(|| RegionSpec {
                locale: self.region_locale.clone(),
            }),
        }
    }

    /// Whether the Windows tweaks section should be shown: an ISO is selected and
    /// it is not known to be a generic (non-Windows) image.
    fn show_tweaks(&self) -> bool {
        self.iso.is_some() && self.media != MediaKind::Generic
    }

    /// Whether the action button may fire: a target and an ISO are chosen, no run
    /// is in flight, and any enabled tweak that needs input has it.
    fn can_run(&self) -> bool {
        self.selected.is_some()
            && self.iso.is_some()
            && !self.running
            && (!self.account_enabled || !self.account_name.trim().is_empty())
            && (!self.region_enabled || !self.region_locale.trim().is_empty())
    }

    fn view(&self) -> Element<'_, Message> {
        let header = column![
            text("Ferrus").size(30),
            text("Dry run — no data will be written (Phase 5a).").size(14),
        ]
        .spacing(4);

        let mut body = column![header, self.device_section(), self.iso_section()].spacing(24);

        if self.show_tweaks() {
            body = body.push(self.tweaks_section());
        }
        body = body.push(self.action_section());

        container(scrollable(container(body).padding(24).max_width(760)))
            .center_x(Fill)
            .into()
    }

    fn device_section(&self) -> Element<'_, Message> {
        let controls = row![
            text("1 · Target device").size(20),
            space().width(Fill),
            checkbox(self.show_large)
                .label("Show large drives")
                .on_toggle(Message::ToggleShowLarge),
            button(text("Refresh").size(14))
                .style(button::secondary)
                .on_press(Message::Refresh),
        ]
        .spacing(12)
        .align_y(Center);

        let list: Element<'_, Message> = if self.loading_devices {
            text("Scanning…").into()
        } else if let Some(err) = &self.device_error {
            text(format!("⚠ Could not enumerate devices: {err}")).into()
        } else if self.devices.is_empty() {
            text("No removable target devices found. Plug a USB stick and Refresh.").into()
        } else {
            self.devices
                .iter()
                .fold(column![].spacing(6), |col, dev| {
                    let selected = self.selected.as_ref().is_some_and(|s| s.path == dev.path);
                    col.push(device_row(dev, selected))
                })
                .into()
        };

        column![controls, list].spacing(10).into()
    }

    fn iso_section(&self) -> Element<'_, Message> {
        let picked: Element<'_, Message> = match (&self.iso, &self.iso_error) {
            (_, Some(err)) => text(format!("⚠ Invalid image: {err}")).into(),
            (Some(iso), _) => text(format!(
                "{}  ({})",
                iso.path.display(),
                format_size(iso.size)
            ))
            .into(),
            (None, None) => text("No image selected.").into(),
        };

        column![
            row![
                text("2 · Image (ISO)").size(20),
                space().width(Fill),
                button(text("Choose image…").size(14))
                    .style(button::secondary)
                    .on_press(Message::PickIso),
            ]
            .spacing(12)
            .align_y(Center),
            picked,
        ]
        .spacing(10)
        .into()
    }

    fn tweaks_section(&self) -> Element<'_, Message> {
        let mut col = column![
            text("3 · Windows install tweaks").size(20),
            text("Applied only to Windows install media, via autounattend.xml.").size(13),
        ]
        .spacing(10);

        if self.media == MediaKind::Unknown {
            col = col.push(
                text("• Media type undetermined — tweaks shown anyway; verified at write time.")
                    .size(12),
            );
        }

        col = col
            .push(
                checkbox(self.bypass_hardware)
                    .label(
                        "Bypass Windows 11 hardware checks (TPM / Secure Boot / RAM / storage / CPU)",
                    )
                    .on_toggle(Message::ToggleBypass),
            )
            .push(
                checkbox(self.account_enabled)
                    .label("Create a local account (no Microsoft account)")
                    .on_toggle(Message::ToggleAccount),
            );

        if self.account_enabled {
            col = col.push(
                row![
                    text_input("Account name", &self.account_name).on_input(Message::AccountName),
                    text_input("Password (optional)", self.account_password.as_str())
                        .on_input(|s| Message::AccountPassword(Password(s)))
                        .secure(true),
                ]
                .spacing(10),
            );
        }

        col = col
            .push(
                checkbox(self.minimize_telemetry)
                    .label("Minimize telemetry / data collection (edition minimum — not fully off on Home/Pro)")
                    .on_toggle(Message::ToggleTelemetry),
            )
            .push(
                checkbox(self.disable_auto_bitlocker)
                    .label("Disable automatic BitLocker device encryption")
                    .on_toggle(Message::ToggleBitlocker),
            )
            .push(
                checkbox(self.region_enabled)
                    .label("Regional preset")
                    .on_toggle(Message::ToggleRegion),
            );

        if self.region_enabled {
            col = col.push(
                text_input("BCP-47 tag, e.g. fr-FR", &self.region_locale)
                    .on_input(Message::RegionLocale)
                    .width(220),
            );
        }

        col.into()
    }

    fn action_section(&self) -> Element<'_, Message> {
        let label = if self.running {
            "Simulating…"
        } else {
            "Simulate (dry run)"
        };
        let run_button = button(text(label).size(16))
            .padding([10, 20])
            .style(button::primary)
            .on_press_maybe(self.can_run().then_some(Message::Run));

        let mut col = column![run_button].spacing(12);

        if let Some(err) = &self.run_error {
            col = col.push(text(format!("⚠ Error: {err}")));
        }
        if !self.log.is_empty() {
            let lines = self
                .log
                .iter()
                .fold(column![].spacing(2), |c, line| c.push(text(line).size(13)));
            col = col.push(
                container(scrollable(lines))
                    .padding(12)
                    .width(Fill)
                    .max_height(240),
            );
        }
        col.into()
    }
}

/// One selectable device row, showing path / size / bus / model unambiguously.
fn device_row(dev: &Device, selected: bool) -> Element<'_, Message> {
    let line = format!(
        "{:<12}  {:>9}  {:<5}  {}",
        dev.path.display(),
        format_size(dev.size_bytes),
        dev.bus,
        dev.model.as_deref().unwrap_or("(unknown model)"),
    );
    let by_id = dev
        .stable_id
        .as_deref()
        .map(|id| format!("by-id: {id}"))
        .unwrap_or_default();

    let content = column![text(line).size(14), text(by_id).size(11)].spacing(2);

    button(content)
        .width(Fill)
        .style(if selected {
            button::primary
        } else {
            button::secondary
        })
        .on_press(Message::SelectDevice(dev.clone()))
        .into()
}

/// Async: enumerate safe candidates (unprivileged, like `ferrus list`).
fn load_devices(show_large: bool) -> Task<Message> {
    Task::perform(
        async move { list_writable_candidates(show_large).map_err(|e| e.to_string()) },
        Message::DevicesLoaded,
    )
}

/// Async: open the native file picker (rfd → xdg-portal).
async fn pick_iso() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .add_filter("Disk image", &["iso", "img"])
        .set_title("Choose an ISO / image")
        .pick_file()
        .await
        .map(|handle| handle.path().to_path_buf())
}

/// Async: validate the chosen image via the core (opaque byte stream).
async fn validate_iso(path: PathBuf) -> Result<IsoInfo, String> {
    RawImage::open(&path)
        .map(|img| IsoInfo {
            path,
            size: img.size_bytes(),
        })
        .map_err(|e| e.to_string())
}

/// Run the whole flow through the core in **dry-run** — no writes. Returns the
/// simulated log lines or a message.
fn run_dry_run(
    device: Device,
    iso_path: PathBuf,
    tweaks: WindowsTweaks,
) -> Result<Vec<String>, String> {
    let target =
        SafeTarget::acquire(device.clone(), &device.path, true).map_err(|e| e.to_string())?;
    let image = RawImage::open(&iso_path).map_err(|e| e.to_string())?;
    let tweaks_opt = if tweaks.any() { Some(&tweaks) } else { None };
    let mut sink = LogSink::default();
    prepare_windows(&target, Some(&image), tweaks_opt, false, &mut sink)
        .map_err(|e| e.to_string())?;
    Ok(sink.lines)
}

/// A `ProgressSink` that collects the (simulated) steps as text lines.
#[derive(Default)]
struct LogSink {
    lines: Vec<String>,
}

impl ProgressSink for LogSink {
    fn stage(&mut self, stage: Stage) {
        self.lines.push(format!("[{stage:?}]"));
    }
    fn advance(&mut self, _done: u64, _total: Option<u64>) {}
    fn message(&mut self, text: &str) {
        self.lines.push(text.to_owned());
    }
}

#[cfg(test)]
mod tests;
