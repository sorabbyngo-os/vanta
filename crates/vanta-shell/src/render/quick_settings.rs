use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use super::{font::FontRenderer, primitives::Canvas, theme::Theme};

const MAX_VISIBLE_WORKSPACES: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectivityPanel {
    #[default]
    None,
    Wifi,
    Bluetooth,
    Appearance,
    KeyboardBacklight,
    Power,
    Workspaces,
    Sound,
    Brightness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Close,
    SwitchWorkspace(u32),
    CreateWorkspace,
    ToggleWifi,
    ToggleBluetooth,
    OpenWifiDetails,
    OpenBluetoothDetails,
    OpenAppearance,
    OpenKeyboardBacklight,
    OpenPower,
    OpenSound,
    OpenBrightness,
    BackToControls,
    RefreshConnectivity,
    ToggleMute,
    SetVolumePercent(u32),
    SetSystemVolumePercent(u32),
    SetOutputVolumePercent(u32),
    SetBrightnessPercent(u32),
    SetKeyboardBacklight(u32),
    OpenWorkspaces,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardBacklightState {
    pub device: String,
    pub current: u32,
    pub maximum: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlState {
    pub wifi_enabled: bool,
    pub wifi_connected: bool,
    pub bluetooth_powered: bool,
    pub audio_muted: bool,
    pub system_volume_percent: u32,
    pub output_volume_percent: u32,
    pub output_description: String,
    pub output_is_bluetooth: bool,
    pub output_muted: bool,
    pub audio_stream_count: usize,
    pub volume: f32,
    pub brightness_percent: Option<u32>,
    pub keyboard_backlight: Option<KeyboardBacklightState>,
    pub wifi_networks: Vec<WifiNetwork>,
    pub bluetooth_devices: Vec<BluetoothDevice>,
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

        Self {
            wifi_enabled,
            wifi_connected,
            bluetooth_powered,
            audio_muted: false,
            volume: 1.0,
            system_volume_percent: 100,
            output_volume_percent: 0,
            output_description: String::new(),
            output_is_bluetooth: false,
            output_muted: false,
            audio_stream_count: 0,
            brightness_percent: read_brightness(),
            keyboard_backlight: read_keyboard_backlight(),
            wifi_networks: Vec::new(),
            bluetooth_devices: Vec::new(),
        }
    }

    pub fn refresh(&mut self) -> bool {
        let mut next = Self::read();
        next.wifi_networks = self.wifi_networks.clone();
        next.bluetooth_devices = self.bluetooth_devices.clone();
        next.audio_muted = self.audio_muted;
        next.volume = self.volume;
        next.system_volume_percent = self.system_volume_percent;
        next.output_volume_percent = self.output_volume_percent;
        next.output_description = self.output_description.clone();
        next.output_is_bluetooth = self.output_is_bluetooth;
        next.output_muted = self.output_muted;
        next.audio_stream_count = self.audio_stream_count;

        let changed = *self != next;
        *self = next;
        changed
    }

    pub fn refresh_connectivity(&mut self) -> bool {
        let networks = if self.wifi_enabled {
            read_wifi_networks()
        } else {
            Vec::new()
        };
        let devices = read_bluetooth_devices();

        let changed = self.wifi_networks != networks || self.bluetooth_devices != devices;

        self.wifi_networks = networks;
        self.bluetooth_devices = devices;
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
                self.output_muted = self.audio_muted;
            }
            ControlAction::SetVolumePercent(percent)
            | ControlAction::SetSystemVolumePercent(percent) => {
                let value = percent.min(100);
                self.system_volume_percent = value;
                self.volume = value as f32 / 100.0;
            }
            ControlAction::SetOutputVolumePercent(percent) => {
                self.output_volume_percent = percent.min(150);
            }
            ControlAction::SetBrightnessPercent(percent) => {
                self.brightness_percent = Some(percent.min(100));
            }
            ControlAction::SetKeyboardBacklight(level) => {
                if let Some(backlight) = self.keyboard_backlight.as_mut() {
                    backlight.current = level.min(backlight.maximum);
                }
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
            ControlAction::SetBrightnessPercent(percent) => {
                self.apply_optimistic(action);
                set_brightness_percent(percent)
            }
            ControlAction::SetKeyboardBacklight(level) => {
                let device = self
                    .keyboard_backlight
                    .as_ref()
                    .map(|item| item.device.clone());
                self.apply_optimistic(action);
                set_keyboard_backlight_level(device.as_deref(), level)
            }
            _ => return,
        };

        if let Err(error) = result {
            eprintln!("vanta-shell: control action failed: {error}");
            self.refresh();
        } else if matches!(
            action,
            ControlAction::SetBrightnessPercent(_) | ControlAction::SetKeyboardBacklight(_)
        ) {
            // Read back the hardware value after the authorized write.
            self.refresh();
        }
    }
}

fn split_nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut escaped = false;

    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                ':' => {
                    fields.push(std::mem::take(&mut current));
                }
                other => current.push(other),
            }
        }
    }

    if escaped {
        current.push('\\');
    }

    fields.push(current);
    fields
}

