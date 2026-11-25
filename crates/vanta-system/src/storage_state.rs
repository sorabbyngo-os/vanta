//! Snapshot-backed, event-driven storage state synchronization.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc, Mutex, RwLock,
};
use std::thread;
use std::time::Duration;

use crate::{storage::StorageSnapshot, storage_events::StorageEvent, SystemBus};

#[derive(Debug, Clone)]
pub struct StorageStateView {
    pub snapshot: StorageSnapshot,
    pub revision: u64,
    pub last_error: Option<String>,
}

struct SharedState {
    snapshot: RwLock<StorageSnapshot>,
    revision: AtomicU64,
    last_error: RwLock<Option<String>>,
    subscribers: Mutex<Vec<mpsc::Sender<u64>>>,
}

#[derive(Clone)]
pub struct StorageStateManager {
    shared: Arc<SharedState>,
}

impl StorageStateManager {
    /// Initialize from a live snapshot and start background synchronization.
    pub fn start(bus: &SystemBus) -> Result<Self, String> {
        let initial_snapshot = bus.storage_snapshot()?;
        let events = bus.subscribe_storage_events()?;

        let shared = Arc::new(SharedState {
            snapshot: RwLock::new(initial_snapshot),
            revision: AtomicU64::new(1),
            last_error: RwLock::new(None),
            subscribers: Mutex::new(Vec::new()),
        });

        let worker_state = Arc::clone(&shared);

        thread::Builder::new()
            .name("vanta-storage-state".into())
            .spawn(move || storage_state_worker(events, worker_state))
            .map_err(|e| format!("Could not start storage state worker: {e}"))?;

        Ok(Self { shared })
    }

    /// Get the current snapshot, revision and error status.
    pub fn view(&self) -> StorageStateView {
        let snapshot = self
            .shared
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let last_error = self
            .shared
            .last_error
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        StorageStateView {
            snapshot,
            revision: self.revision(),
            last_error,
        }
    }

    /// Get the latest storage snapshot.
    pub fn snapshot(&self) -> StorageSnapshot {
        self.shared
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Monotonically increasing state revision.
    pub fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }

    /// Latest monitor or refresh error, if present.
    pub fn last_error(&self) -> Option<String> {
        self.shared
            .last_error
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Subscribe to revision updates.
    pub fn subscribe_updates(&self) -> mpsc::Receiver<u64> {
        let (sender, receiver) = mpsc::channel();

        self.shared
            .subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(sender);

        receiver
    }
}

fn publish_revision(shared: &SharedState, revision: u64) {
    let mut subscribers = shared.subscribers.lock().unwrap_or_else(|e| e.into_inner());

    subscribers.retain(|sender| sender.send(revision).is_ok());
}

fn set_error(shared: &SharedState, message: String) {
    *shared.last_error.write().unwrap_or_else(|e| e.into_inner()) = Some(message);

    publish_revision(shared, shared.revision.load(Ordering::Acquire));
}

fn process_event(event: StorageEvent, refresh_needed: &mut bool, monitor_errors: &mut Vec<String>) {
    match event {
        StorageEvent::InterfaceAdded { .. }
        | StorageEvent::InterfaceRemoved { .. }
        | StorageEvent::PropertiesChanged { .. } => {
            *refresh_needed = true;
        }
        StorageEvent::MonitorError { message } => {
            monitor_errors.push(message);
        }
    }
}

fn storage_state_worker(events: mpsc::Receiver<StorageEvent>, shared: Arc<SharedState>) {
    loop {
        let first = match events.recv() {
            Ok(event) => event,
            Err(error) => {
                set_error(&shared, format!("Storage event channel closed: {error}"));
                return;
            }
        };

        let mut refresh_needed = false;
        let mut monitor_errors = Vec::new();

        process_event(first, &mut refresh_needed, &mut monitor_errors);

        // Coalesce signal bursts into one snapshot refresh.
        loop {
            match events.recv_timeout(Duration::from_millis(120)) {
                Ok(event) => {
                    process_event(event, &mut refresh_needed, &mut monitor_errors);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    set_error(&shared, "Storage event channel disconnected".into());
                    return;
                }
            }
        }

        if refresh_needed {
            match SystemBus::connect().and_then(|bus| bus.storage_snapshot()) {
                Ok(snapshot) => {
                    *shared.snapshot.write().unwrap_or_else(|e| e.into_inner()) = snapshot;

                    *shared.last_error.write().unwrap_or_else(|e| e.into_inner()) = None;

                    let revision = shared.revision.fetch_add(1, Ordering::AcqRel) + 1;

                    publish_revision(&shared, revision);
                }
                Err(error) => {
                    set_error(&shared, format!("Storage snapshot refresh failed: {error}"));
                }
            }
        }

        if !monitor_errors.is_empty() {
            set_error(&shared, monitor_errors.join("; "));
        }
    }
}
