use std::{
    fs,
    io::{self, BufRead, Read, Write},
    os::{
        unix::fs::PermissionsExt,
        unix::net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use smithay::reexports::calloop::{channel, channel::Event, EventLoop};
use vanta_ipc::{ActionResult, Request, RequestStatus, Response, PROTOCOL_VERSION};

use crate::Vanta;

const MAX_FRAME_SIZE: usize = 8 * 1024;
const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(2);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

struct IpcMessage {
    request: Request,
    reply: mpsc::SyncSender<Response>,
}

pub struct IpcServer {
    socket_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }

        if let Err(error) = fs::remove_file(&self.socket_path) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    path = %self.socket_path.display(),
                    %error,
                    "Could not remove Vanta IPC socket"
                );
            }
        }
    }
}

/// Start the local IPC service and register its request channel with Calloop.
pub fn start(event_loop: &mut EventLoop<Vanta>) -> io::Result<IpcServer> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "XDG_RUNTIME_DIR is not set; cannot create the Vanta IPC socket",
            )
        })?;

    let socket_path = runtime_dir.join("vanta-ipc.sock");
    let listener = UnixListener::bind(&socket_path)?;

    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;

    let (sender, receiver) = channel::channel::<IpcMessage>();

    event_loop
        .handle()
        .insert_source(receiver, |event, _, state| {
            if let Event::Msg(message) = event {
                let response = state.dispatch_ipc_request(message.request);
                let _ = message.reply.send(response);
            }
        })
        .map_err(|error| {
            io::Error::other(format!("Could not register IPC event source: {error}"))
        })?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown);

    let worker = thread::Builder::new()
        .name("vanta-ipc".to_owned())
        .spawn(move || {
            while !worker_shutdown.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => handle_client(stream, &sender),

                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }

                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}

                    Err(error) => {
                        tracing::error!(%error, "Vanta IPC listener failed");
                        break;
                    }
                }
            }
        })?;

    tracing::info!(path = %socket_path.display(), "Vanta IPC service started");

    Ok(IpcServer {
        socket_path,
        shutdown,
        worker: Some(worker),
    })
}

fn handle_client(stream: UnixStream, sender: &channel::Sender<IpcMessage>) {
    let _ = stream.set_read_timeout(Some(CLIENT_READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_READ_TIMEOUT));

    let mut reader = io::BufReader::new(stream);
    let mut frame = Vec::with_capacity(1024);

    let read_result = {
        let mut limited = reader.by_ref().take((MAX_FRAME_SIZE + 1) as u64);
        limited.read_until(b'\n', &mut frame)
    };

    match read_result {
        Ok(0) => return,
        Ok(size) if size <= MAX_FRAME_SIZE && frame.last() == Some(&b'\n') => {}
        Ok(_) => {
            tracing::warn!("Rejected oversized or unterminated IPC frame");
            return;
        }
        Err(error) => {
            tracing::debug!(%error, "Could not read IPC request");
            return;
        }
    }

    frame.pop();

    if frame.last() == Some(&b'\r') {
        frame.pop();
    }

    let request: Request = match serde_json::from_slice(&frame) {
        Ok(request) => request,
        Err(error) => {
            tracing::debug!(%error, "Rejected malformed IPC JSON");
            return;
        }
    };

    if let Err(error) = request.validate() {
        if request.request_id == 0 {
            tracing::warn!("Rejected IPC request with zero request ID");
            return;
        }

        let response = Response {
            protocol_version: PROTOCOL_VERSION,
            request_id: request.request_id,
            status: RequestStatus::Rejected,
            result: Some(ActionResult::Err {
                message: format!("Invalid IPC request: {error:?}"),
            }),
        };

        let _ = write_response(reader.into_inner(), response);
        return;
    }

    let (reply_tx, reply_rx) = mpsc::sync_channel(1);

    if sender
        .send(IpcMessage {
            request,
            reply: reply_tx,
        })
        .is_err()
    {
        tracing::warn!("IPC request channel is closed");
        return;
    }

    match reply_rx.recv_timeout(RESPONSE_TIMEOUT) {
        Ok(response) => {
            if let Err(error) = write_response(reader.into_inner(), response) {
                tracing::debug!(%error, "Could not write IPC response");
            }
        }
        Err(error) => {
            tracing::warn!(%error, "Timed out waiting for compositor IPC response");
        }
    }
}

fn write_response(mut stream: UnixStream, response: Response) -> io::Result<()> {
    response
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")))?;

    let encoded = serde_json::to_vec(&response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    stream.write_all(&encoded)?;
    stream.write_all(b"\n")?;
    stream.flush()
}