fn read_wifi_networks() -> Vec<WifiNetwork> {
    let Some(output) = command_text(
        "nmcli",
        &[
            "--terse",
            "--escape",
            "yes",
            "--fields",
            "IN-USE,SSID,SIGNAL,SECURITY",
            "device",
            "wifi",
            "list",
        ],
    ) else {
        return Vec::new();
    };

    let mut networks: Vec<WifiNetwork> = Vec::new();

    for line in output.lines() {
        let fields = split_nmcli_fields(line);
        if fields.len() < 4 {
            continue;
        }

        let connected = fields[0].trim() == "*";
        let ssid = fields[1].trim().to_owned();
        if ssid.is_empty() {
            continue;
        }

        let signal = fields[2].trim().parse::<u8>().unwrap_or(0).min(100);
        let security = if fields[3].trim().is_empty() {
            "Open".to_owned()
        } else {
            fields[3].trim().to_owned()
        };

        if let Some(existing) = networks.iter_mut().find(|item| item.ssid == ssid) {
            existing.connected |= connected;
            if signal > existing.signal {
                existing.signal = signal;
                existing.security = security;
            }
        } else {
            networks.push(WifiNetwork {
                ssid,
                signal,
                security,
                connected,
            });
        }
    }

    networks.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| b.signal.cmp(&a.signal))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });

    networks
}

fn read_bluetooth_devices() -> Vec<BluetoothDevice> {
    let Some(output) = command_text("bluetoothctl", &["devices"]) else {
        return Vec::new();
    };

    let mut devices = Vec::new();

    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("Device ") else {
            continue;
        };

        let Some((address, name)) = rest.split_once(' ') else {
            continue;
        };

        if address.len() != 17 || name.trim().is_empty() {
            continue;
        }

        devices.push(BluetoothDevice {
            address: address.to_owned(),
            name: name.trim().to_owned(),
        });
    }

    devices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    devices.dedup_by(|a, b| a.address == b.address);
    devices
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

fn keyboard_backlight_device() -> Option<(String, PathBuf, u32, u32)> {
    let entries = std::fs::read_dir("/sys/class/leds").ok()?;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let normalized = name.to_ascii_lowercase();

        if !normalized.contains("kbd_backlight")
            && !normalized.contains("keyboard_backlight")
            && !normalized.contains("keyboard-backlight")
        {
            continue;
        }

        let directory = entry.path();
        let brightness_path = directory.join("brightness");
        let maximum_path = directory.join("max_brightness");

        let Ok(current_text) = std::fs::read_to_string(&brightness_path) else {
            continue;
        };
        let Ok(maximum_text) = std::fs::read_to_string(&maximum_path) else {
            continue;
        };
        let Ok(current) = current_text.trim().parse::<u32>() else {
            continue;
        };
        let Ok(maximum) = maximum_text.trim().parse::<u32>() else {
            continue;
        };

        if maximum > 0 && current <= maximum {
            return Some((name, brightness_path, current, maximum));
        }
    }

    None
}

fn read_keyboard_backlight() -> Option<KeyboardBacklightState> {
    let (device, _path, current, maximum) = keyboard_backlight_device()?;
    Some(KeyboardBacklightState {
        device,
        current,
        maximum,
    })
}

