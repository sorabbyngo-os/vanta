use std::{
    io,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use smithay_client_toolkit::reexports::calloop::channel;

use vanta_apps::{
    launch::{activate_dbus_blocking, prepare_launch, LaunchInput, LaunchPlan},
    process_manager::{ProcessEvent, ProcessManager, RunningProcess},
    Application, ApplicationCatalog,
};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(30);

pub enum ApplicationEvent {
    CatalogReady(Vec<Application>),
    Process(ProcessEvent),
    DbusActivated {
        application_id: String,
    },
    TerminalRequested {
        application_id: String,
        executable: std::path::PathBuf,
        arguments: Vec<String>,
        working_directory: Option<std::path::PathBuf>,
        startup_wm_class: Option<String>,
    },
    LaunchFailed {
        application_id: String,
        error: String,
    },
    Unavailable(String),
}

enum WorkerCommand {
    Launch(String),
}

pub struct ApplicationWorker {
    sender: Sender<WorkerCommand>,
    _worker: JoinHandle<()>,
}

impl ApplicationWorker {
    pub fn new(event_sender: channel::Sender<ApplicationEvent>) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<WorkerCommand>();

        let worker = thread::Builder::new()
            .name("vanta-shell-applications".into())
            .spawn(move || {
                application_worker_loop(receiver, event_sender);
            })?;

        Ok(Self {
            sender,
            _worker: worker,
        })
    }

    pub fn launch(&self, application_id: impl Into<String>) -> Result<(), String> {
        self.sender
            .send(WorkerCommand::Launch(application_id.into()))
            .map_err(|_| "Application worker has stopped".to_owned())
    }
}

fn application_worker_loop(
    receiver: Receiver<WorkerCommand>,
    event_sender: channel::Sender<ApplicationEvent>,
) {
    eprintln!("vanta-shell: application worker thread entered");
    let catalog = match ApplicationCatalog::discover() {
        Ok(catalog) => catalog,
        Err(error) => {
            let _ = event_sender.send(ApplicationEvent::Unavailable(format!(
                "Application discovery failed: {error}"
            )));
            return;
        }
    };

    eprintln!(
        "vanta-shell: application catalog discovered {} entries",
        catalog.applications.len()
    );
    let process_manager = match ProcessManager::new() {
        Ok(manager) => manager,
        Err(error) => {
            let _ = event_sender.send(ApplicationEvent::Unavailable(format!(
                "Process manager initialization failed: {error}"
            )));
            return;
        }
    };

    let process_events = process_manager.subscribe();
    eprintln!("vanta-shell: process manager initialized and subscribed");

    eprintln!("vanta-shell: sending application catalog to event loop");
    if event_sender
        .send(ApplicationEvent::CatalogReady(catalog.applications.clone()))
        .is_err()
    {
        eprintln!("vanta-shell: application event channel closed during catalog delivery");
        return;
    }

    loop {
        match receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(WorkerCommand::Launch(application_id)) => {
                launch_application(&application_id, &catalog, &process_manager, &event_sender);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        loop {
            match process_events.try_recv() {
                Ok(event) => {
                    if event_sender.send(ApplicationEvent::Process(event)).is_err() {
                        return;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
    }
}

fn launch_application(
    application_id: &str,
    catalog: &ApplicationCatalog,
    process_manager: &ProcessManager,
    event_sender: &channel::Sender<ApplicationEvent>,
) {
    let Some(application) = catalog.find(application_id).cloned() else {
        let _ = event_sender.send(ApplicationEvent::LaunchFailed {
            application_id: application_id.to_owned(),
            error: "Application ID was not found in the discovered catalog".into(),
        });
        return;
    };

    let plan = match prepare_launch(&application, &LaunchInput::default()) {
        Ok(plan) => plan,
        Err(error) => {
            let _ = event_sender.send(ApplicationEvent::LaunchFailed {
                application_id: application_id.to_owned(),
                error: error.to_string(),
            });
            return;
        }
    };

    match plan {
        plan @ LaunchPlan::Process { .. } => {
            if let Err(error) = process_manager.launch(plan) {
                let _ = event_sender.send(ApplicationEvent::LaunchFailed {
                    application_id: application_id.to_owned(),
                    error: error.to_string(),
                });
            }
        }
        plan @ (LaunchPlan::DbusActivate { .. } | LaunchPlan::DbusOpen { .. }) => {
            match activate_dbus_blocking(&application, &plan) {
                Ok(()) => {
                    let _ = event_sender.send(ApplicationEvent::DbusActivated {
                        application_id: application_id.to_owned(),
                    });
                }
                Err(error) => {
                    let _ = event_sender.send(ApplicationEvent::LaunchFailed {
                        application_id: application_id.to_owned(),
                        error: error.to_string(),
                    });
                }
            }
        }
        LaunchPlan::Terminal {
            executable,
            arguments,
            working_directory,
            startup_wm_class,
        } => {
            let _ = event_sender.send(ApplicationEvent::TerminalRequested {
                application_id: application_id.to_owned(),
                executable,
                arguments,
                working_directory,
                startup_wm_class,
            });
        }
    }
}

// Keep the process type visible to callers integrating application state.
#[allow(dead_code)]
fn _process_type_contract(_: RunningProcess) {}
