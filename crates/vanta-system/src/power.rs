//! Read-only power-state integration using UPower, logind and PowerProfiles.

use std::collections::HashMap;

use serde::de::DeserializeOwned;
use zbus::OwnedVariant;
use zbus::{Connection, Proxy};

#[derive(Debug, Clone)]
pub struct BatteryInfo {
    pub present: bool,
    pub state: BatteryState,
    pub percentage: f64,
    pub energy_wh: f64,
    pub energy_full_wh: f64,
    pub energy_rate_w: f64,
    pub time_to_empty_seconds: Option<u64>,
    pub time_to_full_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    Unknown,
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    PendingCharge,
    PendingDischarge,
}

impl From<u32> for BatteryState {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::FullyCharged,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PowerCapabilities {
    pub suspend: PowerActionCapability,
    pub hibernate: PowerActionCapability,
    pub power_off: PowerActionCapability,
    pub reboot: PowerActionCapability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerActionCapability {
    Yes,
    No,
    Challenge,
    NotApplicable,
    Unknown,
}

impl From<String> for PowerActionCapability {
    fn from(value: String) -> Self {
        match value.as_str() {
            "yes" => Self::Yes,
            "no" => Self::No,
            "challenge" => Self::Challenge,
            "na" => Self::NotApplicable,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PowerProfile {
    pub name: String,
    pub driver: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PowerProfileState {
    pub active_profile: String,
    pub profiles: Vec<PowerProfile>,
    pub performance_degraded: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PowerSnapshot {
    pub on_battery: bool,
    pub lid_present: bool,
    pub lid_closed: bool,
    pub batteries: Vec<BatteryInfo>,
    pub capabilities: PowerCapabilities,
    pub profiles: Option<PowerProfileState>,
}

async fn property<T>(proxy: &Proxy<'_>, name: &str) -> Result<T, String>
where
    T: DeserializeOwned + zbus::zvariant::Type,
{
    proxy
        .get_property(name)
        .await
        .map_err(|error| format!("{name}: {error}"))
}

async fn optional_property<T>(proxy: &Proxy<'_>, name: &str) -> Option<T>
where
    T: DeserializeOwned + zbus::zvariant::Type,
{
    property(proxy, name).await.ok()
}

async fn read_battery_devices(connection: &Connection) -> Result<Vec<BatteryInfo>, String> {
    let root = Proxy::new(
        connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .await
    .map_err(|error| error.to_string())?;

    let paths: Vec<zbus::OwnedObjectPath> = root
        .call("EnumerateDevices", &())
        .await
        .map_err(|error| format!("UPower EnumerateDevices: {error}"))?;

    let mut batteries = Vec::new();

    for path in paths {
        let device = Proxy::new(
            connection,
            "org.freedesktop.UPower",
            path.as_str(),
            "org.freedesktop.UPower.Device",
        )
        .await
        .map_err(|error| error.to_string())?;

        let device_type: u32 = match property(&device, "Type").await {
            Ok(value) => value,
            Err(_) => continue,
        };

        // UPower device type 2 is Battery.
        if device_type != 2 {
            continue;
        }

        let present = optional_property(&device, "IsPresent")
            .await
            .unwrap_or(false);

        if !present {
            continue;
        }

        let state = optional_property::<u32>(&device, "State")
            .await
            .unwrap_or(0)
            .into();

        let battery = BatteryInfo {
            present,
            state,
            percentage: optional_property(&device, "Percentage")
                .await
                .unwrap_or(0.0),
            energy_wh: optional_property(&device, "Energy").await.unwrap_or(0.0),
            energy_full_wh: optional_property(&device, "EnergyFull")
                .await
                .unwrap_or(0.0),
            energy_rate_w: optional_property(&device, "EnergyRate")
                .await
                .unwrap_or(0.0),
            time_to_empty_seconds: if state == BatteryState::Discharging {
                optional_property::<i64>(&device, "TimeToEmpty")
                    .await
                    .filter(|seconds| *seconds > 0)
                    .map(|seconds| seconds as u64)
            } else {
                None
            },
            time_to_full_seconds: if matches!(
                state,
                BatteryState::Charging | BatteryState::PendingCharge
            ) {
                optional_property::<i64>(&device, "TimeToFull")
                    .await
                    .filter(|seconds| *seconds > 0)
                    .map(|seconds| seconds as u64)
            } else {
                None
            },
        };

        batteries.push(battery);
    }

    Ok(batteries)
}

async fn read_capabilities(connection: &Connection) -> Result<PowerCapabilities, String> {
    let proxy = Proxy::new(
        connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|error| error.to_string())?;

    async fn can(proxy: &Proxy<'_>, method: &str) -> PowerActionCapability {
        proxy
            .call::<_, _, String>(method, &())
            .await
            .map(PowerActionCapability::from)
            .unwrap_or(PowerActionCapability::Unknown)
    }

    Ok(PowerCapabilities {
        suspend: can(&proxy, "CanSuspend").await,
        hibernate: can(&proxy, "CanHibernate").await,
        power_off: can(&proxy, "CanPowerOff").await,
        reboot: can(&proxy, "CanReboot").await,
    })
}

async fn read_profiles(connection: &Connection) -> Result<PowerProfileState, String> {
    let proxy = Proxy::new(
        connection,
        "net.hadess.PowerProfiles",
        "/net/hadess/PowerProfiles",
        "net.hadess.PowerProfiles",
    )
    .await
    .map_err(|error| error.to_string())?;

    let active_profile: String = property(&proxy, "ActiveProfile").await?;

    let raw_profiles: Vec<HashMap<String, OwnedVariant>> = property(&proxy, "Profiles").await?;

    let profiles = raw_profiles
        .into_iter()
        .filter_map(|entry| {
            let name = entry
                .get("Profile")
                .and_then(|value| String::try_from(value.clone()).ok())?;

            let driver = entry
                .get("Driver")
                .and_then(|value| String::try_from(value.clone()).ok());

            Some(PowerProfile { name, driver })
        })
        .collect();

    let performance_degraded = optional_property(&proxy, "PerformanceDegraded").await;

    Ok(PowerProfileState {
        active_profile,
        profiles,
        performance_degraded,
    })
}

pub(crate) async fn read_power_snapshot(connection: &Connection) -> Result<PowerSnapshot, String> {
    let upower = Proxy::new(
        connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .await
    .map_err(|error| error.to_string())?;

    let on_battery = property(&upower, "OnBattery").await?;
    let lid_present = property(&upower, "LidIsPresent").await?;
    let lid_closed = property(&upower, "LidIsClosed").await?;

    let batteries = read_battery_devices(connection).await?;
    let capabilities = read_capabilities(connection).await?;

    let profiles = read_profiles(connection).await.ok();

    Ok(PowerSnapshot {
        on_battery,
        lid_present,
        lid_closed,
        batteries,
        capabilities,
        profiles,
    })
}
