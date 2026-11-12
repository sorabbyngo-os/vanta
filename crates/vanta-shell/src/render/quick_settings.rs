use std::{
    io::Write,
    process::{Command, Stdio},
};

use super::{font::FontRenderer, primitives::Canvas};

const PANEL_BG: u32 = 0xF0222427;
const CARD_BG: u32 = 0xFF34373B;
const ACTIVE_BG: u32 = 0xFF45494E;
const ACCENT: u32 = 0xFF42C7E8;
const TEXT_PRIMARY: u32 = 0xFFF4F5F6;
const TEXT_SECONDARY: u32 = 0xFFB9BEC5;
const TRACK_BG: u32 = 0xFF555A61;
const MAX_VISIBLE_WORKSPACES: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Close,
    SwitchWorkspace(u32),
    CreateWorkspace,
    ToggleWifi,
    ToggleBluetooth,
    ToggleMute,
    SetVolumePercent(u32),
    SetBrightnessPercent(u32),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlState {
    pub wifi_enabled: bool,
    pub wifi_connected: bool,
    pub bluetooth_powered: bool,
    pub audio_muted: bool,
    pub volume: f32,
    pub brightness_percent: Option<u32>,
}

impl ControlState {
    pub fn read() -> Self {
        let wifi_enabled = command_text("nmcli", &["radio", "wifi"])
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("enabled"));

        let wifi_connected = command_text("nmcli", &["-t", "-f", "TYPE,STATE", "device", "status"])
            .is_some_and(|s| {
                s.lines().any(|line| {
                    line.starts_with("wifi:") && line.to_ascii_lowercase().contains("connected")
                })
            });

        let bluetooth_powered = command_text("bluetoothctl", &["show"])
            .is_some_and(|s| s.lines().any(|line| line.trim() == "Powered: yes"));

        let audio = command_text("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]);
        let audio_muted = audio.as_deref().is_some_and(|s| s.contains("[MUTED]"));
        let volume = audio
            .as_deref()
            .and_then(|s| {
                s.split_whitespace()
                    .find_map(|part| part.parse::<f32>().ok())
            })
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);