fn set_keyboard_backlight_level(expected_device: Option<&str>, level: u32) -> Result<(), String> {
    let (device, path, _current, maximum) = keyboard_backlight_device()
        .ok_or_else(|| "no supported keyboard backlight was detected".to_owned())?;

    if expected_device != Some(device.as_str()) {
        return Err("keyboard backlight device changed; refresh controls and try again".to_owned());
    }

    let value = level.min(maximum);
    let mut child = Command::new("pkexec")
        .arg("/usr/bin/tee")
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start keyboard backlight authorization: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "keyboard backlight process has no input pipe".to_owned())?;

    if let Err(error) = stdin.write_all(format!("{value}\n").as_bytes()) {
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot send keyboard backlight value: {error}"));
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|error| format!("keyboard backlight authorization failed: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "pkexec keyboard backlight write exited with {}: {}",
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
    colors: &Theme,
    expanded: ConnectivityPanel,
    workspaces_hovered: bool,
    add_workspace_hovered: bool,
) {
    if width < 100 || height < 100 {
        return;
    }

    let w = width as f32;
    let h = height as f32;

    // Dark graphite popup. Individual surfaces provide separation;
    // avoid full-width decorative borders and divider lines.
    canvas.rounded_rect_f(2.0, 2.0, w - 4.0, h - 4.0, 29.0, colors.quick_panel);
    canvas.rounded_rect_f(4.0, 4.0, w - 8.0, 1.0, 0.5, 0x20FFFFFF);

    // Header
    let (header_title, header_subtitle) = if expanded == ConnectivityPanel::Workspaces {
        ("Workspaces", "Choose a workspace")
    } else {
        ("Control Center", "System controls")
    };

    font.draw_text(canvas, 18.0, 37.0, header_title, colors.text, 20.0);
    font.draw_text(canvas, 19.0, 56.0, header_subtitle, colors.text_muted, 10.0);

    // Workspace controls expand leftward; their right edges remain fixed.
    let workspace_button_x = if workspaces_hovered { 176.0 } else { 212.0 };
    let workspace_button_width = if workspaces_hovered { 72.0 } else { 36.0 };
    canvas.rounded_rect_f(
        workspace_button_x,
        14.0,
        workspace_button_width,
        30.0,
        15.0,
        if workspaces_hovered {
            colors.quick_active
        } else {
            colors.quick_card
        },
    );
    font.draw_text(
        canvas,
        if workspaces_hovered { 188.0 } else { 224.0 },
        33.0,
        if workspaces_hovered { "Spaces" } else { "W" },
        colors.text,
        10.0,
    );

    let add_button_x = if add_workspace_hovered { 254.0 } else { 282.0 };
    let add_button_width = if add_workspace_hovered { 64.0 } else { 36.0 };
    canvas.rounded_rect_f(
        add_button_x,
        14.0,
        add_button_width,
        30.0,
        15.0,
        if add_workspace_hovered {
            colors.quick_active
        } else {
            colors.quick_card
        },
    );
    font.draw_text(
        canvas,
        if add_workspace_hovered { 275.0 } else { 296.0 },
        33.0,
        if add_workspace_hovered { "Add" } else { "+" },
        colors.text,
        10.0,
    );

    canvas.rounded_rect_f(w - 50.0, 14.0, 44.0, 30.0, 15.0, 0x704A4A52);
    font.draw_text(canvas, w - 44.0, 33.0, "Close", colors.text, 9.0);

    // Expanded connectivity screens replace the controls instead of
    // appearing underneath them.
    if expanded == ConnectivityPanel::Workspaces {
        canvas.rounded_rect_f(16.0, 72.0, 112.0, 36.0, 16.0, 0x704A4A52);
        font.draw_text(canvas, 29.0, 95.0, "< Controls", colors.text, 11.0);

        if matches!(
            expanded,
            ConnectivityPanel::Wifi | ConnectivityPanel::Bluetooth
        ) {
            canvas.rounded_rect_f(266.0, 72.0, 94.0, 36.0, 16.0, colors.quick_active);
            font.draw_text(canvas, 283.0, 95.0, "Refresh", colors.text, 11.0);
        }

        match expanded {
            ConnectivityPanel::Wifi => {
                let status = if !state.wifi_enabled {
                    "Wi-Fi is turned off"
                } else if state.wifi_connected {
                    "Connected network appears first"
                } else {
                    "Available nearby networks"
                };

                font.draw_text(canvas, 20.0, 139.0, status, colors.text_muted, 10.0);
                font.draw_text(canvas, 20.0, 164.0, "NETWORKS", colors.text_muted, 9.0);

                if !state.wifi_enabled {
                    font.draw_text(
                        canvas,
                        20.0,
                        204.0,
                        "Turn on Wi-Fi to discover networks",
                        colors.text,
                        11.0,
                    );
                } else if state.wifi_networks.is_empty() {
                    font.draw_text(
                        canvas,
                        20.0,
                        204.0,
                        "No networks found. Select Refresh to scan again.",
                        colors.text_muted,
                        10.0,
                    );
                } else {
                    for (index, network) in state.wifi_networks.iter().take(7).enumerate() {
                        let y = 176.0 + index as f32 * 44.0;
                        canvas.rounded_rect_f(
                            16.0,
                            y,
                            344.0,
                            39.0,
                            12.0,
                            if network.connected {
                                0x80426BA8
                            } else {
                                0x704A4A52
                            },
                        );

                        font.draw_text(
                            canvas,
                            28.0,
                            y + 16.0,
                            &truncate_label(&network.ssid, 39),
                            colors.text,
                            11.0,
                        );

                        let meta = if network.connected {
                            format!("Connected · {}% signal", network.signal)
                        } else {
                            format!("{}% signal · {}", network.signal, network.security)
                        };

                        font.draw_text(
                            canvas,
                            28.0,
                            y + 31.0,
                            &truncate_label(&meta, 48),
                            colors.text_muted,
                            9.0,
                        );
                    }
                }
            }
            ConnectivityPanel::Bluetooth => {
                let status = if state.bluetooth_powered {
                    "Bluetooth is on"
                } else {
                    "Bluetooth is turned off"
                };

                font.draw_text(canvas, 20.0, 139.0, status, colors.text_muted, 10.0);
                font.draw_text(canvas, 20.0, 164.0, "KNOWN DEVICES", colors.text_muted, 9.0);

                if !state.bluetooth_powered {
                    font.draw_text(
                        canvas,
                        20.0,
                        204.0,
                        "Turn on Bluetooth to discover devices",
                        colors.text,
                        11.0,
                    );
                } else if state.bluetooth_devices.is_empty() {
                    font.draw_text(
                        canvas,
                        20.0,
                        204.0,
                        "No known devices found. Select Refresh to scan again.",
                        colors.text_muted,
                        10.0,
                    );
                } else {
                    for (index, device) in state.bluetooth_devices.iter().take(7).enumerate() {
                        let y = 176.0 + index as f32 * 44.0;
                        canvas.rounded_rect_f(16.0, y, 344.0, 39.0, 12.0, 0x704A4A52);

                        font.draw_text(
                            canvas,
                            28.0,
                            y + 16.0,
                            &truncate_label(&device.name, 38),
                            colors.text,
                            11.0,
                        );
                        font.draw_text(
                            canvas,
                            28.0,
                            y + 31.0,
                            &device.address,
                            colors.text_muted,
                            9.0,
                        );
                    }
                }
            }
            ConnectivityPanel::Appearance => {
                font.draw_text(
                    canvas,
                    20.0,
                    139.0,
                    "Display preferences",
                    colors.text,
                    13.0,
                );
                font.draw_text(
                    canvas,
                    20.0,
                    164.0,
                    "Theme options will be added here.",
                    colors.text_muted,
                    10.0,
                );
            }
            ConnectivityPanel::KeyboardBacklight => {
                if let Some(backlight) = &state.keyboard_backlight {
                    font.draw_text(
                        canvas,
                        20.0,
                        139.0,
                        &format!(
                            "{} · level {}/{}",
                            backlight.device, backlight.current, backlight.maximum
                        ),
                        colors.text_muted,
                        10.0,
                    );

                    let low = (backlight.maximum / 2).max(1);
                    let options: &[(u32, &str)] = if backlight.maximum >= 2 {
                        &[(0, "Off"), (low, "Low"), (backlight.maximum, "High")]
                    } else {
                        &[(0, "Off"), (backlight.maximum, "On")]
                    };

                    for (index, (level, label)) in options.iter().enumerate() {
                        let x = 16.0 + index as f32 * 114.0;
                        let active = backlight.current == *level;
                        canvas.rounded_rect_f(
                            x,
                            184.0,
                            104.0,
                            54.0,
                            16.0,
                            if active {
                                colors.quick_active
                            } else {
                                colors.quick_card
                            },
                        );
                        font.draw_text(canvas, x + 25.0, 216.0, label, colors.text, 12.0);
                    }
                } else {
                    font.draw_text(
                        canvas,
                        20.0,
                        139.0,
                        "No supported backlit keyboard detected.",
                        colors.text_muted,
                        11.0,
                    );
                }
            }
            ConnectivityPanel::Power => {
                font.draw_text(canvas, 20.0, 139.0, "Power options", colors.text, 13.0);
                font.draw_text(
                    canvas,
                    20.0,
                    164.0,
                    "Session actions will be added here.",
                    colors.text_muted,
                    10.0,
                );
            }
            ConnectivityPanel::Workspaces => {
                canvas.rounded_rect_f(10.0, 116.0, w - 20.0, h - 128.0, 22.0, 0xE92B313B);
                font.draw_text(canvas, 20.0, 139.0, "Choose workspace", colors.text, 13.0);
                font.draw_text(
                    canvas,
                    20.0,
                    153.0,
                    &format!("{} available", workspace_count),
                    colors.text_muted,
                    9.0,
                );

                for index in 0..workspace_count.min(MAX_VISIBLE_WORKSPACES) {
                    let column = index % 5;
                    let row = index / 5;
                    let x = 16.0 + column as f32 * 69.0;
                    let y = 166.0 + row as f32 * 42.0;
                    let active = index == active_workspace;

                    canvas.rounded_rect_f(
                        x,
                        y,
                        62.0,
                        34.0,
                        12.0,
                        if active {
                            0xFF287FC5
                        } else {
                            colors.quick_card
                        },
                    );
                    font.draw_text(
                        canvas,
                        x + 27.0,
                        y + 22.0,
                        &format!("{}", index + 1),
                        colors.text,
                        14.0,
                    );

                    if active {
                        canvas.rounded_rect_f(x + 24.0, y + 28.0, 14.0, 2.0, 1.0, colors.accent);
                    }
                }

                canvas.rounded_rect_f(16.0, 260.0, 344.0, 36.0, 18.0, colors.quick_active);
                font.draw_text(
                    canvas,
                    29.0,
                    283.0,
                    "+  Create workspace",
                    colors.text,
                    11.0,
                );
            }
            ConnectivityPanel::None | ConnectivityPanel::Sound | ConnectivityPanel::Brightness => {
                unreachable!()
            }
        }

        return;
    }

    // Connectivity tiles
    draw_connectivity_tile(
        canvas,
        font,
        colors,
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

    draw_connectivity_tile(
        canvas,
        font,
        colors,
        194.0,
        72.0,
        166.0,
        66.0,
        "Bluetooth",
        if state.bluetooth_powered { "On" } else { "Off" },
        state.bluetooth_powered,
    );

    // Audio card
    canvas.rounded_rect_f(16.0, 150.0, 344.0, 82.0, 20.0, 0x704A4A52);

    font.draw_text(canvas, 28.0, 172.0, "Sound output", colors.text, 13.0);
    font.draw_text(
        canvas,
        28.0,
        190.0,
        if state.audio_muted {
            "Muted"
        } else {
            "Output active"
        },
        colors.text_muted,
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
            colors.quick_active
        } else {
            0xFF41454A
        },
    );
    font.draw_text(
        canvas,
        39.0,
        215.0,
        if state.audio_muted { "Unmute" } else { "Mute" },
        colors.text,
        10.0,
    );

    font.draw_text(
        canvas,
        112.0,
        215.0,
        &format!(
            "System volume · {:.0}%",
            state.volume.clamp(0.0, 1.0) * 100.0
        ),
        colors.text_muted,
        10.0,
    );

    // Brightness card opens its dedicated floating control.
    canvas.rounded_rect_f(16.0, 244.0, 344.0, 58.0, 19.0, 0x704A4A52);
    font.draw_text(canvas, 28.0, 267.0, "Display brightness", colors.text, 12.0);

    let brightness = state
        .brightness_percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "Unavailable".to_string());

    font.draw_text(canvas, 296.0, 267.0, &brightness, colors.text_muted, 11.0);
    font.draw_text(
        canvas,
        28.0,
        289.0,
        "Open brightness controls",
        colors.text_muted,
        9.0,
    );

    // Compact utility shortcuts. Workspace management now lives in the header.
    let mut tiles = vec![(16.0, 330.0, "Appearance", ">")];

    if state.keyboard_backlight.is_some() {
        tiles.push((194.0, 330.0, "Keyboard backlight", ">"));
        tiles.push((16.0, 396.0, "Power", ">"));
    } else {
        tiles.push((194.0, 330.0, "Power", ">"));
    }

    for (x, y, label, arrow) in tiles {
        canvas.rounded_rect_f(x, y, 166.0, 56.0, 17.0, colors.quick_card);
        canvas.rounded_rect_f(x + 1.0, y + 1.0, 164.0, 1.0, 0.5, 0x28FFFFFF);
        font.draw_text(canvas, x + 13.0, y + 25.0, label, colors.text, 11.0);
        font.draw_text(canvas, x + 145.0, y + 34.0, arrow, colors.text_muted, 17.0);
    }

    if expanded != ConnectivityPanel::None {
        draw_feature_popup(canvas, font, width, height, state, colors, expanded);
    }
}

