use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    sync::Arc,
};

use vanta_ipc::{Action, ActionResult, MinimizedWindowInfo, Request, RequestStatus, Response, PROTOCOL_VERSION};

use smithay::{
    backend::renderer::element::solid::SolidColorBuffer,
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{IsAlive, Logical, Point, Rectangle, Serial, Size, SERIAL_COUNTER},
    wayland::{
        compositor::{with_states, CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::wlr_layer::WlrLayerShellState,
        shell::xdg::{XdgShellState, XdgToplevelSurfaceData},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

#[derive(Debug, Clone)]
pub struct MinimizedWindowState {
    pub location: Point<i32, Logical>,
    pub size: Size<i32, Logical>,
    pub was_floating: bool,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

pub struct Vanta {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub(crate) output: Option<Output>,
    pub(crate) snap_preview: Option<Rectangle<i32, Logical>>,
    pub(crate) snap_preview_buffer: SolidColorBuffer,
    pub(crate) minimize_preview: Option<Rectangle<i32, Logical>>,
    pub(crate) minimize_preview_buffer: SolidColorBuffer,
    pub(crate) floating_windows: HashSet<u64>,
    pub(crate) layout_sizes: HashMap<u64, Size<i32, Logical>>,
    pub(crate) minimized_windows: HashMap<u64, MinimizedWindowState>,
    /// Vanta-assigned stable IDs mapped to Smithay window handles.
    pub windows: HashMap<u64, Window>,
    window_ids: WindowIdAllocator,
    pub loop_signal: LoopSignal,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Vanta>,
    pub data_device_state: DataDeviceState,
    pub layer_shell_state: WlrLayerShellState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
}

impl Vanta {
    pub fn new(event_loop: &mut EventLoop<Self>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        // Here we initialize implementations of some wayland protocols
        // Some of them require us to implement traits on the Vanta state,
        // you can find those implementations in the `crate::handlers` module

        // Initialize protocols needed for displaying windows
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let popups = PopupManager::default();

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        // Data device is responsible for clipboard and drag-and-drop
        let data_device_state = DataDeviceState::new::<Self>(&dh);

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        seat.add_keyboard(Default::default(), 200, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();
        let output = None;
        let snap_preview = None;
        let snap_preview_buffer = SolidColorBuffer::new((0, 0), [0.20, 0.48, 0.95, 0.24]);
        let minimize_preview = None;
        let minimize_preview_buffer = SolidColorBuffer::new((0, 0), [0.95, 0.62, 0.16, 0.32]);
        let floating_windows = HashSet::new();
        let layout_sizes = HashMap::new();
        let minimized_windows = HashMap::new();
        let windows = HashMap::new();
        let window_ids = WindowIdAllocator::default();

        // Setup a wayland socket that will be used to accept clients
        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh,

            space,
            output,
            snap_preview,
            snap_preview_buffer,
            minimize_preview,
            minimize_preview_buffer,
            floating_windows,
            layout_sizes,
            minimized_windows,
            windows,
            window_ids,
            loop_signal,
            socket_name,

            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            popups,
            seat,
            layer_shell_state,
        }
    }

    fn init_wayland_listener(
        display: Display<Vanta>,
        event_loop: &mut EventLoop<Self>,
    ) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    /// Register a newly created window and return its stable Vanta ID.
    pub fn register_window(&mut self, window: Window) -> Option<u64> {
        let id = self.window_ids.allocate()?;
        self.windows.insert(id, window);
        Some(id)
    }

    /// Raise a window while preserving the floating-above-tiled stacking policy.
    pub(crate) fn raise_window_with_policy(&mut self, window: &Window) {
        self.space.raise_element(window, true);

        let mut floating: Vec<(u64, Window)> = self
            .floating_windows
            .iter()
            .filter_map(|id| {
                self.windows
                    .get(id)
                    .map(|candidate| (*id, candidate.clone()))
            })
            .filter(|(_, candidate)| {
                candidate.alive() && self.space.elements().any(|mapped| mapped == candidate)
            })
            .collect();

        floating.sort_by_key(|(id, _)| *id);

        let target_is_floating = floating.iter().any(|(_, candidate)| candidate == window);

        for (_, floating_window) in &floating {
            if floating_window != window {
                self.space.raise_element(floating_window, false);
            }
        }

        // Keep the explicitly focused floating window above other floating windows.
        if target_is_floating {
            self.space.raise_element(window, false);
        }
    }

    /// Focus a registered, live, mapped window by its stable Vanta ID.
    pub fn focus_window(&mut self, id: u64) -> Result<(), String> {
        self.focus_window_with_serial(id, SERIAL_COUNTER.next_serial())
    }

    /// Focus a window while preserving the serial supplied by the input event.
    pub fn focus_window_with_serial(&mut self, id: u64, serial: Serial) -> Result<(), String> {
        let window = self
            .windows
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("Unknown window ID: {id}"))?;

        if !window.alive() {
            return Err(format!("Window {id} is no longer alive"));
        }

        if !self.space.elements().any(|mapped| mapped == &window) {
            return Err(format!("Window {id} is not currently mapped"));
        }

        let surface = window
            .toplevel()
            .filter(|toplevel| toplevel.alive())
            .map(|toplevel| toplevel.wl_surface().clone())
            .ok_or_else(|| format!("Window {id} has no live toplevel surface"))?;

        let keyboard = self
            .seat
            .get_keyboard()
            .ok_or_else(|| "No keyboard is available on the Vanta seat".to_string())?;

        self.raise_window_with_policy(&window);

        let mapped_windows: Vec<Window> = self.space.elements().cloned().collect();

        for mapped_window in mapped_windows {
            mapped_window.set_activated(mapped_window == window);

            if let Some(toplevel) = mapped_window.toplevel() {
                toplevel.send_pending_configure();
            }
        }

        keyboard.set_focus(self, Some(surface), serial);

        Ok(())
    }

    /// Clear keyboard focus and deactivate every mapped window.
    pub fn clear_focus_with_serial(&mut self, serial: Serial) -> Result<(), String> {
        let keyboard = self
            .seat
            .get_keyboard()
            .ok_or_else(|| "No keyboard is available on the Vanta seat".to_string())?;

        let mapped_windows: Vec<Window> = self.space.elements().cloned().collect();

        for window in mapped_windows {
            window.set_activated(false);

            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        }

        keyboard.set_focus(self, None, serial);

        Ok(())
    }

    /// Minimize a mapped window without unmapping its Wayland protocol role.
    pub fn minimize_window(&mut self, id: u64) -> Result<(), String> {
        if self.minimized_windows.contains_key(&id) {
            return Err(format!("Window {id} is already minimized"));
        }

        let window = self.windows.get(&id).cloned()
            .ok_or_else(|| format!("Unknown window ID: {id}"))?;

        if !window.alive() {
            return Err(format!("Window {id} is no longer alive"));
        }

        let location = self.space.element_location(&window)
            .ok_or_else(|| format!("Window {id} is not currently mapped"))?;

        let size = self.space.element_geometry(&window)
            .map(|geometry| geometry.size)
            .unwrap_or_else(|| window.geometry().size);

        let surface = window.toplevel()
            .filter(|toplevel| toplevel.alive())
            .map(|toplevel| toplevel.wl_surface().clone())
            .ok_or_else(|| format!("Window {id} has no live toplevel surface"))?;

        let (title, app_id) = with_states(&surface, |states| {
            states.data_map.get::<XdgToplevelSurfaceData>().map(|data| {
                let role = data.lock().unwrap();
                (role.title.clone(), role.app_id.clone())
            })
        }).unwrap_or((None, None));

        let was_floating = self.floating_windows.contains(&id);

        let keyboard = self.seat.get_keyboard()
            .ok_or_else(|| "No keyboard is available on the Vanta seat".to_string())?;

        let had_keyboard_focus = keyboard.current_focus().as_ref() == Some(&surface);

        window.set_activated(false);
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_pending_configure();
        }

        if had_keyboard_focus {
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }

        self.space.unmap_elem(&window);
        self.layout_sizes.remove(&id);

        self.minimized_windows.insert(id, MinimizedWindowState {
            location,
            size,
            was_floating,
            title,
            app_id,
        });

        self.relayout_windows();
        Ok(())
    }

    /// Restore the original live window handle and its saved geometry.
    pub fn restore_window(&mut self, id: u64) -> Result<(), String> {
        let saved = self.minimized_windows.get(&id).cloned()
            .ok_or_else(|| format!("Window {id} is not minimized"))?;

        let window = self.windows.get(&id).cloned()
            .ok_or_else(|| format!("Unknown window ID: {id}"))?;

        if !window.alive() {
            return Err(format!("Window {id} is no longer alive"));
        }

        if self.space.elements().any(|mapped| mapped == &window) {
            return Err(format!("Window {id} is already mapped"));
        }

        let toplevel = window.toplevel()
            .filter(|toplevel| toplevel.alive())
            .ok_or_else(|| format!("Window {id} has no live toplevel surface"))?;

        self.space.map_element(window.clone(), saved.location, true);

        if saved.was_floating {
            self.floating_windows.insert(id);
            toplevel.with_pending_state(|state| {
                state.size = Some(saved.size);
            });
            toplevel.send_pending_configure();
        } else {
            self.floating_windows.remove(&id);
        }

        self.layout_sizes.remove(&id);
        self.minimized_windows.remove(&id);

        if saved.was_floating {
            self.raise_window_with_policy(&window);
        } else {
            self.relayout_windows();
        }

        self.focus_window(id)
    }

    /// Return minimized windows in stable ID order for shell thumbnail rendering.
    pub fn list_minimized_windows(&self) -> Vec<MinimizedWindowInfo> {
        let mut windows: Vec<MinimizedWindowInfo> = self.minimized_windows
            .iter()
            .map(|(id, state)| MinimizedWindowInfo {
                id: *id,
                title: state.title.clone(),
                app_id: state.app_id.clone(),
            })
            .collect();

        windows.sort_by_key(|window| window.id);
        windows
    }

    /// Ask a registered, live toplevel client to close its window.
    ///
    /// The window remains registered until the client destroys its toplevel
    /// and Smithay invokes `toplevel_destroyed`.
    pub fn close_window(&mut self, id: u64) -> Result<(), String> {
        let window = self
            .windows
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("Unknown window ID: {id}"))?;

        if !window.alive() {
            return Err(format!("Window {id} is no longer alive"));
        }

        let toplevel = window
            .toplevel()
            .filter(|toplevel| toplevel.alive())
            .ok_or_else(|| format!("Window {id} has no live toplevel surface"))?;

        toplevel.send_close();

        Ok(())
    }

    /// Remove a destroyed toplevel from the registry and visible space.
    pub fn unregister_window_for_surface(&mut self, surface: &WlSurface) {
        let removed_ids: Vec<u64> = self.windows
            .iter()
            .filter_map(|(id, window)| {
                window.toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == surface)
                    .then_some(*id)
            })
            .collect();

        for id in &removed_ids {
            self.minimized_windows.remove(id);
            self.floating_windows.remove(id);
            self.layout_sizes.remove(id);
        }

        self.windows.retain(|_, window| {
            !window.toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == surface)
        });

        let window_to_unmap = self.space.elements()
            .find(|window| {
                window.toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == surface)
            })
            .cloned();

        if let Some(window) = window_to_unmap {
            self.space.unmap_elem(&window);
        }

        self.prune_layout_state();
        self.relayout_windows();
    }

    /// Validate and execute an IPC action on the compositor thread.
    pub fn dispatch_ipc_request(&mut self, request: Request) -> Response {
        let request_id = request.request_id;

        let response = |status, result| Response {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            status,
            result: Some(result),
        };

        if let Err(error) = request.validate() {
            return response(
                RequestStatus::Rejected,
                ActionResult::Err {
                    message: format!("Invalid IPC request: {error:?}"),
                },
            );
        }

        match request.action {
            Action::FocusWindow { id } => match self.focus_window(id) {
                Ok(()) => response(RequestStatus::Completed, ActionResult::Ok),
                Err(message) => response(RequestStatus::Rejected, ActionResult::Err { message }),
            },

            Action::CloseWindow { id } => match self.close_window(id) {
                Ok(()) => response(RequestStatus::Completed, ActionResult::Ok),
                Err(message) => response(RequestStatus::Rejected, ActionResult::Err { message }),
            },

            Action::ToggleFloat { id } => match self.toggle_float(id) {
                Ok(()) => response(RequestStatus::Completed, ActionResult::Ok),
                Err(message) => response(RequestStatus::Rejected, ActionResult::Err { message }),
            },

            Action::MinimizeWindow { id } => match self.minimize_window(id) {
                Ok(()) => response(RequestStatus::Completed, ActionResult::Ok),
                Err(message) => response(RequestStatus::Rejected, ActionResult::Err { message }),
            },

            Action::RestoreWindow { id } => match self.restore_window(id) {
                Ok(()) => response(RequestStatus::Completed, ActionResult::Ok),
                Err(message) => response(RequestStatus::Rejected, ActionResult::Err { message }),
            },

            Action::ListMinimizedWindows => response(
                RequestStatus::Completed,
                ActionResult::MinimizedWindows {
                    windows: self.list_minimized_windows(),
                },
            ),

            Action::SwitchWorkspace { .. } => response(
                RequestStatus::Rejected,
                ActionResult::Err {
                    message: "SwitchWorkspace is not implemented yet".to_owned(),
                },
            ),

            Action::RequestIntent { .. } => response(
                RequestStatus::Rejected,
                ActionResult::Err {
                    message: "RequestIntent requires the AION mediation service".to_owned(),
                },
            ),
        }
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            })
    }
}

#[derive(Debug)]
struct WindowIdAllocator {
    next: Option<u64>,
}

impl Default for WindowIdAllocator {
    fn default() -> Self {
        Self { next: Some(1) }
    }
}

impl WindowIdAllocator {
    fn allocate(&mut self) -> Option<u64> {
        let id = self.next?;
        self.next = id.checked_add(1);
        Some(id)
    }
}

#[cfg(test)]
mod window_id_tests {
    use super::WindowIdAllocator;

    #[test]
    fn allocates_monotonically_increasing_ids() {
        let mut allocator = WindowIdAllocator::default();
        assert_eq!(allocator.allocate(), Some(1));
        assert_eq!(allocator.allocate(), Some(2));
        assert_eq!(allocator.allocate(), Some(3));
    }

    #[test]
    fn allocates_max_id_without_wrapping() {
        let mut allocator = WindowIdAllocator {
            next: Some(u64::MAX),
        };
        assert_eq!(allocator.allocate(), Some(u64::MAX));
        assert_eq!(allocator.allocate(), None);
    }
}

/// Data associated with a wayland client that connects to Vanta.
/// One instance of this type per client.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
