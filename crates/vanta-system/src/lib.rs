//! System D-Bus integration for Vanta.
//!
//! This crate discovers existing Linux system services. It does not
//! replace or manage those services.

use zbus::Connection;

const DBUS_DESTINATION: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    Available,
    Unavailable,
    Unknown(String),
}

impl ServiceStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub name: &'static str,
    pub bus_name: &'static str,
    pub status: ServiceStatus,
}

pub mod bluetooth;
pub mod bluetooth_events;
pub mod network;
pub mod power;
pub mod storage;
pub mod storage_events;
pub mod storage_state;

#[derive(Debug, Clone)]
pub struct SystemServices {
    pub upower: ServiceInfo,
    pub logind: ServiceInfo,
    pub power_profiles: ServiceInfo,
    pub network_manager: ServiceInfo,
    pub bluez: ServiceInfo,
    pub udisks2: ServiceInfo,
    pub tuned: ServiceInfo,
}

pub struct SystemBus {
    connection: Connection,
}

impl SystemBus {
    /// Connect to the system D-Bus.
    pub fn connect() -> Result<Self, String> {
        let connection = zbus::block_on(Connection::system())
            .map_err(|error| format!("System D-Bus connection failed: {error}"))?;

        Ok(Self { connection })
    }

    /// Check whether a well-known D-Bus name currently has an owner.
    pub fn service_status(&self, bus_name: &'static str) -> ServiceStatus {
        let result = zbus::block_on(self.connection.call_method(
            Some(DBUS_DESTINATION),
            DBUS_PATH,
            Some(DBUS_INTERFACE),
            "NameHasOwner",
            &(bus_name,),
        ));

        match result {
            Ok(reply) => match reply.body().deserialize::<bool>() {
                Ok(true) => ServiceStatus::Available,
                Ok(false) => ServiceStatus::Unavailable,
                Err(error) => ServiceStatus::Unknown(error.to_string()),
            },
            Err(error) => ServiceStatus::Unknown(error.to_string()),
        }
    }

    /// Subscribe to asynchronous NetworkManager events.
    pub fn subscribe_network_events(
        &self,
    ) -> Result<std::sync::mpsc::Receiver<network::NetworkEvent>, String> {
        network::spawn_network_monitor()
    }

    /// Subscribe to asynchronous BlueZ adapter and device events.
    pub fn subscribe_bluetooth_events(
        &self,
    ) -> Result<std::sync::mpsc::Receiver<bluetooth_events::BluetoothEvent>, String> {
        bluetooth_events::spawn_bluetooth_monitor()
    }

    pub fn network_snapshot(&self) -> Result<network::NetworkSnapshot, String> {
        zbus::block_on(network::read_network_snapshot(&self.connection))
    }

    pub fn power_snapshot(&self) -> Result<power::PowerSnapshot, String> {
        zbus::block_on(power::read_power_snapshot(&self.connection))
    }

    pub fn bluetooth_snapshot(&self) -> Result<bluetooth::BluetoothSnapshot, String> {
        zbus::block_on(bluetooth::read_bluetooth_snapshot(&self.connection))
    }

    pub fn storage_snapshot(&self) -> Result<storage::StorageSnapshot, String> {
        zbus::block_on(storage::read_storage_snapshot(&self.connection))
    }
    pub fn subscribe_storage_events(
        &self,
    ) -> Result<std::sync::mpsc::Receiver<storage_events::StorageEvent>, String> {
        storage_events::spawn_storage_monitor()
    }

    /// Discover the system services relevant to Vanta.
    pub fn discover_services(&self) -> SystemServices {
        fn inspect(bus: &SystemBus, name: &'static str, bus_name: &'static str) -> ServiceInfo {
            ServiceInfo {
                name,
                bus_name,
                status: bus.service_status(bus_name),
            }
        }

        SystemServices {
            upower: inspect(self, "UPower", "org.freedesktop.UPower"),
            logind: inspect(self, "systemd-logind", "org.freedesktop.login1"),
            power_profiles: inspect(self, "Power Profiles", "net.hadess.PowerProfiles"),
            network_manager: inspect(self, "NetworkManager", "org.freedesktop.NetworkManager"),
            bluez: inspect(self, "BlueZ", "org.bluez"),
            udisks2: inspect(self, "UDisks2", "org.freedesktop.UDisks2"),
            tuned: inspect(self, "TuneD", "com.redhat.tuned"),
        }
    }
}