fn draw_feature_popup(
    canvas: &mut Canvas<'_>,
    font: &FontRenderer,
    width: u32,
    height: u32,
    state: &ControlState,
    colors: &Theme,
    expanded: ConnectivityPanel,
) {
    let popup_x = 20.0;
    let popup_y = 76.0;
    let popup_width = width as f32 - 40.0;
    let popup_height = height as f32 - 96.0;

    canvas.rounded_rect_f(
        popup_x,
        popup_y,
        popup_width,
        popup_height,
        24.0,
        0xF0202733,
    );
    canvas.rounded_rect_f(
        popup_x + 2.0,
        popup_y + 2.0,
        popup_width - 4.0,
        1.0,
        0.5,
        0x30FFFFFF,
    );

    let title = match expanded {
        ConnectivityPanel::Wifi => "Wi-Fi",
        ConnectivityPanel::Bluetooth => "Bluetooth",
        ConnectivityPanel::Appearance => "Appearance",
        ConnectivityPanel::KeyboardBacklight => "Keyboard backlight",
        ConnectivityPanel::Power => "Power",
        ConnectivityPanel::Sound => "Sound",
        ConnectivityPanel::Brightness => "Brightness",
        ConnectivityPanel::None | ConnectivityPanel::Workspaces => return,
    };
    font.draw_text(
        canvas,
        popup_x + 16.0,
        popup_y + 30.0,
        title,
        colors.text,
        15.0,
    );

    match expanded {
        ConnectivityPanel::Wifi => {
            let status = if !state.wifi_enabled {
                "Wi-Fi is turned off"
            } else if state.wifi_connected {
                "Connected network appears first"
            } else {
                "Available nearby networks"
            };
            font.draw_text(canvas, 36.0, 132.0, status, colors.text_muted, 10.0);
            canvas.rounded_rect_f(260.0, 88.0, 76.0, 28.0, 14.0, colors.quick_active);
            font.draw_text(canvas, 278.0, 106.0, "Refresh", colors.text, 10.0);
            font.draw_text(canvas, 36.0, 153.0, "NETWORKS", colors.text_muted, 9.0);

            if !state.wifi_enabled || state.wifi_networks.is_empty() {
                let message = if state.wifi_enabled {
                    "No networks found. Refresh to scan."
                } else {
                    "Turn on Wi-Fi to discover networks."
                };
                font.draw_text(canvas, 36.0, 190.0, message, colors.text, 10.0);
            } else {
                for (index, network) in state.wifi_networks.iter().take(6).enumerate() {
                    let y = 164.0 + index as f32 * 46.0;
                    canvas.rounded_rect_f(
                        32.0,
                        y,
                        312.0,
                        40.0,
                        12.0,
                        if network.connected {
                            0x80426BA8
                        } else {
                            colors.quick_card
                        },
                    );
                    font.draw_text(
                        canvas,
                        44.0,
                        y + 16.0,
                        &truncate_label(&network.ssid, 34),
                        colors.text,
                        10.0,
                    );
                    let meta = if network.connected {
                        format!("Connected · {}% signal", network.signal)
                    } else {
                        format!("{}% signal · {}", network.signal, network.security)
                    };
                    font.draw_text(
                        canvas,
                        44.0,
                        y + 31.0,
                        &truncate_label(&meta, 42),
                        colors.text_muted,
                        9.0,
                    );
                }
            }
        }
        ConnectivityPanel::Bluetooth => {
            let status = if state.bluetooth_powered {
                "Bluetooth is on"
            } else {
                "Bluetooth is turned off"
            };
            font.draw_text(canvas, 36.0, 132.0, status, colors.text_muted, 10.0);
            canvas.rounded_rect_f(260.0, 88.0, 76.0, 28.0, 14.0, colors.quick_active);
            font.draw_text(canvas, 278.0, 106.0, "Refresh", colors.text, 10.0);
            font.draw_text(canvas, 36.0, 153.0, "KNOWN DEVICES", colors.text_muted, 9.0);

            if !state.bluetooth_powered || state.bluetooth_devices.is_empty() {
                let message = if state.bluetooth_powered {
                    "No known devices found. Refresh to scan."
                } else {
                    "Turn on Bluetooth to discover devices."
                };
                font.draw_text(canvas, 36.0, 190.0, message, colors.text, 10.0);
            } else {
                for (index, device) in state.bluetooth_devices.iter().take(6).enumerate() {
                    let y = 164.0 + index as f32 * 46.0;
                    canvas.rounded_rect_f(32.0, y, 312.0, 40.0, 12.0, colors.quick_card);
                    font.draw_text(
                        canvas,
                        44.0,
                        y + 16.0,
                        &truncate_label(&device.name, 34),
                        colors.text,
                        10.0,
                    );
                    font.draw_text(
                        canvas,
                        44.0,
                        y + 31.0,
                        &device.address,
                        colors.text_muted,
                        9.0,
                    );
                }
            }
        }
        ConnectivityPanel::KeyboardBacklight => {
            if let Some(backlight) = &state.keyboard_backlight {
                font.draw_text(
                    canvas,
                    36.0,
                    140.0,
                    &format!(
                        "{} · {}/{}",
                        backlight.device, backlight.current, backlight.maximum
                    ),
                    colors.text_muted,
                    10.0,
                );
                let low = (backlight.maximum / 2).max(1);
                let options: &[(u32, &str)] = if backlight.maximum >= 2 {
                    &[(0, "Off"), (low, "Low"), (backlight.maximum, "High")]
                } else {
                    &[(0, "Off"), (backlight.maximum, "On")]
                };
                for (index, (level, label)) in options.iter().enumerate() {
                    let x = 32.0 + index as f32 * 102.0;
                    canvas.rounded_rect_f(
                        x,
                        166.0,
                        92.0,
                        54.0,
                        15.0,
                        if backlight.current == *level {
                            colors.quick_active
                        } else {
                            colors.quick_card
                        },
                    );
                    font.draw_text(canvas, x + 24.0, 198.0, label, colors.text, 11.0);
                }
            } else {
                font.draw_text(
                    canvas,
                    36.0,
                    144.0,
                    "No supported backlit keyboard detected.",
                    colors.text_muted,
                    10.0,
                );
            }
        }
        ConnectivityPanel::Sound => {
            let system_volume = state.system_volume_percent.min(100);
            let output_volume = state.output_volume_percent.min(150);

            font.draw_text(canvas, 36.0, 142.0, "SYSTEM VOLUME", colors.text_muted, 9.0);
            font.draw_text(
                canvas,
                296.0,
                142.0,
                &format!("{system_volume}%"),
                colors.text,
                10.0,
            );
            canvas.rounded_rect_f(44.0, 158.0, 288.0, 5.0, 2.5, colors.quick_track);
            canvas.rounded_rect_f(
                44.0,
                158.0,
                288.0 * system_volume as f32 / 100.0,
                5.0,
                2.5,
                colors.accent,
            );
            let system_thumb = 44.0 + 288.0 * system_volume as f32 / 100.0;
            canvas.rounded_rect_f(system_thumb - 6.0, 152.0, 12.0, 17.0, 6.0, colors.text);

            font.draw_text(
                canvas,
                36.0,
                214.0,
                "OUTPUT DEVICE VOLUME",
                colors.text_muted,
                9.0,
            );
            font.draw_text(
                canvas,
                286.0,
                214.0,
                &format!("{output_volume}%"),
                colors.text,
                10.0,
            );
            canvas.rounded_rect_f(44.0, 230.0, 288.0, 5.0, 2.5, colors.quick_track);
            canvas.rounded_rect_f(
                44.0,
                230.0,
                288.0 * output_volume as f32 / 150.0,
                5.0,
                2.5,
                colors.accent,
            );
            let output_thumb = 44.0 + 288.0 * output_volume as f32 / 150.0;
            canvas.rounded_rect_f(output_thumb - 6.0, 224.0, 12.0, 17.0, 6.0, colors.text);

            let device_label = if state.output_is_bluetooth {
                format!("Bluetooth · {}", state.output_description)
            } else {
                state.output_description.clone()
            };
            font.draw_text(
                canvas,
                36.0,
                268.0,
                &truncate_label(&device_label, 45),
                colors.text,
                10.0,
            );
            font.draw_text(
                canvas,
                36.0,
                286.0,
                &format!("{} playback streams", state.audio_stream_count),
                colors.text_muted,
                9.0,
            );
            canvas.rounded_rect_f(32.0, 310.0, 132.0, 34.0, 17.0, colors.quick_card);
            font.draw_text(
                canvas,
                48.0,
                332.0,
                if state.output_muted {
                    "Unmute output"
                } else {
                    "Mute output"
                },
                colors.text,
                10.0,
            );
        }
        ConnectivityPanel::Brightness => {
            font.draw_text(
                canvas,
                36.0,
                144.0,
                "DISPLAY BRIGHTNESS",
                colors.text_muted,
                9.0,
            );
            if let Some(value) = state.brightness_percent {
                font.draw_text(
                    canvas,
                    296.0,
                    144.0,
                    &format!("{value}%"),
                    colors.text,
                    10.0,
                );
                canvas.rounded_rect_f(44.0, 174.0, 288.0, 5.0, 2.5, colors.quick_track);
                canvas.rounded_rect_f(
                    44.0,
                    174.0,
                    288.0 * value.min(100) as f32 / 100.0,
                    5.0,
                    2.5,
                    colors.accent,
                );
                let thumb = 44.0 + 288.0 * value.min(100) as f32 / 100.0;
                canvas.rounded_rect_f(thumb - 6.0, 168.0, 12.0, 17.0, 6.0, colors.text);
            } else {
                font.draw_text(
                    canvas,
                    36.0,
                    174.0,
                    "Brightness control unavailable",
                    colors.text_muted,
                    10.0,
                );
            }
        }
        ConnectivityPanel::Appearance => {
            font.draw_text(
                canvas,
                36.0,
                144.0,
                "DISPLAY PREFERENCES",
                colors.text_muted,
                9.0,
            );
            font.draw_text(
                canvas,
                36.0,
                174.0,
                "Theme options will be added here.",
                colors.text,
                10.0,
            );
        }
        ConnectivityPanel::Power => {
            font.draw_text(canvas, 36.0, 144.0, "POWER OPTIONS", colors.text_muted, 9.0);
            font.draw_text(
                canvas,
                36.0,
                174.0,
                "Session actions will be added here.",
                colors.text,
                10.0,
            );
        }
        ConnectivityPanel::None | ConnectivityPanel::Workspaces => {}
    }
}

