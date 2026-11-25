use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    process::Child,
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use crate::{
    launch::LaunchPlan,
    process::{spawn_process, ProcessLaunchError},
};

const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningProcess {
    pub launch_id: u64,
    pub pid: u32,
    pub executable: PathBuf,
    pub startup_wm_class: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessEvent {
    Started(RunningProcess),
    Exited {
        process: RunningProcess,
        success: bool,
        exit_code: Option<i32>,
    },
}

#[derive(Debug)]
pub enum ProcessManagerError {
    Launch(ProcessLaunchError),
    WorkerUnavailable,
    WorkerStart(io::Error),
}

impl std::fmt::Display for ProcessManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Launch(error) => write!(f, "{error}"),
            Self::WorkerUnavailable => write!(f, "Process manager worker is unavailable"),
            Self::WorkerStart(error) => write!(f, "Could not start process manager: {error}"),
        }
    }
}

impl std::error::Error for ProcessManagerError {}

impl From<ProcessLaunchError> for ProcessManagerError {
    fn from(error: ProcessLaunchError) -> Self {
        Self::Launch(error)
    }
}

struct SharedState {
    running: Mutex<Vec<RunningProcess>>,
    subscribers: Mutex<Vec<Sender<ProcessEvent>>>,
}

enum WorkerCommand {
    Launch {
        plan: LaunchPlan,
        reply: Sender<Result<RunningProcess, ProcessManagerError>>,
    },
}

#[derive(Clone)]
pub struct ProcessManager {
    commands: Sender<WorkerCommand>,
    shared: Arc<SharedState>,
}

impl ProcessManager {
    pub fn new() -> Result<Self, ProcessManagerError> {
        let (commands, receiver) = mpsc::channel();

        let shared = Arc::new(SharedState {
            running: Mutex::new(Vec::new()),
            subscribers: Mutex::new(Vec::new()),
        });

        let worker_state = Arc::clone(&shared);

        thread::Builder::new()
            .name("vanta-process-manager".into())
            .spawn(move || worker_loop(receiver, worker_state))
            .map_err(ProcessManagerError::WorkerStart)?;

        Ok(Self { commands, shared })
    }

    /// Request an application launch and receive its process identity.
    pub fn launch(&self, plan: LaunchPlan) -> Result<RunningProcess, ProcessManagerError> {
        let (reply, response) = mpsc::channel();

        self.commands
            .send(WorkerCommand::Launch { plan, reply })
            .map_err(|_| ProcessManagerError::WorkerUnavailable)?;

        response
            .recv()
            .map_err(|_| ProcessManagerError::WorkerUnavailable)?
    }

    /// Return a consistent snapshot of currently tracked processes.
    pub fn snapshot(&self) -> Vec<RunningProcess> {
        let running = self
            .shared
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        running.clone()
    }

    /// Subscribe to process lifecycle events.
    pub fn subscribe(&self) -> Receiver<ProcessEvent> {
        let (sender, receiver) = mpsc::channel();

        self.shared
            .subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(sender);

        receiver
    }
}

fn worker_loop(receiver: Receiver<WorkerCommand>, shared: Arc<SharedState>) {
    let mut children: HashMap<u64, (RunningProcess, Child)> = HashMap::new();
    let mut next_id = 1u64;

    loop {
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(WorkerCommand::Launch { plan, reply }) => {
                let metadata = match &plan {
                    LaunchPlan::Process {
                        executable,
                        startup_wm_class,
                        ..
                    } => Some((executable.clone(), startup_wm_class.clone())),
                    _ => None,
                };

                let result = match (metadata, spawn_process(&plan)) {
                    (Some((executable, startup_wm_class)), Ok(child)) => {
                        let process = RunningProcess {
                            launch_id: next_id,
                            pid: child.id(),
                            executable,
                            startup_wm_class,
                        };

                        next_id = next_id.wrapping_add(1).max(1);

                        children.insert(process.launch_id, (process.clone(), child));

                        shared
                            .running
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(process.clone());

                        publish(&shared, ProcessEvent::Started(process.clone()));
                        Ok(process)
                    }
                    (_, Err(error)) => Err(ProcessManagerError::Launch(error)),
                    (None, Ok(_)) => unreachable!("spawn_process only accepts process plans"),
                };

                let _ = reply.send(result);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        reap_children(&mut children, &shared);
    }
}

fn reap_children(children: &mut HashMap<u64, (RunningProcess, Child)>, shared: &SharedState) {
    let mut completed = Vec::new();

    for (launch_id, (process, child)) in children.iter_mut() {
        match child.try_wait() {
            Ok(Some(status)) => {
                completed.push((*launch_id, process.clone(), status));
            }
            Ok(None) | Err(_) => {}
        }
    }

    for (launch_id, process, status) in completed {
        children.remove(&launch_id);

        shared
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|item| item.launch_id != launch_id);

        publish(
            shared,
            ProcessEvent::Exited {
                process,
                success: status.success(),
                exit_code: status.code(),
            },
        );
    }
}

fn publish(shared: &SharedState, event: ProcessEvent) {
    let mut subscribers = shared.subscribers.lock().unwrap_or_else(|e| e.into_inner());

    subscribers.retain(|subscriber| subscriber.send(event.clone()).is_ok());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[cfg(unix)]
    fn short_process_plan() -> LaunchPlan {
        LaunchPlan::Process {
            executable: PathBuf::from("/usr/bin/sleep"),
            arguments: vec!["/usr/bin/sleep".into(), "0.2".into()],
            working_directory: None,
            startup_wm_class: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn tracks_process_start_and_exit() {
        let manager = ProcessManager::new().unwrap();
        let events = manager.subscribe();

        let process = manager.launch(short_process_plan()).unwrap();

        assert!(process.pid > 0);
        assert_eq!(manager.snapshot().len(), 1);

        assert_eq!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            ProcessEvent::Started(process.clone())
        );

        let event = events.recv_timeout(Duration::from_secs(3)).unwrap();

        assert!(matches!(
            event,
            ProcessEvent::Exited {
                process: exited,
                success: true,
                ..
            } if exited.launch_id == process.launch_id
        ));

        assert!(manager.snapshot().is_empty());
    }

    #[test]
    fn rejects_terminal_launch_plan() {
        let manager = ProcessManager::new().unwrap();

        let plan = LaunchPlan::Terminal {
            executable: PathBuf::from("/usr/bin/htop"),
            arguments: vec!["htop".into()],
            working_directory: None,
            startup_wm_class: None,
        };

        assert!(matches!(
            manager.launch(plan),
            Err(ProcessManagerError::Launch(_))
        ));
    }

    #[test]
    fn supports_multiple_event_subscribers() {
        let manager = ProcessManager::new().unwrap();

        let first = manager.subscribe();
        let second = manager.subscribe();

        let _process = manager.launch(short_process_plan()).unwrap();

        assert!(matches!(
            first.recv_timeout(Duration::from_secs(2)).unwrap(),
            ProcessEvent::Started(_)
        ));

        assert!(matches!(
            second.recv_timeout(Duration::from_secs(2)).unwrap(),
            ProcessEvent::Started(_)
        ));
    }
}
