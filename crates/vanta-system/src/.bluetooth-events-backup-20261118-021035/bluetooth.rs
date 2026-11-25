//! Read-only Bluetooth integration through BlueZ.
//!
//! BlueZ owns Bluetooth hardware and connection management.
//! Vanta discovers adapters and known devices through system D-Bus.
//! This module does not initiate discovery, pairing, or connections.

use serde::de::DeserializeOwned;
use std::collections::HashMap;
use zbus::OwnedVariant;
use zbus::{Connection, OwnedObjectPath, Proxy};

const BLUEZ_BUS: &str = "org.bluez";
const OBJECT_MANAGER_PATH: &str = "/";
const OBJECT_MANAGER_INTERFACE: &str = "org.freedesktop.DBus.ObjectManager";
const ADAPTER_INTERFACE: &str = "org.bluez.Adapter1";
const DEVICE_INTERFACE: &str = "org.bluez.Device1";

#[derive(Debug, Clone)]
pub struct BluetoothSnapshot {
    pub adapters: Vec<BluetoothAdapter>,
    pub devices: Vec<BluetoothDevice>,
}

#[derive(Debug, Clone)]
pub struct BluetoothAdapter {
    pub object_path: String,
    pub address: String,
    pub address_type: Option<String>,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub powered: bool,
    pub discoverable: Option<bool>,
    pub discoverable_timeout_seconds: Option<u32>,
    pub pairable: Option<bool>,
    pub pairable_timeout_seconds: Option<u32>,
    pub discovering: Option<bool>,
    pub power_state: Option<String>,
    pub uuids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BluetoothDevice {
    pub object_path: String,
    pub adapter_path: String,
    pub address: String,
    pub address_type: Option<String>,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub icon: Option<String>,
    pub paired: bool,
    pub bonded: Option<bool>,
    pub trusted: bool,
    pub connected: bool,
    pub blocked: bool,
    pub rssi_dbm: Option<i16>,
    pub tx_power_dbm: Option<i16>,
    pub services_resolved: Option<bool>,
    pub uuids: Vec<String>,
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

async fn read_adapter(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> Result<BluetoothAdapter, String> {
    let proxy = Proxy::new(connection, BLUEZ_BUS, path.as_str(), ADAPTER_INTERFACE)
        .await
        .map_err(|error| format!("Adapter {}: {error}", path.as_str()))?;

    Ok(BluetoothAdapter {
        object_path: path.to_string(),
        address: property(&proxy, "Address").await?,
        address_type: optional_property(&proxy, "AddressType").await,
        name: optional_property(&proxy, "Name").await,
        alias: optional_property(&proxy, "Alias").await,
        powered: property(&proxy, "Powered").await?,
        discoverable: optional_property(&proxy, "Discoverable").await,
        discoverable_timeout_seconds: optional_property(&proxy, "DiscoverableTimeout").await,
        pairable: optional_property(&proxy, "Pairable").await,
        pairable_timeout_seconds: optional_property(&proxy, "PairableTimeout").await,
        discovering: optional_property(&proxy, "Discovering").await,
        power_state: optional_property(&proxy, "PowerState").await,
        uuids: optional_property(&proxy, "UUIDs").await.unwrap_or_default(),
    })
}

async fn read_device(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> Result<BluetoothDevice, String> {
    let proxy = Proxy::new(connection, BLUEZ_BUS, path.as_str(), DEVICE_INTERFACE)
        .await
        .map_err(|error| format!("Device {}: {error}", path.as_str()))?;

    let adapter: OwnedObjectPath = property(&proxy, "Adapter").await?;

    Ok(BluetoothDevice {
        object_path: path.to_string(),
        adapter_path: adapter.to_string(),
        address: property(&proxy, "Address").await?,
        address_type: optional_property(&proxy, "AddressType").await,
        name: optional_property(&proxy, "Name").await,
        alias: optional_property(&proxy, "Alias").await,
        icon: optional_property(&proxy, "Icon").await,
        paired: property(&proxy, "Paired").await?,
        bonded: optional_property(&proxy, "Bonded").await,
        trusted: property(&proxy, "Trusted").await?,
        connected: property(&proxy, "Connected").await?,
        blocked: property(&proxy, "Blocked").await?,
        rssi_dbm: optional_property(&proxy, "RSSI").await,
        tx_power_dbm: optional_property(&proxy, "TxPower").await,
        services_resolved: optional_property(&proxy, "ServicesResolved").await,
        uuids: optional_property(&proxy, "UUIDs").await.unwrap_or_default(),
    })
}

pub(crate) async fn read_bluetooth_snapshot(
    connection: &Connection,
) -> Result<BluetoothSnapshot, String> {
    let manager = Proxy::new(
        connection,
        BLUEZ_BUS,
        OBJECT_MANAGER_PATH,
        OBJECT_MANAGER_INTERFACE,
    )
    .await
    .map_err(|error| format!("BlueZ ObjectManager: {error}"))?;

    // The property values are retained only to deserialize the standard
    // ObjectManager response. We use the interface names to discover objects.
    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedVariant>>> = manager
        .call("GetManagedObjects", &())
        .await
        .map_err(|error| format!("BlueZ GetManagedObjects: {error}"))?;

    let mut adapters = Vec::new();
    let mut devices = Vec::new();

    for (path, interfaces) in objects {
        if interfaces.contains_key(ADAPTER_INTERFACE) {
            match read_adapter(connection, &path).await {
                Ok(adapter) => adapters.push(adapter),
                Err(error) => {
                    eprintln!("Vanta: skipping unreadable Bluetooth adapter: {error}");
                }
            }
        }

        if interfaces.contains_key(DEVICE_INTERFACE) {
            match read_device(connection, &path).await {
                Ok(device) => devices.push(device),
                Err(error) => {
                    eprintln!("Vanta: skipping unreadable Bluetooth device: {error}");
                }
            }
        }
    }

    adapters.sort_by(|a, b| a.object_path.cmp(&b.object_path));
    devices.sort_by(|a, b| a.address.cmp(&b.address));

    Ok(BluetoothSnapshot { adapters, devices })
}