fn truncate_label(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let result: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{result}…")
    } else {
        result
    }
}

fn draw_tile(
    canvas: &mut Canvas<'_>,
    font: &FontRenderer,
    colors: &Theme,
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
        if active { 0xFF287FC5 } else { 0x704A4A52 },
    );

    font.draw_text(canvas, x + 14.0, y + 28.0, title, colors.text, 15.0);
    font.draw_text(canvas, x + 14.0, y + 49.0, status, colors.text_muted, 10.0);

    let switch_x = x + width - 43.0;
    canvas.rounded_rect_f(
        switch_x,
        y + 22.0,
        30.0,
        18.0,
        9.0,
        if active { 0xFF174F7D } else { 0xFF343A43 },
    );
    canvas.rounded_rect_f(
        if active {
            switch_x + 15.0
        } else {
            switch_x + 3.0
        },
        y + 25.0,
        12.0,
        12.0,
        6.0,
        colors.text,
    );
}

fn draw_connectivity_tile(
    canvas: &mut Canvas<'_>,
    font: &FontRenderer,
    colors: &Theme,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    title: &str,
    status: &str,
    active: bool,
) {
    let arrow_width = 48.0;
    let arrow_x = x + width - arrow_width;

    // Main tile communicates the power state.
    canvas.rounded_rect_f(
        x,
        y,
        width,
        height,
        17.0,
        if active { 0xFF287FC5 } else { 0x704A4A52 },
    );

    // The arrow occupies an independent, visibly separated hit target.
    canvas.rounded_rect_f(
        arrow_x,
        y,
        arrow_width,
        height,
        17.0,
        if active { 0xFF176DAE } else { 0x80565C66 },
    );

    // Divider between the power-toggle area and the details button.
    canvas.rounded_rect_f(arrow_x, y + 12.0, 1.5, height - 24.0, 0.75, 0x55FFFFFF);

    font.draw_text(canvas, x + 14.0, y + 28.0, title, colors.text, 15.0);
    font.draw_text(
        canvas,
        x + 14.0,
        y + 49.0,
        &truncate_label(status, 14),
        colors.text_muted,
        10.0,
    );

    font.draw_text(canvas, arrow_x + 17.0, y + 41.0, ">", colors.text, 19.0);
}

