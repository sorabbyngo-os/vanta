#![allow(irrefutable_let_patterns)]

mod handlers;

mod backend;
mod grabs;
mod input;
mod ipc;
mod layout;
mod native;
mod scene;
mod state;
mod winit;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::Vanta;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let backend = backend::BackendKind::from_env_and_args()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;

    let mut event_loop: EventLoop<Vanta> = EventLoop::try_new()?;

    let display: Display<Vanta> = Display::new()?;

    let mut state = Vanta::new(&mut event_loop, display);

    // Start the local control socket before entering the compositor loop.
    let _ipc_server = ipc::start(&mut event_loop)?;

    // Initialize the selected backend. Keep the native session alive for the
    // entire compositor lifetime.
    let _native_session = match backend {
        backend::BackendKind::Winit => {
            crate::winit::init_winit(&mut event_loop, &mut state)?;
            None
        }
        backend::BackendKind::Native => {
            let session = crate::native::init_native(&mut event_loop, &mut state)?;

            // Connect the shell to Vanta's Wayland socket in native mode too.
            // Start it once; it can receive output configuration after hotplug.
            unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };
            spawn_shell();

            Some(session)
        }
    };

    // Winit is a nested development backend. Native mode will launch the shell
    // only after its DRM output has been initialized.
    if matches!(backend, backend::BackendKind::Winit) {
        unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };
        spawn_shell();
    }

    event_loop.run(None, &mut state, move |_| {
        // Vanta is running
    })?;

    Ok(())
}

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}

fn spawn_shell() {
    let shell_path = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join("vanta-shell")));

    let Some(shell_path) = shell_path else {
        eprintln!("vanta-comp: cannot determine the shell executable path");
        return;
    };

    if !shell_path.is_file() {
        eprintln!(
            "vanta-comp: shell executable not found at {}; build with cargo build -p vanta-shell",
            shell_path.display()
        );
        return;
    }

    match std::process::Command::new(&shell_path).spawn() {
        Ok(child) => {
            eprintln!("vanta-comp: started vanta-shell (pid {})", child.id());
        }
        Err(error) => {
            eprintln!("vanta-comp: failed to start vanta-shell: {error}");
        }
    }
}
