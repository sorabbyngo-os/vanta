use std::{
    io,
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

use smithay_client_toolkit::reexports::calloop::channel;

use crate::{
    audio::AudioController,
    render::quick_settings::{ControlAction, ControlState},
};

const POLL_INTERVAL: Duration = Duration::from_millis(750);

pub enum ControlEvent {
    State(ControlState),
}

enum WorkerCommand {
    SetVisible(bool),
    RefreshConnectivity,
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
                let mut state = ControlState::read();
                let mut audio = AudioController::new();
                refresh_audio_state(&mut state, &mut audio);

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

                            if became_visible {
                                state.refresh();
                                refresh_audio_state(&mut state, &mut audio);
                                if event_sender
                                    .send(ControlEvent::State(state.clone()))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        Ok(WorkerCommand::RefreshConnectivity) => {
                            state.refresh_connectivity();
                            if event_sender
                                .send(ControlEvent::State(state.clone()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(WorkerCommand::Perform(
                            action @ (ControlAction::SetVolumePercent(_)
                            | ControlAction::SetSystemVolumePercent(_)),
                        )) => {
                            let mut percent = match action {
                                ControlAction::SetVolumePercent(value)
                                | ControlAction::SetSystemVolumePercent(value) => value,
                                _ => unreachable!(),
                            };

                            // Keep only the latest consecutive system-volume update.
                            while let Ok(command) = receiver.try_recv() {
                                match command {
                                    WorkerCommand::Perform(
                                        ControlAction::SetVolumePercent(next)
                                        | ControlAction::SetSystemVolumePercent(next),
                                    ) => percent = next,
                                    other => {
                                        pending_command = Some(other);
                                        break;
                                    }
                                }
                            }

                            match audio.set_master_percent(percent) {
                                Ok(()) => {
                                    state.system_volume_percent = percent.min(100);
                                    state.volume = percent.min(100) as f32 / 100.0;
                                }
                                Err(error) => {
                                    eprintln!("vanta-shell: system master volume failed: {error}")
                                }
                            }

                            refresh_audio_state(&mut state, &mut audio);
                            if event_sender
                                .send(ControlEvent::State(state.clone()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(WorkerCommand::Perform(ControlAction::SetOutputVolumePercent(
                            percent,
                        ))) => {
                            if let Err(error) = audio.set_output_percent(percent) {
                                eprintln!("vanta-shell: output volume failed: {error}");
                            }
                            refresh_audio_state(&mut state, &mut audio);
                            if event_sender
                                .send(ControlEvent::State(state.clone()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(WorkerCommand::Perform(ControlAction::ToggleMute)) => {
                            if let Err(error) = audio.set_output_muted(!state.output_muted) {
                                eprintln!("vanta-shell: output mute failed: {error}");
                            }
                            refresh_audio_state(&mut state, &mut audio);
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
                            let controls_changed = state.refresh();
                            let audio_changed = refresh_audio_state(&mut state, &mut audio);

                            if (controls_changed || audio_changed)
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

    pub fn refresh_connectivity(&self) -> Result<(), String> {
        self.send(WorkerCommand::RefreshConnectivity)
    }

    pub fn perform(&self, action: ControlAction) -> Result<(), String> {
        self.send(WorkerCommand::Perform(action))
    }
}

fn refresh_audio_state(state: &mut ControlState, audio: &mut AudioController) -> bool {
    if let Err(error) = audio.refresh_streams() {
        eprintln!("vanta-shell: audio stream refresh failed: {error}");
    }

    let before = (
        state.system_volume_percent,
        state.output_volume_percent,
        state.output_description.clone(),
        state.output_is_bluetooth,
        state.output_muted,
        state.audio_stream_count,
    );

    state.system_volume_percent = audio.master_percent();
    state.volume = state.system_volume_percent as f32 / 100.0;
    state.audio_stream_count = audio.stream_count();

    match audio.read_output() {
        Ok(Some(output)) => {
            state.output_volume_percent = output.volume_percent;
            state.output_description = output.description;
            state.output_is_bluetooth = output.is_bluetooth;
            state.output_muted = output.muted;
            state.audio_muted = output.muted;
        }
        Ok(None) => {
            state.output_volume_percent = 0;
            state.output_description.clear();
            state.output_is_bluetooth = false;
            state.output_muted = false;
            state.audio_muted = false;
        }
        Err(error) => eprintln!("vanta-shell: output device refresh failed: {error}"),
    }

    before
        != (
            state.system_volume_percent,
            state.output_volume_percent,
            state.output_description.clone(),
            state.output_is_bluetooth,
            state.output_muted,
            state.audio_stream_count,
        )
}

impl Drop for ControlWorker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
