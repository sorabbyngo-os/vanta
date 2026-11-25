//! Read-only BlueZ event monitoring.
//!
//! This module observes ObjectManager and PropertiesChanged signals.
//! It does not initiate discovery, pairing, connection, or adapter changes.

use std::collections::HashMap;
use std::time::Duration;

use zbus::{Connection, OwnedObjectPath, Proxy};

const BLUEZ_BUS: &str = "org.bluez";
const OBJECT_MANAGER_PATH: &str = "/";
const OBJECT_MANAGER_INTERFACE: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const ADAPTER_INTERFACE: &str = "org.bluez.Adapter1";
const DEVICE_INTERFACE: &str = "org.bluez.Device1";

#[derive(Debug, Clone)]
pub enum BluetoothEvent {
    AdapterAdded {
        object_path: String,
    },
    AdapterRemoved {
        object_path: String,
    },
    DeviceAdded {
        object_path: String,
    },
    DeviceRemoved {
        object_path: String,
    },
    PropertiesChanged {
        object_path: String,
        interface: String,
        changed_properties: Vec<String>,
        invalidated_properties: Vec<String>,
    },
    MonitorError {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ObjectKey {
    path: String,
    interface: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubscriptionToken {
    object_generation: u64,
    subscription_generation: u64,
}

#[derive(Debug, Clone, Copy)]
struct RetryTicket {
    object_generation: u64,
    retry_generation: u64,
    delay: Duration,
}

struct ObjectLifecycle {
    generation: u64,
    active_subscription: Option<u64>,
    pending_retry: Option<u64>,
    retry_attempt: u32,
}

struct ObjectRegistry {
    next_object_generation: u64,
    next_subscription_generation: u64,
    next_retry_generation: u64,
    objects: HashMap<ObjectKey, ObjectLifecycle>,
}

impl ObjectRegistry {
    fn new() -> Self {
        Self {
            next_object_generation: 1,
            next_subscription_generation: 1,
            next_retry_generation: 1,
            objects: HashMap::new(),
        }
    }

    fn allocate(counter: &mut u64) -> u64 {
        let value = *counter;
        *counter = counter.wrapping_add(1).max(1);
        value
    }

    fn retry_delay(attempt: u32) -> Duration {
        Duration::from_secs(match attempt {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 8,
            _ => 30,
        })
    }

    fn add(&mut self, key: ObjectKey) -> Option<SubscriptionToken> {
        if self.objects.contains_key(&key) {
            return None;
        }

        let generation = Self::allocate(&mut self.next_object_generation);
        let subscription = Self::allocate(&mut self.next_subscription_generation);

        self.objects.insert(
            key,
            ObjectLifecycle {
                generation,
                active_subscription: Some(subscription),
                pending_retry: None,
                retry_attempt: 0,
            },
        );

        Some(SubscriptionToken {
            object_generation: generation,
            subscription_generation: subscription,
        })
    }

    fn subscription_succeeded(&mut self, key: &ObjectKey, token: SubscriptionToken) -> bool {
        let Some(object) = self.objects.get_mut(key) else {
            return false;
        };

        if object.generation != token.object_generation
            || object.active_subscription != Some(token.subscription_generation)
        {
            return false;
        }

        object.retry_attempt = 0;
        true
    }

    fn schedule_retry(&mut self, key: &ObjectKey, token: SubscriptionToken) -> Option<RetryTicket> {
        let object = self.objects.get_mut(key)?;

        if object.generation != token.object_generation
            || object.active_subscription != Some(token.subscription_generation)
        {
            return None;
        }

        object.active_subscription = None;

        let delay = Self::retry_delay(object.retry_attempt);
        object.retry_attempt = object.retry_attempt.saturating_add(1);

        let retry_generation = Self::allocate(&mut self.next_retry_generation);
        object.pending_retry = Some(retry_generation);

        Some(RetryTicket {
            object_generation: object.generation,
            retry_generation,
            delay,
        })
    }

    fn retry_due(&mut self, key: &ObjectKey, ticket: RetryTicket) -> Option<SubscriptionToken> {
        let object = self.objects.get_mut(key)?;

        if object.generation != ticket.object_generation
            || object.pending_retry != Some(ticket.retry_generation)
            || object.active_subscription.is_some()
        {
            return None;
        }

        object.pending_retry = None;

        let subscription = Self::allocate(&mut self.next_subscription_generation);
        object.active_subscription = Some(subscription);

        Some(SubscriptionToken {
            object_generation: object.generation,
            subscription_generation: subscription,
        })
    }

    fn remove(&mut self, key: &ObjectKey) {
        self.objects.remove(key);
    }
}

enum ManagerEvent {
    Added {
        path: String,
        interfaces: Vec<String>,
    },
    Removed {
        path: String,
        interfaces: Vec<String>,
    },
    Error(String),
}

enum MonitorEvent {
    Bluetooth(BluetoothEvent),
    ObjectStreamEnded {
        key: ObjectKey,
        token: SubscriptionToken,
    },
    RetryObject {
        key: ObjectKey,
        ticket: RetryTicket,
    },
}

fn retry_stream(
    key: ObjectKey,
    ticket: RetryTicket,
) -> futures_util::stream::BoxStream<'static, MonitorEvent> {
    use futures_util::StreamExt;

    futures_util::stream::once(async move {
        futures_timer::Delay::new(ticket.delay).await;
        MonitorEvent::RetryObject { key, ticket }
    })
    .boxed()
}

fn track_stream(
    key: ObjectKey,
    token: SubscriptionToken,
    stream: futures_util::stream::BoxStream<'static, BluetoothEvent>,
) -> futures_util::stream::BoxStream<'static, MonitorEvent> {
    use futures_util::StreamExt;

    stream
        .map(MonitorEvent::Bluetooth)
        .chain(futures_util::stream::once(async move {
            MonitorEvent::ObjectStreamEnded { key, token }
        }))
        .boxed()
}

async fn create_object_stream(
    connection: &Connection,
    key: &ObjectKey,
) -> Result<
    (
        futures_util::stream::AbortHandle,
        futures_util::stream::BoxStream<'static, BluetoothEvent>,
    ),
    String,
> {
    use futures_util::stream::{AbortHandle, Abortable, StreamExt};

    let proxy = Proxy::new_owned(
        connection.clone(),
        BLUEZ_BUS,
        key.path.clone(),
        PROPERTIES_INTERFACE,
    )
    .await
    .map_err(|error| format!("BlueZ properties proxy {}: {error}", key.path))?;

    let path = key.path.clone();
    let expected_interface = key.interface.clone();

    let stream = proxy
        .receive_signal("PropertiesChanged")
        .await
        .map_err(|error| format!("Subscribe to BlueZ properties {}: {error}", key.path))?
        .filter_map(move |message| {
            let path = path.clone();
            let expected_interface = expected_interface.clone();

            async move {
                let decoded =
                    message
                        .body()
                        .deserialize::<(String, HashMap<String, zbus::OwnedVariant>, Vec<String>)>(
                        );

                match decoded {
                    Ok((interface, changed, invalidated)) if interface == expected_interface => {
                        Some(BluetoothEvent::PropertiesChanged {
                            object_path: path,
                            interface,
                            changed_properties: changed.into_keys().collect(),
                            invalidated_properties: invalidated,
                        })
                    }
                    Ok(_) => None,
                    Err(error) => Some(BluetoothEvent::MonitorError {
                        message: format!("Invalid PropertiesChanged signal for {path}: {error}"),
                    }),
                }
            }
        })
        .boxed();

    let (abort_handle, registration) = AbortHandle::new_pair();
    let managed = Abortable::new(stream, registration).boxed();

    Ok((abort_handle, managed))
}

async fn start_subscription(
    connection: &Connection,
    key: ObjectKey,
    token: SubscriptionToken,
    registry: &mut ObjectRegistry,
    active: &mut HashMap<ObjectKey, (SubscriptionToken, futures_util::stream::AbortHandle)>,
    streams: &mut futures_util::stream::SelectAll<
        futures_util::stream::BoxStream<'static, MonitorEvent>,
    >,
    sender: &std::sync::mpsc::Sender<BluetoothEvent>,
) {
    match create_object_stream(connection, &key).await {
        Ok((abort_handle, stream)) => {
            if registry.subscription_succeeded(&key, token) {
                active.insert(key.clone(), (token, abort_handle));
                streams.push(track_stream(key, token, stream));
            } else {
                abort_handle.abort();
            }
        }
        Err(error) => {
            let _ = sender.send(BluetoothEvent::MonitorError { message: error });

            if let Some(ticket) = registry.schedule_retry(&key, token) {
                streams.push(retry_stream(key, ticket));
            }
        }
    }
}

fn target_interface(interface: &str) -> bool {
    interface == ADAPTER_INTERFACE || interface == DEVICE_INTERFACE
}

fn added_event(key: &ObjectKey) -> BluetoothEvent {
    if key.interface == ADAPTER_INTERFACE {
        BluetoothEvent::AdapterAdded {
            object_path: key.path.clone(),
        }
    } else {
        BluetoothEvent::DeviceAdded {
            object_path: key.path.clone(),
        }
    }
}

fn removed_event(key: &ObjectKey) -> BluetoothEvent {
    if key.interface == ADAPTER_INTERFACE {
        BluetoothEvent::AdapterRemoved {
            object_path: key.path.clone(),
        }
    } else {
        BluetoothEvent::DeviceRemoved {
            object_path: key.path.clone(),
        }
    }
}

pub fn spawn_bluetooth_monitor() -> Result<std::sync::mpsc::Receiver<BluetoothEvent>, String> {
    use std::sync::mpsc;
    use std::thread;

    let (sender, receiver) = mpsc::channel();

    thread::Builder::new()
        .name("vanta-bluetooth-events".into())
        .spawn(move || {
            let result = zbus::block_on(async {
                let connection = Connection::system()
                    .await
                    .map_err(|error| format!("Bluetooth event D-Bus connection: {error}"))?;

                run_bluetooth_monitor(&connection, &sender).await
            });

            if let Err(error) = result {
                let _ = sender.send(BluetoothEvent::MonitorError { message: error });
            }
        })
        .map_err(|error| format!("Could not start Bluetooth event thread: {error}"))?;

    Ok(receiver)
}

async fn run_bluetooth_monitor(
    connection: &Connection,
    sender: &std::sync::mpsc::Sender<BluetoothEvent>,
) -> Result<(), String> {
    use futures_util::stream::{BoxStream, SelectAll};
    use futures_util::{stream::select, StreamExt};

    let manager = Proxy::new(
        connection,
        BLUEZ_BUS,
        OBJECT_MANAGER_PATH,
        OBJECT_MANAGER_INTERFACE,
    )
    .await
    .map_err(|error| format!("BlueZ event ObjectManager: {error}"))?;

    let added = manager
        .receive_signal("InterfacesAdded")
        .await
        .map_err(|error| format!("Subscribe to BlueZ InterfacesAdded: {error}"))?
        .map(|message| {
            match message.body().deserialize::<(
                OwnedObjectPath,
                HashMap<String, HashMap<String, zbus::OwnedVariant>>,
            )>() {
                Ok((path, interfaces)) => ManagerEvent::Added {
                    path: path.to_string(),
                    interfaces: interfaces.into_keys().collect(),
                },
                Err(error) => {
                    ManagerEvent::Error(format!("Invalid BlueZ InterfacesAdded signal: {error}"))
                }
            }
        });

    let removed = manager
        .receive_signal("InterfacesRemoved")
        .await
        .map_err(|error| format!("Subscribe to BlueZ InterfacesRemoved: {error}"))?
        .map(|message| {
            match message
                .body()
                .deserialize::<(OwnedObjectPath, Vec<String>)>()
            {
                Ok((path, interfaces)) => ManagerEvent::Removed {
                    path: path.to_string(),
                    interfaces,
                },
                Err(error) => {
                    ManagerEvent::Error(format!("Invalid BlueZ InterfacesRemoved signal: {error}"))
                }
            }
        });

    let mut manager_stream = select(added, removed).boxed().fuse();

    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, zbus::OwnedVariant>>> =
        manager
            .call("GetManagedObjects", &())
            .await
            .map_err(|error| format!("BlueZ GetManagedObjects: {error}"))?;

    let mut registry = ObjectRegistry::new();
    let mut active: HashMap<ObjectKey, (SubscriptionToken, futures_util::stream::AbortHandle)> =
        HashMap::new();
    let mut streams: SelectAll<BoxStream<'static, MonitorEvent>> = SelectAll::new();

    for (path, interfaces) in objects {
        for interface in interfaces.keys().filter(|name| target_interface(name)) {
            let key = ObjectKey {
                path: path.to_string(),
                interface: interface.clone(),
            };

            if let Some(token) = registry.add(key.clone()) {
                let _ = sender.send(added_event(&key));
                start_subscription(
                    connection,
                    key,
                    token,
                    &mut registry,
                    &mut active,
                    &mut streams,
                    sender,
                )
                .await;
            }
        }
    }

    loop {
        if streams.is_empty() {
            match manager_stream.next().await {
                Some(event) => {
                    handle_manager_event(
                        event,
                        connection,
                        sender,
                        &mut registry,
                        &mut active,
                        &mut streams,
                    )
                    .await;
                }
                None => return Err("BlueZ ObjectManager event stream ended".into()),
            }
        } else {
            futures_util::select! {
                manager_event = manager_stream.next() => {
                    match manager_event {
                        Some(event) => {
                            handle_manager_event(
                                event,
                                connection,
                                sender,
                                &mut registry,
                                &mut active,
                                &mut streams,
                            ).await;
                        }
                        None => return Err("BlueZ ObjectManager event stream ended".into()),
                    }
                },
                object_event = streams.next() => {
                    if let Some(event) = object_event {
                        handle_object_event(
                            event,
                            connection,
                            sender,
                            &mut registry,
                            &mut active,
                            &mut streams,
                        ).await;
                    }
                }
            }
        }
    }
}

async fn handle_manager_event(
    event: ManagerEvent,
    connection: &Connection,
    sender: &std::sync::mpsc::Sender<BluetoothEvent>,
    registry: &mut ObjectRegistry,
    active: &mut HashMap<ObjectKey, (SubscriptionToken, futures_util::stream::AbortHandle)>,
    streams: &mut futures_util::stream::SelectAll<
        futures_util::stream::BoxStream<'static, MonitorEvent>,
    >,
) {
    match event {
        ManagerEvent::Error(message) => {
            let _ = sender.send(BluetoothEvent::MonitorError { message });
        }
        ManagerEvent::Added { path, interfaces } => {
            for interface in interfaces.into_iter().filter(|name| target_interface(name)) {
                let key = ObjectKey {
                    path: path.clone(),
                    interface,
                };

                if let Some(token) = registry.add(key.clone()) {
                    let _ = sender.send(added_event(&key));
                    start_subscription(connection, key, token, registry, active, streams, sender)
                        .await;
                }
            }
        }
        ManagerEvent::Removed { path, interfaces } => {
            for interface in interfaces.into_iter().filter(|name| target_interface(name)) {
                let key = ObjectKey {
                    path: path.clone(),
                    interface,
                };

                registry.remove(&key);

                if let Some((_, abort_handle)) = active.remove(&key) {
                    abort_handle.abort();
                }

                let _ = sender.send(removed_event(&key));
            }
        }
    }
}

async fn handle_object_event(
    event: MonitorEvent,
    connection: &Connection,
    sender: &std::sync::mpsc::Sender<BluetoothEvent>,
    registry: &mut ObjectRegistry,
    active: &mut HashMap<ObjectKey, (SubscriptionToken, futures_util::stream::AbortHandle)>,
    streams: &mut futures_util::stream::SelectAll<
        futures_util::stream::BoxStream<'static, MonitorEvent>,
    >,
) {
    match event {
        MonitorEvent::Bluetooth(event) => {
            let _ = sender.send(event);
        }
        MonitorEvent::ObjectStreamEnded { key, token } => {
            if let Some((active_token, _)) = active.get(&key) {
                if *active_token == token {
                    active.remove(&key);

                    if let Some(ticket) = registry.schedule_retry(&key, token) {
                        streams.push(retry_stream(key, ticket));
                    }
                }
            }
        }
        MonitorEvent::RetryObject { key, ticket } => {
            if let Some(token) = registry.retry_due(&key, ticket) {
                start_subscription(connection, key, token, registry, active, streams, sender).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ObjectKey {
        ObjectKey {
            path: "/org/bluez/hci0".into(),
            interface: ADAPTER_INTERFACE.into(),
        }
    }

    #[test]
    fn retry_backoff_is_bounded() {
        assert_eq!(ObjectRegistry::retry_delay(0).as_secs(), 1);
        assert_eq!(ObjectRegistry::retry_delay(1).as_secs(), 2);
        assert_eq!(ObjectRegistry::retry_delay(2).as_secs(), 4);
        assert_eq!(ObjectRegistry::retry_delay(3).as_secs(), 8);
        assert_eq!(ObjectRegistry::retry_delay(4).as_secs(), 30);
        assert_eq!(ObjectRegistry::retry_delay(100).as_secs(), 30);
    }

    #[test]
    fn duplicate_add_is_ignored() {
        let mut registry = ObjectRegistry::new();
        assert!(registry.add(key()).is_some());
        assert!(registry.add(key()).is_none());
    }

    #[test]
    fn removal_invalidates_retry() {
        let mut registry = ObjectRegistry::new();
        let object_key = key();
        let token = registry.add(object_key.clone()).unwrap();
        let ticket = registry.schedule_retry(&object_key, token).unwrap();

        registry.remove(&object_key);

        assert!(registry.retry_due(&object_key, ticket).is_none());
    }

    #[test]
    fn readded_object_gets_new_generation() {
        let mut registry = ObjectRegistry::new();
        let object_key = key();

        let first = registry.add(object_key.clone()).unwrap();
        registry.remove(&object_key);
        let second = registry.add(object_key).unwrap();

        assert_ne!(first.object_generation, second.object_generation);
    }

    #[test]
    fn stale_retry_is_ignored() {
        let mut registry = ObjectRegistry::new();
        let object_key = key();
        let token = registry.add(object_key.clone()).unwrap();
        let ticket = registry.schedule_retry(&object_key, token).unwrap();

        let _new_token = registry.retry_due(&object_key, ticket).unwrap();

        assert!(registry.retry_due(&object_key, ticket).is_none());
    }

    #[test]
    #[test]
    fn stale_subscription_cannot_schedule_retry() {
        let mut registry = ObjectRegistry::new();
        let object_key = key();

        let old_token = registry.add(object_key.clone()).unwrap();
        let ticket = registry.schedule_retry(&object_key, old_token).unwrap();
        let current_token = registry.retry_due(&object_key, ticket).unwrap();

        assert_ne!(
            old_token.subscription_generation,
            current_token.subscription_generation
        );
        assert!(registry.schedule_retry(&object_key, old_token).is_none());
    }

    #[test]
    fn removed_object_rejects_stale_subscription_completion() {
        let mut registry = ObjectRegistry::new();
        let object_key = key();

        let old_token = registry.add(object_key.clone()).unwrap();
        registry.remove(&object_key);
        let new_token = registry.add(object_key.clone()).unwrap();

        assert_ne!(old_token.object_generation, new_token.object_generation);
        assert!(!registry.subscription_succeeded(&object_key, old_token));
        assert!(registry.subscription_succeeded(&object_key, new_token));
    }

    #[test]
    fn retry_ticket_cannot_be_consumed_twice() {
        let mut registry = ObjectRegistry::new();
        let object_key = key();
        let token = registry.add(object_key.clone()).unwrap();
        let ticket = registry.schedule_retry(&object_key, token).unwrap();

        assert!(registry.retry_due(&object_key, ticket).is_some());
        assert!(registry.retry_due(&object_key, ticket).is_none());
    }

    fn successful_subscription_resets_backoff() {
        let mut registry = ObjectRegistry::new();
        let object_key = key();

        let first = registry.add(object_key.clone()).unwrap();
        let first_ticket = registry.schedule_retry(&object_key, first).unwrap();
        assert_eq!(first_ticket.delay.as_secs(), 1);

        let second = registry.retry_due(&object_key, first_ticket).unwrap();
        let second_ticket = registry.schedule_retry(&object_key, second).unwrap();
        assert_eq!(second_ticket.delay.as_secs(), 2);

        let third = registry.retry_due(&object_key, second_ticket).unwrap();
        assert!(registry.subscription_succeeded(&object_key, third));

        let third_ticket = registry.schedule_retry(&object_key, third).unwrap();
        assert_eq!(third_ticket.delay.as_secs(), 1);
    }
}
