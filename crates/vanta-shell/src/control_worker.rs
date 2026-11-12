use std::{
    io,
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

use smithay_client_toolkit::reexports::calloop::channel;

use crate::render::quick_settings::{ControlAction, ControlState};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub enum ControlEvent {
    State(ControlState),
}

enum WorkerCommand {
    SetVisible(bool),
    Perform(ControlAction),
}

pub struct ControlWorker {
    sender: Option<mpsc::Sender<WorkerCommand>>,
    worker: Option<JoinHandle<()>>,
}

impl ControlWorker {
    pub fn new(event_sender: channel::Sender<ControlEvent>) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<WorkerCommand>();

        let worker = thread::Builder::new()
            .name("vanta-shell-controls".into())
            .spawn(move || {
                // All hardware and system queries run on this thread.
                let mut state = ControlState::read();

                if event_sender
                    .send(ControlEvent::State(state.clone()))
                    .is_err()
                {
                    return;
                }

                let mut visible = false;
                let mut pending_command = None;

                loop {
                    let message = if let Some(command) = pending_command.take() {
                        Ok(command)
                    } else if visible {
                        receiver.recv_timeout(POLL_INTERVAL)
                    } else {
                        match receiver.recv() {
                            Ok(command) => Ok(command),
                            Err(_) => break,
                        }
                    };

                    match message {
                        Ok(WorkerCommand::SetVisible(next_visible)) => {
                            let became_visible = !visible && next_visible;
                            visible = next_visible;

                            // Refresh after opening, but never on the UI thread.
                            if became_visible {
                                state.refresh();

                                if event_sender
                                    .send(ControlEvent::State(state.clone()))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        Ok(WorkerCommand::Perform(ControlAction::SetVolumePercent(
                            mut percent,
                        ))) => {
                            // Keep only the newest consecutive slider value.
                            while let Ok(command) = receiver.try_recv() {
                                match command {
                                    WorkerCommand::Perform(ControlAction::SetVolumePercent(
                                        next,
                                    )) => percent = next,
                                    other => {
                                        pending_command = Some(other);
                                        break;
                                    }
                                }
                            }

                            state.perform(ControlAction::SetVolumePercent(percent));

                            if event_sender
                                .send(ControlEvent::State(state.clone()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(WorkerCommand::Perform(action)) => {
                            state.perform(action);

                            if event_sender
                                .send(ControlEvent::State(state.clone()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if state.refresh()
                                && event_sender
                                    .send(ControlEvent::State(state.clone()))
                                    .is_err()
                            {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?;

        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    fn send(&self, command: WorkerCommand) -> Result<(), String> {
        self.sender
            .as_ref()
            .ok_or_else(|| "Controls worker is unavailable".to_owned())?
            .send(command)
            .map_err(|_| "Controls worker has stopped".to_owned())
    }

    pub fn set_visible(&self, visible: bool) -> Result<(), String> {
        self.send(WorkerCommand::SetVisible(visible))
    }

    pub fn perform(&self, action: ControlAction) -> Result<(), String> {
        self.send(WorkerCommand::Perform(action))
    }
}

impl Drop for ControlWorker {
    fn drop(&mut self) {
        // Closing the command channel wakes the worker before joining it.
        self.sender.take();

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
