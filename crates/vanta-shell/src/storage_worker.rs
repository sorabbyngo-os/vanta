use std::{io, thread, time::Duration};

use smithay_client_toolkit::reexports::calloop::channel;

use vanta_system::{
    storage_state::{StorageStateManager, StorageStateView},
    SystemBus,
};

pub enum StorageEvent {
    State(StorageStateView),
    Unavailable(String),
}

pub struct StorageWorker {
    _worker: thread::JoinHandle<()>,
}

impl StorageWorker {
    pub fn new(event_sender: channel::Sender<StorageEvent>) -> io::Result<Self> {
        let worker = thread::Builder::new()
            .name("vanta-shell-storage".into())
            .spawn(move || loop {
                let bus = match SystemBus::connect() {
                    Ok(bus) => bus,
                    Err(error) => {
                        let _ = event_sender.send(StorageEvent::Unavailable(error));
                        thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                };

                let manager = match StorageStateManager::start(&bus) {
                    Ok(manager) => manager,
                    Err(error) => {
                        let _ = event_sender.send(StorageEvent::Unavailable(error));
                        thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                };

                let updates = manager.subscribe_updates();

                if event_sender
                    .send(StorageEvent::State(manager.view()))
                    .is_err()
                {
                    return;
                }

                loop {
                    match updates.recv() {
                        Ok(_revision) => {
                            if event_sender
                                .send(StorageEvent::State(manager.view()))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Err(error) => {
                            let _ = event_sender.send(StorageEvent::Unavailable(format!(
                                "Storage update monitor disconnected: {error}"
                            )));
                            break;
                        }
                    }
                }

                thread::sleep(Duration::from_secs(3));
            })?;

        Ok(Self { _worker: worker })
    }
}