        Self {
            wifi_enabled,
            wifi_connected,
            bluetooth_powered,
            audio_muted,
            volume,
            brightness_percent: read_brightness(),
        }
    }

    pub fn refresh(&mut self) -> bool {
        let next = Self::read();
        let changed = *self != next;
        *self = next;
        changed
    }

    pub fn apply_optimistic(&mut self, action: ControlAction) {
        match action {
            ControlAction::ToggleWifi => {
                self.wifi_enabled = !self.wifi_enabled;
                if !self.wifi_enabled {
                    self.wifi_connected = false;
                }
            }
            ControlAction::ToggleBluetooth => {
                self.bluetooth_powered = !self.bluetooth_powered;
            }
            ControlAction::ToggleMute => {
                self.audio_muted = !self.audio_muted;
            }
            ControlAction::SetVolumePercent(percent) => {
                self.volume = percent.min(100) as f32 / 100.0;
            }
            ControlAction::SetBrightnessPercent(percent) => {
                self.brightness_percent = Some(percent.min(100));
            }
            _ => {}
        }
    }

    pub fn perform(&mut self, action: ControlAction) {
        let result = match action {
            ControlAction::ToggleWifi => {
                let target = if self.wifi_enabled { "off" } else { "on" };
                self.apply_optimistic(action);
                run_command("nmcli", &["radio", "wifi", target])
            }
            ControlAction::ToggleBluetooth => {
                let target = if self.bluetooth_powered { "off" } else { "on" };
                self.apply_optimistic(action);
                run_command("bluetoothctl", &["power", target])
            }
            ControlAction::ToggleMute => {
                self.apply_optimistic(action);
                run_command("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
            }
            ControlAction::SetVolumePercent(percent) => {
                self.apply_optimistic(action);
                let target = percent.min(100) as f32 / 100.0;
                let value = format!("{target:.2}");
                run_command("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &value])
            }
            ControlAction::SetBrightnessPercent(percent) => {
                self.apply_optimistic(action);
                set_brightness_percent(percent)
            }
            _ => return,
        };

        if let Err(error) = result {
            eprintln!("vanta-shell: control action failed: {error}");
            self.refresh();
        } else if matches!(action, ControlAction::SetBrightnessPercent(_)) {
            // Read back the hardware value after the authorized write.
            self.refresh();
        }
    }
}

fn command_text(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_command(program: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn set_brightness_percent(percent: u32) -> Result<(), String> {
    const BACKLIGHT: &str = "/sys/class/backlight/intel_backlight/brightness";
    const MAX_BACKLIGHT: &str = "/sys/class/backlight/intel_backlight/max_brightness";

    let maximum = std::fs::read_to_string(MAX_BACKLIGHT)
        .map_err(|error| format!("cannot read maximum brightness: {error}"))?
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("invalid maximum brightness: {error}"))?;

    if maximum == 0 {
        return Err("backlight reports a zero maximum".to_owned());
    }

    let percent = percent.min(100) as u64;
    let value = (maximum.saturating_mul(percent) + 50) / 100;

    let mut child = Command::new("pkexec")
        .arg("/usr/bin/tee")
        .arg(BACKLIGHT)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start brightness authorization: {error}"))?;

    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| "brightness process has no input pipe".to_owned())?
        .write_all(format!("{value}\n").as_bytes());

    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot send brightness value: {error}"));
    }

    let output = child
        .wait_with_output()
        .map_err(|error| format!("brightness authorization failed: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "pkexec brightness write exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn read_brightness() -> Option<u32> {
    let current = std::fs::read_to_string("/sys/class/backlight/intel_backlight/brightness")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;

    let maximum = std::fs::read_to_string("/sys/class/backlight/intel_backlight/max_brightness")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;

    if maximum == 0 {
        return None;
    }

    Some((current.saturating_mul(100) / maximum).min(100) as u32)
}

pub fn draw(
    canvas: &mut Canvas<'_>,
    font: &FontRenderer,
    width: u32,
    height: u32,
    workspace_count: u32,
    active_workspace: u32,
    state: &ControlState,
) {
    if width < 100 || height < 100 {
        return;
    }

    let w = width as f32;
    let h = height as f32;

    // Dark graphite popup. Individual surfaces provide separation;
    // avoid full-width decorative borders and divider lines.
    canvas.rounded_rect_f(2.0, 2.0, w - 4.0, h - 4.0, 27.0, PANEL_BG);

    // Header
    font.draw_text(canvas, 18.0, 37.0, "Quick Settings", TEXT_PRIMARY, 20.0);
    font.draw_text(canvas, 19.0, 56.0, "System controls", TEXT_SECONDARY, 10.0);

    canvas.rounded_rect_f(w - 66.0, 14.0, 48.0, 30.0, 15.0, CARD_BG);
    font.draw_text(canvas, w - 56.0, 33.0, "Close", TEXT_PRIMARY, 10.0);

    // Connectivity tiles
    draw_tile(
        canvas,
        font,
        16.0,
        72.0,
        166.0,
        66.0,
        "Wi-Fi",
        if state.wifi_enabled {
            if state.wifi_connected {
                "Connected"
            } else {
                "Enabled · not connected"
            }
        } else {
            "Off"
        },
        state.wifi_enabled,
    );

    draw_tile(
        canvas,
        font,
        194.0,
        72.0,
        166.0,
        66.0,
        "Bluetooth",
        if state.bluetooth_powered { "On" } else { "Off" },
        state.bluetooth_powered,
    );

    // Audio card
    canvas.rounded_rect_f(16.0, 150.0, 344.0, 82.0, 18.0, CARD_BG);

    font.draw_text(canvas, 28.0, 172.0, "Sound output", TEXT_PRIMARY, 13.0);
    font.draw_text(
        canvas,
        28.0,
        190.0,
        if state.audio_muted {
            "Muted"
        } else {
            "Output active"
        },
        TEXT_SECONDARY,
        10.0,
    );

    // Mute remains a separate working action.
    canvas.rounded_rect_f(
        26.0,
        197.0,
        70.0,
        27.0,
        13.5,
        if state.audio_muted {
            ACTIVE_BG
        } else {
            0xFF41454A
        },
    );
    font.draw_text(
        canvas,
        39.0,
        215.0,
        if state.audio_muted { "Unmute" } else { "Mute" },
        TEXT_PRIMARY,
        10.0,
    );

    // Volume slider: coordinates are shared with main.rs hit testing.
    let slider_x = 112.0_f32;
    let slider_width = 176.0_f32;
    let slider_y = 210.0_f32;
    let volume = state.volume.clamp(0.0, 1.0);

    canvas.rounded_rect_f(slider_x, slider_y - 3.0, slider_width, 6.0, 3.0, TRACK_BG);

    if volume > 0.0 {
        canvas.rounded_rect_f(
            slider_x,
            slider_y - 3.0,
            slider_width * volume,
            6.0,
            3.0,
            ACCENT,
        );
    }

    let thumb_x = slider_x + slider_width * volume;
    canvas.rounded_rect_f(
        (thumb_x - 6.0).clamp(slider_x - 1.0, slider_x + slider_width - 11.0),
        slider_y - 8.0,
        12.0,
        16.0,
        6.0,
        0xFFF5F7F8,
    );

    font.draw_text(
        canvas,
        307.0,
        215.0,
        &format!("{:.0}%", volume * 100.0),
        TEXT_PRIMARY,
        11.0,
    );

    // Display brightness slider.
    canvas.rounded_rect_f(16.0, 244.0, 344.0, 58.0, 17.0, CARD_BG);
    font.draw_text(
        canvas,
        28.0,
        267.0,
        "Display brightness",
        TEXT_PRIMARY,
        12.0,
    );

    let brightness = state
        .brightness_percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "Unavailable".to_string());

    font.draw_text(canvas, 296.0, 267.0, &brightness, TEXT_SECONDARY, 11.0);

    if let Some(value) = state.brightness_percent {
        let track_x = 28.0;
        let track_width = 320.0;
        canvas.rounded_rect_f(track_x, 282.0, track_width, 4.0, 2.0, TRACK_BG);

        let progress_width = track_width * value as f32 / 100.0;
        if progress_width > 0.0 {
            canvas.rounded_rect_f(track_x, 282.0, progress_width, 4.0, 2.0, ACCENT);
        }

        // White thumb, centered on the current brightness value.
        let thumb_x = (track_x + progress_width).clamp(track_x, track_x + track_width);
        canvas.rounded_rect_f(thumb_x - 6.0, 278.0, 12.0, 12.0, 6.0, TEXT_PRIMARY);
    }

    // Workspaces remain functional in this first pass.
    font.draw_text(canvas, 19.0, 329.0, "Workspaces", TEXT_PRIMARY, 14.0);

    if workspace_count > MAX_VISIBLE_WORKSPACES {
        font.draw_text(
            canvas,
            w - 100.0,
            329.0,
            &format!("{workspace_count} total"),
            TEXT_SECONDARY,
            10.0,
        );
    }

    let visible_count = workspace_count.min(MAX_VISIBLE_WORKSPACES);

    for index in 0..visible_count {
        let column = index % 5;
        let row = index / 5;
        let x = 16.0 + column as f32 * 69.0;
        let y = 340.0 + row as f32 * 42.0;
        let active = index == active_workspace;

        canvas.rounded_rect_f(
            x,
            y,
            62.0,
            34.0,
            12.0,
            if active { ACTIVE_BG } else { CARD_BG },
        );

        let label = format!("{}", index + 1);
        let label_width = font.text_width(&label, 14.0);

        font.draw_text(
            canvas,
            x + (62.0 - label_width) * 0.5,
            y + 22.0,
            &label,
            TEXT_PRIMARY,
            14.0,
        );

        if active {
            canvas.rounded_rect_f(x + 24.0, y + 28.0, 14.0, 2.0, 1.0, ACCENT);
        }
    }

    // Workspace creation action.
    canvas.rounded_rect_f(16.0, 430.0, w - 32.0, 34.0, 16.0, CARD_BG);
    font.draw_text(canvas, 29.0, 452.0, "New workspace", TEXT_PRIMARY, 12.0);
}

fn draw_tile(
    canvas: &mut Canvas<'_>,
    font: &FontRenderer,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    title: &str,
    status: &str,
    active: bool,
) {
    canvas.rounded_rect_f(
        x,
        y,
        width,
        height,
        17.0,
        if active { ACTIVE_BG } else { CARD_BG },
    );

    font.draw_text(canvas, x + 14.0, y + 28.0, title, TEXT_PRIMARY, 15.0);
    font.draw_text(canvas, x + 14.0, y + 49.0, status, TEXT_SECONDARY, 10.0);

    if active {
        canvas.rounded_rect_f(x + width - 18.0, y + 15.0, 6.0, 6.0, 3.0, ACCENT);
    }
}

pub fn hit_test(
    width: u32,
    _height: u32,
    x: f64,
    y: f64,
    workspace_count: u32,
) -> Option<ControlAction> {
    if x >= width.saturating_sub(66) as f64
        && x < width.saturating_sub(18) as f64
        && (14.0..44.0).contains(&y)
    {
        return Some(ControlAction::Close);
    }

    if (16.0..182.0).contains(&x) && (72.0..138.0).contains(&y) {
        return Some(ControlAction::ToggleWifi);
    }

    if (194.0..360.0).contains(&x) && (72.0..138.0).contains(&y) {
        return Some(ControlAction::ToggleBluetooth);
    }

    if (26.0..96.0).contains(&x) && (194.0..228.0).contains(&y) {
        return Some(ControlAction::ToggleMute);
    }

    if (108.0..294.0).contains(&x) && (194.0..228.0).contains(&y) {
        let percent = (((x - 112.0) / 176.0) * 100.0).round().clamp(0.0, 100.0) as u32;

        return Some(ControlAction::SetVolumePercent(percent));
    }

    // Brightness track: x=28..348, centered at y=282.
    if (28.0..348.0).contains(&x) && (274.0..296.0).contains(&y) {
        let percent = (((x - 28.0) / 320.0) * 100.0).round().clamp(0.0, 100.0) as u32;
        return Some(ControlAction::SetBrightnessPercent(percent));
    }

    if (16.0..360.0).contains(&x) && (340.0..416.0).contains(&y) {
        let column = ((x - 16.0) / 69.0) as u32;
        let row = ((y - 340.0) / 42.0) as u32;
        let local_x = (x - 16.0) - column as f64 * 69.0;
        let local_y = (y - 340.0) - row as f64 * 42.0;
        let index = row * 5 + column;

        if column < 5
            && row < 2
            && local_x < 62.0
            && local_y < 34.0
            && index < workspace_count.min(MAX_VISIBLE_WORKSPACES)
        {
            return Some(ControlAction::SwitchWorkspace(index));
        }
    }

    if (16.0..360.0).contains(&x) && (430.0..464.0).contains(&y) {
        return Some(ControlAction::CreateWorkspace);
    }

    None
}
