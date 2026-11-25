//! Read-only NetworkManager integration.
//!
//! NetworkManager owns network hardware and connection management.
//! Vanta discovers devices and observes their state through system D-Bus.

use serde::de::DeserializeOwned;
use zbus::OwnedObjectPath;
use zbus::{Connection, Proxy};

const NM_BUS: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_INTERFACE: &str = "org.freedesktop.NetworkManager";

const DEVICE_INTERFACE: &str = "org.freedesktop.NetworkManager.Device";
const WIFI_INTERFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const AP_INTERFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub manager_state: NetworkManagerState,
    pub connectivity: NetworkConnectivity,
    pub networking_enabled: bool,
    pub wifi_enabled: bool,
    pub wifi_hardware_enabled: Option<bool>,
    pub devices: Vec<NetworkDevice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkManagerState {
    Unknown,
    Asleep,
    Disconnected,
    Disconnecting,
    Connecting,
    ConnectedLocal,
    ConnectedSite,
    ConnectedGlobal,
    Other(u32),
}

impl From<u32> for NetworkManagerState {
    fn from(value: u32) -> Self {
        match value {
            10 => Self::Asleep,
            20 => Self::Disconnected,
            30 => Self::Disconnecting,
            40 => Self::Connecting,
            50 => Self::ConnectedLocal,
            60 => Self::ConnectedSite,
            70 => Self::ConnectedGlobal,
            0 => Self::Unknown,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkConnectivity {
    Unknown,
    None,
    Portal,
    Limited,
    Full,
    Other(u32),
}

impl From<u32> for NetworkConnectivity {
    fn from(value: u32) -> Self {
        match value {
            0 => Self::Unknown,
            1 => Self::None,
            2 => Self::Portal,
            3 => Self::Limited,
            4 => Self::Full,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NetworkDevice {
    pub object_path: String,
    pub interface: String,
    pub ip_interface: Option<String>,
    pub device_type: NetworkDeviceType,
    pub raw_device_type: u32,
    pub state: NetworkDeviceState,
    pub raw_state: u32,
    pub managed: bool,
    pub hardware_address: Option<String>,
    pub driver: Option<String>,
    pub mtu: Option<u32>,
    pub active_connection: Option<String>,
    pub wifi: Option<WifiState>,
    pub link_connected: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkDeviceType {
    Ethernet,
    Wifi,
    Bluetooth,
    Bridge,
    Tunnel,
    Veth,
    WifiP2p,
    Loopback,
    Modem,
    Other(u32),
}

impl From<u32> for NetworkDeviceType {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::Ethernet,
            2 => Self::Wifi,
            5 => Self::Bluetooth,
            8 => Self::Modem,
            13 => Self::Bridge,
            16 => Self::Tunnel,
            20 => Self::Veth,
            30 => Self::WifiP2p,
            32 => Self::Loopback,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkDeviceState {
    Unknown,
    Unmanaged,
    Unavailable,
    Disconnected,
    Prepare,
    Config,
    NeedAuth,
    IpConfig,
    IpCheck,
    Secondaries,
    Activated,
    Deactivating,
    Failed,
    Other(u32),
}

impl From<u32> for NetworkDeviceState {
    fn from(value: u32) -> Self {
        match value {
            0 => Self::Unknown,
            10 => Self::Unmanaged,
            20 => Self::Unavailable,
            30 => Self::Disconnected,
            40 => Self::Prepare,
            50 => Self::Config,
            60 => Self::NeedAuth,
            70 => Self::IpConfig,
            80 => Self::IpCheck,
            90 => Self::Secondaries,
            100 => Self::Activated,
            110 => Self::Deactivating,
            120 => Self::Failed,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WifiState {
    pub active_access_point: Option<String>,
    pub ssid: Option<String>,
    pub signal_strength: Option<u8>,
    pub frequency_mhz: Option<u32>,
    pub bitrate_kbps: Option<u32>,
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

async fn read_wifi_state(connection: &Connection, device_path: &str) -> Result<WifiState, String> {
    let wifi = Proxy::new(connection, NM_BUS, device_path, WIFI_INTERFACE)
        .await
        .map_err(|error| format!("Wi-Fi device proxy: {error}"))?;

    let access_point: OwnedObjectPath = property(&wifi, "ActiveAccessPoint").await?;

    let active_path = access_point.as_str();

    if active_path == "/" {
        return Ok(WifiState {
            active_access_point: None,
            ssid: None,
            signal_strength: None,
            frequency_mhz: None,
            bitrate_kbps: optional_property(&wifi, "Bitrate").await,
        });
    }

    let ap = Proxy::new(connection, NM_BUS, active_path, AP_INTERFACE)
        .await
        .map_err(|error| format!("Access point proxy: {error}"))?;

    let raw_ssid: Option<Vec<u8>> = optional_property(&ap, "Ssid").await;

    let ssid = raw_ssid.and_then(|bytes| {
        if bytes.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
    });

    Ok(WifiState {
        active_access_point: Some(active_path.to_string()),
        ssid,
        signal_strength: optional_property(&ap, "Strength").await,
        frequency_mhz: optional_property(&ap, "Frequency").await,
        bitrate_kbps: optional_property(&wifi, "Bitrate").await,
    })
}

async fn read_device(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> Result<NetworkDevice, String> {
    let device = Proxy::new(connection, NM_BUS, path.as_str(), DEVICE_INTERFACE)
        .await
        .map_err(|error| format!("Device {}: {error}", path.as_str()))?;

    let interface: String = property(&device, "Interface").await?;
    let raw_device_type: u32 = property(&device, "DeviceType").await?;
    let raw_state: u32 = property(&device, "State").await?;

    let wifi = if raw_device_type == 2 {
        match read_wifi_state(connection, path.as_str()).await {
            Ok(state) => Some(state),
            Err(error) => {
                eprintln!("Vanta: Wi-Fi details unavailable for {interface}: {error}");
                None
            }
        }
    } else {
        None
    };

    let link_connected = if raw_device_type == 1 {
        let wired = Proxy::new(
            connection,
            NM_BUS,
            path.as_str(),
            "org.freedesktop.NetworkManager.Device.Wired",
        )
        .await
        .ok();
        match wired {
            Some(proxy) => optional_property::<bool>(&proxy, "Carrier").await,
            None => None,
        }
    } else {
        None
    };

    let active_connection: OwnedObjectPath = optional_property(&device, "ActiveConnection")
        .await
        .unwrap_or_else(|| OwnedObjectPath::try_from("/").unwrap());

    let active_connection = if active_connection.as_str() == "/" {
        None
    } else {
        Some(active_connection.to_string())
    };

    Ok(NetworkDevice {
        object_path: path.to_string(),
        interface,
        ip_interface: optional_property(&device, "IpInterface").await,
        device_type: raw_device_type.into(),
        raw_device_type,
        state: raw_state.into(),
        raw_state,
        managed: property(&device, "Managed").await.unwrap_or(false),
        hardware_address: optional_property(&device, "HwAddress").await,
        driver: optional_property(&device, "Driver").await,
        mtu: optional_property(&device, "Mtu").await,
        active_connection,
        wifi,
        link_connected,
    })
}

pub(crate) async fn read_network_snapshot(
    connection: &Connection,
) -> Result<NetworkSnapshot, String> {
    let manager = Proxy::new(connection, NM_BUS, NM_PATH, NM_INTERFACE)
        .await
        .map_err(|error| format!("NetworkManager proxy: {error}"))?;

    let raw_state: u32 = property(&manager, "State").await?;
    let raw_connectivity: u32 = property(&manager, "Connectivity").await?;

    let networking_enabled = property(&manager, "NetworkingEnabled").await?;
    let wifi_enabled = property(&manager, "WirelessEnabled").await?;

    let wifi_hardware_enabled = optional_property(&manager, "WirelessHardwareEnabled").await;

    let paths: Vec<OwnedObjectPath> = manager
        .call("GetDevices", &())
        .await
        .map_err(|error| format!("NetworkManager GetDevices: {error}"))?;

    let mut devices = Vec::with_capacity(paths.len());

    for path in paths {
        match read_device(connection, &path).await {
            Ok(device) => devices.push(device),
            Err(error) => {
                eprintln!("Vanta: skipping unreadable network device: {error}");
            }
        }
    }

    Ok(NetworkSnapshot {
        manager_state: raw_state.into(),
        connectivity: raw_connectivity.into(),
        networking_enabled,
        wifi_enabled,
        wifi_hardware_enabled,
        devices,
    })
}

/// NetworkManager events delivered independently of UI rendering.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    ManagerStateChanged {
        state: NetworkManagerState,
    },
    ConnectivityChanged {
        connectivity: NetworkConnectivity,
    },
    DeviceAdded {
        object_path: String,
    },
    DeviceRemoved {
        object_path: String,
    },
    DeviceStateChanged {
        object_path: String,
        interface: String,
        state: NetworkDeviceState,
        old_state: NetworkDeviceState,
        reason: u32,
    },
    EthernetCarrierChanged {
        object_path: String,
        interface: String,
        connected: bool,
    },
    WifiAccessPointChanged {
        object_path: String,
        interface: String,
        active_access_point: Option<String>,
    },
    WifiSignalStrengthChanged {
        object_path: String,
        interface: String,
        access_point: String,
        strength: u8,
    },
    MonitorError {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubscriptionToken {
    device_generation: u64,
    subscription_generation: u64,
}

#[derive(Debug, Clone, Copy)]
struct RetryTicket {
    device_generation: u64,
    retry_generation: u64,
    delay: std::time::Duration,
}

struct DeviceLifecycle {
    generation: u64,
    active_stream: Option<u64>,
    pending_retry: Option<u64>,
    retry_attempt: u32,
}

struct DeviceRegistry {
    next_device_generation: u64,
    next_subscription_generation: u64,
    next_retry_generation: u64,
    devices: std::collections::HashMap<String, DeviceLifecycle>,
}

impl DeviceRegistry {
    fn new() -> Self {
        Self {
            next_device_generation: 1,
            next_subscription_generation: 1,
            next_retry_generation: 1,
            devices: std::collections::HashMap::new(),
        }
    }

    fn allocate(counter: &mut u64) -> u64 {
        let value = *counter;
        *counter = (*counter).wrapping_add(1).max(1);
        value
    }

    fn retry_delay(attempt: u32) -> std::time::Duration {
        let seconds = match attempt {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 8,
            _ => 30,
        };

        std::time::Duration::from_secs(seconds)
    }

    fn add_device(&mut self, path: &str) -> Option<SubscriptionToken> {
        if self.devices.contains_key(path) {
            return None;
        }

        let generation = Self::allocate(&mut self.next_device_generation);

        self.devices.insert(
            path.to_owned(),
            DeviceLifecycle {
                generation,
                active_stream: None,
                pending_retry: None,
                retry_attempt: 0,
            },
        );

        self.begin_subscription(path)
    }

    fn begin_subscription(&mut self, path: &str) -> Option<SubscriptionToken> {
        let device = self.devices.get_mut(path)?;

        if device.active_stream.is_some() || device.pending_retry.is_some() {
            return None;
        }

        let subscription_generation = Self::allocate(&mut self.next_subscription_generation);

        device.active_stream = Some(subscription_generation);

        Some(SubscriptionToken {
            device_generation: device.generation,
            subscription_generation,
        })
    }

    fn subscription_succeeded(&mut self, path: &str, token: SubscriptionToken) -> bool {
        let Some(device) = self.devices.get_mut(path) else {
            return false;
        };

        if device.generation != token.device_generation
            || device.active_stream != Some(token.subscription_generation)
        {
            return false;
        }

        device.retry_attempt = 0;
        true
    }

    fn subscription_failed(&mut self, path: &str, token: SubscriptionToken) -> Option<RetryTicket> {
        let device = self.devices.get_mut(path)?;

        if device.generation != token.device_generation
            || device.active_stream != Some(token.subscription_generation)
            || device.pending_retry.is_some()
        {
            return None;
        }

        device.active_stream = None;

        let delay = Self::retry_delay(device.retry_attempt);
        device.retry_attempt = device.retry_attempt.saturating_add(1);

        let retry_generation = Self::allocate(&mut self.next_retry_generation);
        device.pending_retry = Some(retry_generation);

        Some(RetryTicket {
            device_generation: device.generation,
            retry_generation,
            delay,
        })
    }

    fn retry_due(&mut self, path: &str, ticket: RetryTicket) -> Option<SubscriptionToken> {
        {
            let device = self.devices.get_mut(path)?;

            if device.generation != ticket.device_generation
                || device.pending_retry != Some(ticket.retry_generation)
                || device.active_stream.is_some()
            {
                return None;
            }

            device.pending_retry = None;
        }

        self.begin_subscription(path)
    }

    fn remove_device(&mut self, path: &str) {
        self.devices.remove(path);
    }
}

enum MonitorEvent {
    Network(NetworkEvent),
    DeviceStreamEnded {
        object_path: String,
        token: SubscriptionToken,
    },
    RetryDevice {
        object_path: String,
        ticket: RetryTicket,
    },
}
/// Start a dedicated system-D-Bus event monitor.
///
/// Obtain the initial state separately through `SystemBus::network_snapshot()`.
/// Initial property values are deliberately discarded so startup does not
/// generate connection animations.
pub fn spawn_network_monitor() -> Result<std::sync::mpsc::Receiver<NetworkEvent>, String> {
    use std::sync::mpsc;
    use std::thread;

    let (sender, receiver) = mpsc::channel();

    thread::Builder::new()
        .name("vanta-network-events".into())
        .spawn(move || {
            let result = zbus::block_on(async {
                let connection = Connection::system()
                    .await
                    .map_err(|error| format!("Network event D-Bus connection: {error}"))?;

                run_network_monitor(&connection, &sender).await
            });

            if let Err(error) = result {
                let _ = sender.send(NetworkEvent::MonitorError { message: error });
            }
        })
        .map_err(|error| format!("Could not start network event thread: {error}"))?;

    Ok(receiver)
}

async fn create_wifi_ap_strength_stream(
    connection: &Connection,
    access_point_path: &str,
    device_path: &str,
    device_interface: &str,
) -> Result<
    (
        futures_util::stream::AbortHandle,
        futures_util::stream::BoxStream<'static, NetworkEvent>,
    ),
    String,
> {
    use futures_util::stream::{AbortHandle, Abortable, StreamExt};

    let access_point = Proxy::new_owned(
        connection.clone(),
        NM_BUS,
        access_point_path.to_owned(),
        AP_INTERFACE,
    )
    .await
    .map_err(|error| format!("Access-point proxy {access_point_path}: {error}"))?;

    // zbus 6.3 returns PropertyStream directly; it is not a Result.
    let changes = access_point
        .receive_property_changed::<u8>("Strength")
        .await;

    let ap_path = access_point_path.to_owned();
    let device_path = device_path.to_owned();
    let device_interface = device_interface.to_owned();

    let strength_stream = changes.skip(1).then(move |change| {
        let ap_path = ap_path.clone();
        let device_path = device_path.clone();
        let device_interface = device_interface.clone();

        async move {
            match change.get().await {
                Ok(strength) => NetworkEvent::WifiSignalStrengthChanged {
                    object_path: device_path,
                    interface: device_interface,
                    access_point: ap_path,
                    strength,
                },
                Err(error) => NetworkEvent::MonitorError {
                    message: format!("Could not read Wi-Fi strength for {ap_path}: {error}"),
                },
            }
        }
    });

    let (abort_handle, abort_registration) = AbortHandle::new_pair();

    let managed_stream = Abortable::new(strength_stream, abort_registration).boxed();

    Ok((abort_handle, managed_stream))
}

async fn create_device_event_stream(
    connection: &Connection,
    object_path: &str,
) -> Result<
    (
        futures_util::stream::AbortHandle,
        futures_util::stream::BoxStream<'static, NetworkEvent>,
    ),
    String,
> {
    use futures_util::stream::{AbortHandle, Abortable, StreamExt};

    let device = Proxy::new_owned(
        connection.clone(),
        NM_BUS,
        object_path.to_owned(),
        DEVICE_INTERFACE,
    )
    .await
    .map_err(|error| format!("Device proxy {object_path}: {error}"))?;

    let interface: String = device
        .get_property("Interface")
        .await
        .map_err(|error| format!("Read interface for {object_path}: {error}"))?;

    let device_type: u32 = device
        .get_property("DeviceType")
        .await
        .map_err(|error| format!("Read device type for {object_path}: {error}"))?;

    let state_path = object_path.to_owned();
    let state_interface = interface.clone();

    let state_stream = device
        .receive_signal("StateChanged")
        .await
        .map_err(|error| format!("Subscribe to device state {object_path}: {error}"))?
        .map(
            move |message| match message.body().deserialize::<(u32, u32, u32)>() {
                Ok((new_state, old_state, reason)) => NetworkEvent::DeviceStateChanged {
                    object_path: state_path.clone(),
                    interface: state_interface.clone(),
                    state: new_state.into(),
                    old_state: old_state.into(),
                    reason,
                },
                Err(error) => NetworkEvent::MonitorError {
                    message: format!(
                        "Invalid device StateChanged signal for {state_path}: {error}"
                    ),
                },
            },
        );

    let device_stream = match NetworkDeviceType::from(device_type) {
        NetworkDeviceType::Ethernet => {
            let carrier_path = object_path.to_owned();
            let carrier_interface = interface.clone();

            let carrier_stream = device
                .receive_property_changed::<bool>("Carrier")
                .await
                .skip(1)
                .then(move |change| {
                    let path = carrier_path.clone();
                    let interface = carrier_interface.clone();

                    async move {
                        match change.get().await {
                            Ok(connected) => NetworkEvent::EthernetCarrierChanged {
                                object_path: path,
                                interface,
                                connected,
                            },
                            Err(error) => NetworkEvent::MonitorError {
                                message: format!(
                                    "Could not read Ethernet carrier for {path}: {error}"
                                ),
                            },
                        }
                    }
                });

            futures_util::stream::select(state_stream, carrier_stream).boxed()
        }

        NetworkDeviceType::Wifi => {
            use futures_util::stream::{BoxStream, SelectAll, StreamExt};

            let wifi = Proxy::new_owned(
                connection.clone(),
                NM_BUS,
                object_path.to_owned(),
                WIFI_INTERFACE,
            )
            .await
            .map_err(|error| format!("Wi-Fi proxy {object_path}: {error}"))?;

            // Subscribe first, then read the current AP.
            // PropertyStream is returned directly by zbus 6.3.
            let access_point_changes = wifi
                .receive_property_changed::<OwnedObjectPath>("ActiveAccessPoint")
                .await;

            let initial_access_point: OwnedObjectPath = wifi
                .get_property("ActiveAccessPoint")
                .await
                .map_err(|error| {
                    format!("Read initial Wi-Fi access point {object_path}: {error}")
                })?;

            let wifi_path = object_path.to_owned();
            let wifi_interface = interface.clone();

            let access_point_stream = access_point_changes
                .skip(1)
                .then(move |change| {
                    let path = wifi_path.clone();
                    let interface = wifi_interface.clone();

                    async move {
                        match change.get().await {
                            Ok(access_point) => {
                                let value = access_point.as_str();

                                NetworkEvent::WifiAccessPointChanged {
                                    object_path: path,
                                    interface,
                                    active_access_point: if value == "/" {
                                        None
                                    } else {
                                        Some(value.to_owned())
                                    },
                                }
                            }
                            Err(error) => NetworkEvent::MonitorError {
                                message: format!(
                                    "Could not read active Wi-Fi access point for {path}: {error}"
                                ),
                            },
                        }
                    }
                })
                .boxed();

            let mut streams: SelectAll<BoxStream<'static, NetworkEvent>> = SelectAll::new();

            streams.push(access_point_stream);

            let mut strength_abort = None;
            let initial_path = initial_access_point.as_str();

            if initial_path != "/" {
                let (abort, strength_stream) = create_wifi_ap_strength_stream(
                    connection,
                    initial_path,
                    object_path,
                    &interface,
                )
                .await?;

                strength_abort = Some(abort);
                streams.push(strength_stream);
            }

            // When ActiveAccessPoint changes, abort the previous AP
            // subscription and attach a stream for the new AP.
            let device_stream =
                futures_util::stream::unfold(
                    (
                        streams,
                        strength_abort,
                        connection.clone(),
                        object_path.to_owned(),
                        interface.clone(),
                    ),
                    |(
                        mut streams,
                        mut strength_abort,
                        connection,
                        device_path,
                        device_interface,
                    )| async move {
                        loop {
                            let event = streams.next().await?;

                            if let NetworkEvent::WifiAccessPointChanged {
                                active_access_point,
                                ..
                            } = &event
                            {
                                if let Some(abort) = strength_abort.take() {
                                    abort.abort();
                                }

                                if let Some(access_point_path) = active_access_point.clone() {
                                    match create_wifi_ap_strength_stream(
                                        &connection,
                                        &access_point_path,
                                        &device_path,
                                        &device_interface,
                                    )
                                    .await
                                    {
                                        Ok((abort, strength_stream)) => {
                                            strength_abort = Some(abort);
                                            streams.push(strength_stream);
                                        }
                                        Err(error) => {
                                            let error_event =
                                                NetworkEvent::MonitorError { message: error };

                                            streams.push(
                                                futures_util::stream::once(
                                                    async move { error_event },
                                                )
                                                .boxed(),
                                            );
                                        }
                                    }
                                }
                            }

                            return Some((
                                event,
                                (
                                    streams,
                                    strength_abort,
                                    connection,
                                    device_path,
                                    device_interface,
                                ),
                            ));
                        }
                    },
                )
                .boxed();

            futures_util::stream::select(state_stream, device_stream).boxed()
        }

        _ => state_stream.boxed(),
    };

    let (abort_handle, abort_registration) = AbortHandle::new_pair();

    let managed_stream = Abortable::new(device_stream, abort_registration).boxed();

    Ok((abort_handle, managed_stream))
}

fn retry_device_stream(
    object_path: String,
    ticket: RetryTicket,
) -> futures_util::stream::BoxStream<'static, MonitorEvent> {
    use futures_util::StreamExt;

    futures_util::stream::once(async move {
        futures_timer::Delay::new(ticket.delay).await;

        MonitorEvent::RetryDevice {
            object_path,
            ticket,
        }
    })
    .boxed()
}

fn track_device_stream(
    object_path: String,
    token: SubscriptionToken,
    stream: futures_util::stream::BoxStream<'static, NetworkEvent>,
) -> futures_util::stream::BoxStream<'static, MonitorEvent> {
    use futures_util::StreamExt;

    stream
        .map(MonitorEvent::Network)
        .chain(futures_util::stream::once(async move {
            MonitorEvent::DeviceStreamEnded { object_path, token }
        }))
        .boxed()
}

async fn start_device_subscription(
    connection: &Connection,
    object_path: String,
    token: SubscriptionToken,
    registry: &mut DeviceRegistry,
    active: &mut std::collections::HashMap<
        String,
        (SubscriptionToken, futures_util::stream::AbortHandle),
    >,
    events: &mut futures_util::stream::SelectAll<
        futures_util::stream::BoxStream<'static, MonitorEvent>,
    >,
    sender: &std::sync::mpsc::Sender<NetworkEvent>,
) {
    match create_device_event_stream(connection, &object_path).await {
        Ok((abort_handle, stream)) => {
            if registry.subscription_succeeded(&object_path, token) {
                active.insert(object_path.clone(), (token, abort_handle));
                events.push(track_device_stream(object_path, token, stream));
            } else {
                abort_handle.abort();
            }
        }

        Err(error) => {
            let _ = sender.send(NetworkEvent::MonitorError { message: error });

            if let Some(ticket) = registry.subscription_failed(&object_path, token) {
                events.push(retry_device_stream(object_path, ticket));
            }
        }
    }
}

async fn run_network_monitor(
    connection: &Connection,
    sender: &std::sync::mpsc::Sender<NetworkEvent>,
) -> Result<(), String> {
    use futures_util::stream::{AbortHandle, BoxStream, SelectAll};
    use futures_util::{stream::select, FutureExt, StreamExt};

    let manager = Proxy::new(connection, NM_BUS, NM_PATH, NM_INTERFACE)
        .await
        .map_err(|error| format!("Network event manager proxy: {error}"))?;

    let state_stream = manager
        .receive_property_changed::<u32>("State")
        .await
        .skip(1)
        .then(|change| async move {
            match change.get().await {
                Ok(state) => NetworkEvent::ManagerStateChanged {
                    state: state.into(),
                },
                Err(error) => NetworkEvent::MonitorError {
                    message: format!("Could not read NetworkManager State: {error}"),
                },
            }
        });

    let connectivity_stream = manager
        .receive_property_changed::<u32>("Connectivity")
        .await
        .skip(1)
        .then(|change| async move {
            match change.get().await {
                Ok(connectivity) => NetworkEvent::ConnectivityChanged {
                    connectivity: connectivity.into(),
                },
                Err(error) => NetworkEvent::MonitorError {
                    message: format!("Could not read NetworkManager Connectivity: {error}"),
                },
            }
        });

    let added_stream = manager
        .receive_signal("DeviceAdded")
        .await
        .map_err(|error| format!("Subscribe to DeviceAdded: {error}"))?
        .map(
            |message| match message.body().deserialize::<OwnedObjectPath>() {
                Ok(path) => NetworkEvent::DeviceAdded {
                    object_path: path.to_string(),
                },
                Err(error) => NetworkEvent::MonitorError {
                    message: format!("Invalid DeviceAdded signal: {error}"),
                },
            },
        );

    let removed_stream = manager
        .receive_signal("DeviceRemoved")
        .await
        .map_err(|error| format!("Subscribe to DeviceRemoved: {error}"))?
        .map(
            |message| match message.body().deserialize::<OwnedObjectPath>() {
                Ok(path) => NetworkEvent::DeviceRemoved {
                    object_path: path.to_string(),
                },
                Err(error) => NetworkEvent::MonitorError {
                    message: format!("Invalid DeviceRemoved signal: {error}"),
                },
            },
        );

    let mut manager_events = select(
        select(state_stream, connectivity_stream),
        select(added_stream, removed_stream),
    )
    .map(MonitorEvent::Network)
    .boxed();

    // Device subscriptions and retry timers share this collection.
    // No blocking sleeps or additional async runtime are needed.
    let mut events: SelectAll<BoxStream<'static, MonitorEvent>> = SelectAll::new();

    let mut registry = DeviceRegistry::new();

    let mut active: std::collections::HashMap<String, (SubscriptionToken, AbortHandle)> =
        std::collections::HashMap::new();

    let existing_devices: Vec<OwnedObjectPath> = manager
        .get_property("Devices")
        .await
        .map_err(|error| format!("Read initial NetworkManager devices: {error}"))?;

    for path in existing_devices {
        let object_path = path.to_string();

        if let Some(token) = registry.add_device(&object_path) {
            start_device_subscription(
                connection,
                object_path,
                token,
                &mut registry,
                &mut active,
                &mut events,
                sender,
            )
            .await;
        }
    }

    loop {
        let (from_manager, event) = if events.is_empty() {
            (true, manager_events.next().await)
        } else {
            futures_util::select! {
                event = manager_events.next().fuse() => (true, event),
                event = events.next().fuse() => (false, event),
            }
        };

        let Some(event) = event else {
            if from_manager {
                return Ok(());
            }
            continue;
        };

        match event {
            MonitorEvent::Network(network_event) => {
                match &network_event {
                    NetworkEvent::DeviceAdded { object_path } => {
                        if let Some(token) = registry.add_device(object_path) {
                            start_device_subscription(
                                connection,
                                object_path.clone(),
                                token,
                                &mut registry,
                                &mut active,
                                &mut events,
                                sender,
                            )
                            .await;
                        }
                    }

                    NetworkEvent::DeviceRemoved { object_path } => {
                        registry.remove_device(object_path);

                        if let Some((_, abort_handle)) = active.remove(object_path) {
                            abort_handle.abort();
                        }
                    }

                    _ => {}
                }

                if sender.send(network_event).is_err() {
                    break;
                }
            }

            MonitorEvent::DeviceStreamEnded { object_path, token } => {
                let is_current = active
                    .get(&object_path)
                    .is_some_and(|(active_token, _)| *active_token == token);

                if !is_current {
                    continue;
                }

                active.remove(&object_path);

                if let Some(ticket) = registry.subscription_failed(&object_path, token) {
                    events.push(retry_device_stream(object_path, ticket));
                }
            }

            MonitorEvent::RetryDevice {
                object_path,
                ticket,
            } => {
                if let Some(token) = registry.retry_due(&object_path, ticket) {
                    start_device_subscription(
                        connection,
                        object_path,
                        token,
                        &mut registry,
                        &mut active,
                        &mut events,
                        sender,
                    )
                    .await;
                }
            }
        }
    }

    Ok(())
}
#[cfg(test)]
mod device_lifecycle_tests {
    use super::*;
    use futures_util::StreamExt;

    fn token() -> SubscriptionToken {
        SubscriptionToken {
            device_generation: 1,
            subscription_generation: 1,
        }
    }

    #[test]
    fn tracked_stream_emits_end_marker_after_events() {
        let stream = futures_util::stream::iter(vec![NetworkEvent::MonitorError {
            message: "test event".to_string(),
        }])
        .boxed();

        let events = zbus::block_on(
            track_device_stream("/test/device".to_string(), token(), stream).collect::<Vec<_>>(),
        );

        assert_eq!(events.len(), 2);

        assert!(matches!(
            &events[0],
            MonitorEvent::Network(NetworkEvent::MonitorError { message })
                if message == "test event"
        ));

        assert!(matches!(
            &events[1],
            MonitorEvent::DeviceStreamEnded { object_path, token: event_token }
                if object_path == "/test/device" && *event_token == token()
        ));
    }

    #[test]
    fn tracked_stream_emits_end_marker_after_abort() {
        use futures_util::stream::{AbortHandle, Abortable};

        let (abort_handle, registration) = AbortHandle::new_pair();

        let stream = Abortable::new(
            futures_util::stream::pending::<NetworkEvent>(),
            registration,
        )
        .boxed();

        abort_handle.abort();

        let events = zbus::block_on(
            track_device_stream("/test/aborted-device".to_string(), token(), stream)
                .collect::<Vec<_>>(),
        );

        assert_eq!(events.len(), 1);

        assert!(matches!(
            &events[0],
            MonitorEvent::DeviceStreamEnded { object_path, token: event_token }
                if object_path == "/test/aborted-device" && *event_token == token()
        ));
    }

    #[test]
    fn duplicate_add_does_not_create_another_subscription() {
        let mut registry = DeviceRegistry::new();

        let first = registry.add_device("/device/1").unwrap();

        assert!(registry.add_device("/device/1").is_none());
        assert_eq!(first.device_generation, 1);
    }

    #[test]
    fn retry_backoff_is_bounded() {
        assert_eq!(DeviceRegistry::retry_delay(0).as_secs(), 1);
        assert_eq!(DeviceRegistry::retry_delay(1).as_secs(), 2);
        assert_eq!(DeviceRegistry::retry_delay(2).as_secs(), 4);
        assert_eq!(DeviceRegistry::retry_delay(3).as_secs(), 8);
        assert_eq!(DeviceRegistry::retry_delay(4).as_secs(), 30);
        assert_eq!(DeviceRegistry::retry_delay(100).as_secs(), 30);
    }

    #[test]
    fn failed_subscription_schedules_retry() {
        let mut registry = DeviceRegistry::new();
        let token = registry.add_device("/device/1").unwrap();

        let ticket = registry.subscription_failed("/device/1", token).unwrap();

        assert_eq!(ticket.delay.as_secs(), 1);
        assert!(registry.begin_subscription("/device/1").is_none());
    }

    #[test]
    fn stale_subscription_failure_is_ignored() {
        let mut registry = DeviceRegistry::new();
        let first = registry.add_device("/device/1").unwrap();

        let ticket = registry.subscription_failed("/device/1", first).unwrap();

        let second = registry.retry_due("/device/1", ticket).unwrap();

        assert_ne!(
            first.subscription_generation,
            second.subscription_generation
        );

        assert!(registry.subscription_failed("/device/1", first).is_none());
    }

    #[test]
    fn removal_invalidates_pending_retry() {
        let mut registry = DeviceRegistry::new();
        let token = registry.add_device("/device/1").unwrap();

        let ticket = registry.subscription_failed("/device/1", token).unwrap();

        registry.remove_device("/device/1");

        assert!(registry.retry_due("/device/1", ticket).is_none());
    }

    #[test]
    fn readded_device_gets_new_generation() {
        let mut registry = DeviceRegistry::new();

        let first = registry.add_device("/device/1").unwrap();
        registry.remove_device("/device/1");

        let second = registry.add_device("/device/1").unwrap();

        assert_ne!(first.device_generation, second.device_generation);
    }

    #[test]
    fn successful_subscription_resets_backoff() {
        let mut registry = DeviceRegistry::new();
        let first = registry.add_device("/device/1").unwrap();

        let first_ticket = registry.subscription_failed("/device/1", first).unwrap();

        let second = registry.retry_due("/device/1", first_ticket).unwrap();

        let second_ticket = registry.subscription_failed("/device/1", second).unwrap();

        assert_eq!(second_ticket.delay.as_secs(), 2);

        let third = registry.retry_due("/device/1", second_ticket).unwrap();

        assert!(registry.subscription_succeeded("/device/1", third));

        let third_ticket = registry.subscription_failed("/device/1", third).unwrap();

        assert_eq!(third_ticket.delay.as_secs(), 1);
    }

    #[test]
    fn stale_retry_cannot_start_another_subscription() {
        let mut registry = DeviceRegistry::new();
        let token = registry.add_device("/device/1").unwrap();

        let ticket = registry.subscription_failed("/device/1", token).unwrap();

        let _new_token = registry.retry_due("/device/1", ticket).unwrap();

        assert!(registry.retry_due("/device/1", ticket).is_none());
    }
}
