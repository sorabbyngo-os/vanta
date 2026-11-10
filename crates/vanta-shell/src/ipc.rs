use std::{
    env,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use smithay_client_toolkit::reexports::calloop::channel;
use vanta_ipc::{Action, Request, Response, PROTOCOL_VERSION};

const CLIENT_ID: &str = "vanta-shell";
const MAX_FRAME_SIZE: usize = 8 * 1024;
const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(6);
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

pub struct IpcEvent {
    pub request_id: u64,
    pub result: Result<Response, String>,
}

enum WorkerCommand {
    Request { request_id: u64, action: Action },
}

pub struct IpcClient {
    sender: Option<mpsc::Sender<WorkerCommand>>,
    next_request_id: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
}

impl IpcClient {
    pub fn new(response_sender: channel::Sender<IpcEvent>) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<WorkerCommand>();

        let worker = thread::Builder::new()
            .name("vanta-shell-ipc".into())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    let WorkerCommand::Request { request_id, action } = command;

                    let result = perform_request(request_id, action);

                    if response_sender
                        .send(IpcEvent { request_id, result })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;

        Ok(Self {
            sender: Some(sender),
            next_request_id: Arc::new(AtomicU64::new(1)),
            worker: Some(worker),
        })
    }

    pub fn request(&self, action: Action) -> Result<u64, String> {
        let request_id = self
            .next_request_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1).filter(|_| current != 0)
            })
            .map_err(|_| "IPC request ID space exhausted".to_owned())?;

        self.sender
            .as_ref()
            .ok_or_else(|| "IPC worker is unavailable".to_owned())?
            .send(WorkerCommand::Request { request_id, action })
            .map_err(|_| "IPC worker is unavailable".to_owned())?;

        Ok(request_id)
    }
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        // Close the command channel before joining the worker.
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn socket_path() -> Result<PathBuf, String> {
    let runtime_dir =
        env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_owned())?;

    Ok(PathBuf::from(runtime_dir).join("vanta-ipc.sock"))
}

fn perform_request(request_id: u64, action: Action) -> Result<Response, String> {
    let request = Request {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        client_id: CLIENT_ID.to_owned(),
        action,
    };

    request
        .validate()
        .map_err(|error| format!("Invalid IPC request: {error:?}"))?;

    let path = socket_path()?;

    let mut stream = UnixStream::connect(&path)
        .map_err(|error| format!("Cannot connect to Vanta compositor: {error}"))?;

    stream
        .set_read_timeout(Some(SOCKET_READ_TIMEOUT))
        .map_err(|error| format!("Cannot set IPC read timeout: {error}"))?;

    stream
        .set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))
        .map_err(|error| format!("Cannot set IPC write timeout: {error}"))?;

    let payload = serde_json::to_vec(&request)
        .map_err(|error| format!("Cannot serialize IPC request: {error}"))?;

    if payload.len() >= MAX_FRAME_SIZE {
        return Err("IPC request exceeds the maximum frame size".into());
    }

    stream
        .write_all(&payload)
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|error| format!("Cannot send IPC request: {error}"))?;

    let response_bytes = read_frame(&mut stream)?;

    let response: Response = serde_json::from_slice(&response_bytes)
        .map_err(|error| format!("Invalid IPC response JSON: {error}"))?;

    response
        .validate()
        .map_err(|error| format!("Invalid IPC response: {error:?}"))?;

    if response.request_id != request_id {
        return Err(format!(
            "IPC response ID mismatch: expected {request_id}, received {}",
            response.request_id
        ));
    }

    Ok(response)
}

fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>, String> {
    let mut frame = Vec::with_capacity(512);
    let mut byte = [0u8; 1];

    loop {
        match stream.read(&mut byte) {
            Ok(0) => {
                return Err(
                    "Compositor closed the IPC connection before sending a complete response"
                        .into(),
                )
            }
            Ok(_) if byte[0] == b'\n' => return Ok(frame),
            Ok(_) => {
                if frame.len() >= MAX_FRAME_SIZE {
                    return Err("IPC response exceeds the maximum frame size".into());
                }

                frame.push(byte[0]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Cannot read IPC response: {error}")),
        }
    }
}