pub fn hit_test(
    width: u32,
    _height: u32,
    x: f64,
    y: f64,
    workspace_count: u32,
    expanded: ConnectivityPanel,
    state: &ControlState,
    workspaces_hovered: bool,
    add_workspace_hovered: bool,
) -> Option<ControlAction> {
    if !matches!(
        expanded,
        ConnectivityPanel::None | ConnectivityPanel::Workspaces
    ) {
        let inside_popup = (20.0..(width as f64 - 20.0)).contains(&x)
            && (76.0..(_height as f64 - 20.0)).contains(&y);

        if !inside_popup {
            return Some(ControlAction::BackToControls);
        }
    }

    let close_left = width.saturating_sub(50) as f64;
    let close_right = width.saturating_sub(6) as f64;

    if (close_left..close_right).contains(&x) && (14.0..44.0).contains(&y) {
        return Some(ControlAction::Close);
    }

    let workspace_left = if workspaces_hovered { 176.0 } else { 212.0 };
    let workspace_width = if workspaces_hovered { 72.0 } else { 36.0 };
    if (workspace_left..workspace_left + workspace_width).contains(&x) && (14.0..44.0).contains(&y)
    {
        return Some(ControlAction::OpenWorkspaces);
    }

    let add_left = if add_workspace_hovered { 254.0 } else { 282.0 };
    let add_width = if add_workspace_hovered { 64.0 } else { 36.0 };
    if (add_left..add_left + add_width).contains(&x) && (14.0..44.0).contains(&y) {
        return Some(ControlAction::CreateWorkspace);
    }

    if expanded == ConnectivityPanel::Workspaces {
        if (16.0..128.0).contains(&x) && (72.0..108.0).contains(&y) {
            return Some(ControlAction::BackToControls);
        }

        if (16.0..360.0).contains(&x) && (166.0..242.0).contains(&y) {
            let column = ((x - 16.0) / 69.0) as u32;
            let row = ((y - 166.0) / 42.0) as u32;
            let local_x = (x - 16.0) - column as f64 * 69.0;
            let local_y = (y - 166.0) - row as f64 * 42.0;
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

        if (16.0..360.0).contains(&x) && (260.0..296.0).contains(&y) {
            return Some(ControlAction::CreateWorkspace);
        }

        return None;
    }

    if expanded != ConnectivityPanel::None {
        let inside_popup = (20.0..(width as f64 - 20.0)).contains(&x)
            && (76.0..(_height as f64 - 20.0)).contains(&y);

        if !inside_popup {
            return Some(ControlAction::BackToControls);
        }

        match expanded {
            ConnectivityPanel::Wifi | ConnectivityPanel::Bluetooth => {
                if (260.0..336.0).contains(&x) && (88.0..116.0).contains(&y) {
                    return Some(ControlAction::RefreshConnectivity);
                }
            }
            ConnectivityPanel::KeyboardBacklight => {
                if let Some(backlight) = &state.keyboard_backlight {
                    let low = (backlight.maximum / 2).max(1);
                    let levels: &[u32] = if backlight.maximum >= 2 {
                        &[0, low, backlight.maximum]
                    } else {
                        &[0, backlight.maximum]
                    };

                    for (index, level) in levels.iter().enumerate() {
                        let left = 32.0 + index as f64 * 102.0;
                        if (left..left + 92.0).contains(&x) && (166.0..220.0).contains(&y) {
                            return Some(ControlAction::SetKeyboardBacklight(*level));
                        }
                    }
                }
            }
            ConnectivityPanel::Sound => {
                if (44.0..332.0).contains(&x) && (148.0..176.0).contains(&y) {
                    let percent = (((x - 44.0) / 288.0) * 100.0).round().clamp(0.0, 100.0) as u32;
                    return Some(ControlAction::SetSystemVolumePercent(percent));
                }

                if (44.0..332.0).contains(&x) && (220.0..242.0).contains(&y) {
                    let percent = (((x - 44.0) / 288.0) * 150.0).round().clamp(0.0, 150.0) as u32;
                    return Some(ControlAction::SetOutputVolumePercent(percent));
                }

                if (32.0..164.0).contains(&x) && (310.0..344.0).contains(&y) {
                    return Some(ControlAction::ToggleMute);
                }
            }
            ConnectivityPanel::Brightness => {
                if (44.0..332.0).contains(&x) && (164.0..190.0).contains(&y) {
                    let percent = (((x - 44.0) / 288.0) * 100.0).round().clamp(0.0, 100.0) as u32;
                    return Some(ControlAction::SetBrightnessPercent(percent));
                }
            }
            ConnectivityPanel::Appearance | ConnectivityPanel::Power => {}
            ConnectivityPanel::None | ConnectivityPanel::Workspaces => unreachable!(),
        }

        // Consume clicks on the popup surface; never fall through to controls below.
        return None;
    }

    if (16.0..182.0).contains(&x) && (72.0..138.0).contains(&y) {
        return Some(if x >= 134.0 {
            ControlAction::OpenWifiDetails
        } else {
            ControlAction::ToggleWifi
        });
    }

    if (194.0..360.0).contains(&x) && (72.0..138.0).contains(&y) {
        return Some(if x >= 312.0 {
            ControlAction::OpenBluetoothDetails
        } else {
            ControlAction::ToggleBluetooth
        });
    }

    if (16.0..360.0).contains(&x) && (150.0..232.0).contains(&y) {
        if (26.0..96.0).contains(&x) && (194.0..228.0).contains(&y) {
            return Some(ControlAction::ToggleMute);
        }
        return Some(ControlAction::OpenSound);
    }

    if (16.0..360.0).contains(&x) && (244.0..302.0).contains(&y) {
        return Some(ControlAction::OpenBrightness);
    }

    if (16.0..182.0).contains(&x) && (330.0..386.0).contains(&y) {
        return Some(ControlAction::OpenAppearance);
    }

    if state.keyboard_backlight.is_some() {
        if (194.0..360.0).contains(&x) && (330.0..386.0).contains(&y) {
            return Some(ControlAction::OpenKeyboardBacklight);
        }
        if (16.0..182.0).contains(&x) && (396.0..452.0).contains(&y) {
            return Some(ControlAction::OpenPower);
        }
    } else if (194.0..360.0).contains(&x) && (330.0..386.0).contains(&y) {
        return Some(ControlAction::OpenPower);
    }

    None
}
