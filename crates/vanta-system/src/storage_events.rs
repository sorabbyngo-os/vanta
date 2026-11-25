//! Read-only UDisks2 storage event monitoring.
//!
//! Interface subscriptions are cancellable and generation-tracked.
//! The monitor reconnects after service/connection failures.
//! No storage mutations are performed.

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use futures_channel::mpsc;
use futures_util::future::{AbortHandle, Abortable, BoxFuture};
use futures_util::stream::{BoxStream, FuturesUnordered};
use futures_util::{FutureExt, StreamExt};
use zbus::{Connection, OwnedObjectPath, Proxy};

const BUS: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

const RELEVANT_INTERFACES: &[&str] = &[
    "org.freedesktop.UDisks2.Drive",
    "org.freedesktop.UDisks2.Block",
    "org.freedesktop.UDisks2.Partition",
    "org.freedesktop.UDisks2.PartitionTable",
    "org.freedesktop.UDisks2.Filesystem",
    "org.freedesktop.UDisks2.Encrypted",
];

type InterfaceKey = (String, String);
type ActiveInterfaces = HashMap<InterfaceKey, u64>;
type MonitorTask = BoxFuture<'static, (InterfaceKey, u64)>;

#[derive(Debug, Clone)]
pub enum StorageEvent {
    InterfaceAdded {
        object_path: String,
        interface: String,
    },
    InterfaceRemoved {
        object_path: String,
        interface: String,
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

struct PropertyEvent {
    key: InterfaceKey,
    generation: u64,
    event: StorageEvent,
}

fn relevant(interface: &str) -> bool {
    RELEVANT_INTERFACES.contains(&interface)
}

async fn property_stream(
    connection: &Connection,
    path: String,
    interface: String,
) -> Result<BoxStream<'static, StorageEvent>, String> {
    let proxy = Proxy::new_owned(connection.clone(), BUS, path.clone(), PROPERTIES)
        .await
        .map_err(|e| format!("Storage properties proxy {path}: {e}"))?;

    let stream = proxy
        .receive_signal("PropertiesChanged")
        .await
        .map_err(|e| format!("Storage PropertiesChanged subscription {path}: {e}"))?
        .filter_map(move |message| {
            let path = path.clone();
            let expected = interface.clone();

            async move {
                match message
                    .body()
                    .deserialize::<(String, HashMap<String, zbus::OwnedVariant>, Vec<String>)>()
                {
                    Ok((changed_interface, changed, invalidated))
                        if changed_interface == expected =>
                    {
                        Some(StorageEvent::PropertiesChanged {
                            object_path: path,
                            interface: changed_interface,
                            changed_properties: changed.into_keys().collect(),
                            invalidated_properties: invalidated,
                        })
                    }
                    Ok(_) => None,
                    Err(error) => Some(StorageEvent::MonitorError {
                        message: format!("Invalid storage PropertiesChanged: {error}"),
                    }),
                }
            }
        })
        .boxed();

    Ok(stream)
}

async fn register_interface(
    connection: &Connection,
    path: &str,
    interface: &str,
    generation: &mut u64,
    active: &mut ActiveInterfaces,
    aborts: &mut HashMap<InterfaceKey, AbortHandle>,
    tasks: &mut FuturesUnordered<MonitorTask>,
    event_sender: &mpsc::UnboundedSender<PropertyEvent>,
    output: &std::sync::mpsc::Sender<StorageEvent>,
    emit_added: bool,
) {
    if !relevant(interface) {
        return;
    }

    let key = (path.to_owned(), interface.to_owned());

    if active.contains_key(&key) {
        return;
    }

    match property_stream(connection, path.to_owned(), interface.to_owned()).await {
        Ok(mut stream) => {
            *generation = generation.wrapping_add(1);
            let current_generation = *generation;

            let (abort_handle, abort_registration) = AbortHandle::new_pair();

            active.insert(key.clone(), current_generation);
            aborts.insert(key.clone(), abort_handle);

            let sender = event_sender.clone();
            let task_key = key.clone();

            tasks.push(
                async move {
                    let monitor = async move {
                        while let Some(event) = stream.next().await {
                            if sender
                                .unbounded_send(PropertyEvent {
                                    key: task_key.clone(),
                                    generation: current_generation,
                                    event,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    };

                    let _ = Abortable::new(monitor, abort_registration).await;
                    (key, current_generation)
                }
                .boxed(),
            );

            if emit_added {
                let _ = output.send(StorageEvent::InterfaceAdded {
                    object_path: path.to_owned(),
                    interface: interface.to_owned(),
                });
            }
        }
        Err(error) => {
            let _ = output.send(StorageEvent::MonitorError { message: error });
        }
    }
}

fn remove_interface(
    path: &str,
    interface: &str,
    active: &mut ActiveInterfaces,
    aborts: &mut HashMap<InterfaceKey, AbortHandle>,
    output: &std::sync::mpsc::Sender<StorageEvent>,
) {
    let key = (path.to_owned(), interface.to_owned());

    if active.remove(&key).is_some() {
        if let Some(abort) = aborts.remove(&key) {
            abort.abort();
        }

        let _ = output.send(StorageEvent::InterfaceRemoved {
            object_path: path.to_owned(),
            interface: interface.to_owned(),
        });
    }
}

pub fn spawn_storage_monitor() -> Result<std::sync::mpsc::Receiver<StorageEvent>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();

    thread::Builder::new()
        .name("vanta-storage-events".into())
        .spawn(move || {
            let mut retry_delay = Duration::from_secs(1);

            loop {
                let result = zbus::block_on(async {
                    let connection = Connection::system()
                        .await
                        .map_err(|e| format!("Storage monitor D-Bus connection: {e}"))?;

                    run_monitor(&connection, &sender).await
                });

                match result {
                    Ok(()) => {
                        let _ = sender.send(StorageEvent::MonitorError {
                            message: "Storage monitor stopped unexpectedly".into(),
                        });
                    }
                    Err(error) => {
                        let _ = sender.send(StorageEvent::MonitorError {
                            message: format!("Storage monitor disconnected: {error}"),
                        });
                    }
                }

                thread::sleep(retry_delay);
                retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
            }
        })
        .map_err(|e| format!("Could not start storage monitor thread: {e}"))?;

    Ok(receiver)
}

async fn run_monitor(
    connection: &Connection,
    output: &std::sync::mpsc::Sender<StorageEvent>,
) -> Result<(), String> {
    use futures_util::stream::select;

    let manager = Proxy::new(connection, BUS, ROOT, OBJECT_MANAGER)
        .await
        .map_err(|e| format!("UDisks2 ObjectManager proxy: {e}"))?;

    // Subscribe before snapshotting to minimize the race window.
    let added = manager
        .receive_signal("InterfacesAdded")
        .await
        .map_err(|e| format!("InterfacesAdded subscription: {e}"))?
        .map(|message| {
            match message.body().deserialize::<(
                OwnedObjectPath,
                HashMap<String, HashMap<String, zbus::OwnedVariant>>,
            )>() {
                Ok((path, interfaces)) => ManagerEvent::Added {
                    path: path.to_string(),
                    interfaces: interfaces.into_keys().collect(),
                },
                Err(e) => ManagerEvent::Error(format!("Invalid InterfacesAdded: {e}")),
            }
        });

    let removed = manager
        .receive_signal("InterfacesRemoved")
        .await
        .map_err(|e| format!("InterfacesRemoved subscription: {e}"))?
        .map(|message| {
            match message
                .body()
                .deserialize::<(OwnedObjectPath, Vec<String>)>()
            {
                Ok((path, interfaces)) => ManagerEvent::Removed {
                    path: path.to_string(),
                    interfaces,
                },
                Err(e) => ManagerEvent::Error(format!("Invalid InterfacesRemoved: {e}")),
            }
        });

    let mut manager_stream = select(added, removed).boxed().fuse();

    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, zbus::OwnedVariant>>> =
        manager
            .call("GetManagedObjects", &())
            .await
            .map_err(|e| format!("GetManagedObjects: {e}"))?;

    let (property_sender, mut property_receiver) = mpsc::unbounded::<PropertyEvent>();

    let mut tasks: FuturesUnordered<MonitorTask> = FuturesUnordered::new();
    let mut active: ActiveInterfaces = HashMap::new();
    let mut aborts: HashMap<InterfaceKey, AbortHandle> = HashMap::new();
    let mut generation = 0u64;

    // Subscribe to currently existing interfaces.
    for (path, interfaces) in &objects {
        for interface in interfaces.keys().filter(|name| relevant(name)) {
            register_interface(
                connection,
                path.as_str(),
                interface,
                &mut generation,
                &mut active,
                &mut aborts,
                &mut tasks,
                &property_sender,
                output,
                false,
            )
            .await;
        }
    }

    loop {
        if tasks.is_empty() {
            futures_util::select! {
                manager_event = manager_stream.next() => {
                    match manager_event {
                        Some(event) => {
                            handle_manager_event(
                                event,
                                connection,
                                output,
                                &property_sender,
                                &mut property_receiver,
                                &mut tasks,
                                &mut active,
                                &mut aborts,
                                &mut generation,
                            ).await;
                        }
                        None => return Err("UDisks2 manager stream ended".into()),
                    }
                },
                property_event = property_receiver.next() => {
                    if let Some(event) = property_event {
                        if active.get(&event.key) == Some(&event.generation) {
                            let _ = output.send(event.event);
                        }
                    }
                }
            }
        } else {
            futures_util::select! {
                manager_event = manager_stream.next() => {
                    match manager_event {
                        Some(event) => {
                            handle_manager_event(
                                event,
                                connection,
                                output,
                                &property_sender,
                                &mut property_receiver,
                                &mut tasks,
                                &mut active,
                                &mut aborts,
                                &mut generation,
                            ).await;
                        }
                        None => return Err("UDisks2 manager stream ended".into()),
                    }
                },
                property_event = property_receiver.next() => {
                    if let Some(event) = property_event {
                        if active.get(&event.key) == Some(&event.generation) {
                            let _ = output.send(event.event);
                        }
                    }
                },
                completed = tasks.next() => {
                    if let Some((key, completed_generation)) = completed {
                        if active.get(&key) == Some(&completed_generation) {
                            active.remove(&key);
                            aborts.remove(&key);

                            let _ = output.send(StorageEvent::MonitorError {
                                message: format!(
                                    "Storage property stream ended: {} {}",
                                    key.0, key.1
                                ),
                            });
                        }
                    }
                }
            }
        }
    }
}

async fn handle_manager_event(
    event: ManagerEvent,
    connection: &Connection,
    output: &std::sync::mpsc::Sender<StorageEvent>,
    property_sender: &mpsc::UnboundedSender<PropertyEvent>,
    _property_receiver: &mut mpsc::UnboundedReceiver<PropertyEvent>,
    tasks: &mut FuturesUnordered<MonitorTask>,
    active: &mut ActiveInterfaces,
    aborts: &mut HashMap<InterfaceKey, AbortHandle>,
    generation: &mut u64,
) {
    match event {
        ManagerEvent::Error(message) => {
            let _ = output.send(StorageEvent::MonitorError { message });
        }
        ManagerEvent::Added { path, interfaces } => {
            for interface in interfaces.iter().filter(|name| relevant(name)) {
                register_interface(
                    connection,
                    &path,
                    interface,
                    generation,
                    active,
                    aborts,
                    tasks,
                    property_sender,
                    output,
                    true,
                )
                .await;
            }
        }
        ManagerEvent::Removed { path, interfaces } => {
            for interface in interfaces.iter().filter(|name| relevant(name)) {
                remove_interface(&path, interface, active, aborts, output);
            }
        }
    }
}
