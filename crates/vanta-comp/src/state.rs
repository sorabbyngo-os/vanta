use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    sync::Arc,
};

use crate::layout::SnapTarget;
use crate::layout_engine::SplitTree;

use vanta_ipc::{
    Action, ActionResult, LayoutMode, MinimizedWindowInfo, OverviewWindowInfo,
    OverviewWorkspaceInfo, Request, RequestStatus, Response, PROTOCOL_VERSION,
};

use smithay::{
    backend::{
        allocator::Fourcc,
        input::TouchSlot,
        renderer::element::{memory::MemoryRenderBuffer, solid::SolidColorBuffer},
    },
    desktop::{layer_map_for_output, PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{channel, generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{IsAlive, Logical, Point, Rectangle, Serial, Size, SERIAL_COUNTER},
    wayland::{
        compositor::{with_states, CompositorClientState, CompositorState},
        dmabuf::{DmabufGlobal, DmabufState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::wlr_layer::{Layer, WlrLayerShellState},
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
    pub snap_target: Option<SnapTarget>,
}

#[derive(Debug, Clone)]
pub(crate) enum WindowTransition {
    Minimizing(MinimizedWindowState),
    Restoring(MinimizedWindowState),
}

impl WindowTransition {
    pub(crate) fn is_minimizing(&self) -> bool {
        matches!(self, Self::Minimizing(_))
    }
}

/// Build a reusable translucent glass texture for overview cards.
pub(crate) fn create_overview_glass_buffer() -> MemoryRenderBuffer {
    const WIDTH: i32 = 256;
    const HEIGHT: i32 = 160;
    const RADIUS: f32 = 18.0;

    let mut buffer = MemoryRenderBuffer::new(
        Fourcc::Argb8888,
        (WIDTH, HEIGHT),
        1,
        smithay::utils::Transform::Normal,
        None,
    );

    let _ = buffer.render().draw(|pixels| {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let px = x as f32;
                let py = y as f32;
                let cx = px.clamp(RADIUS, WIDTH as f32 - RADIUS);
                let cy = py.clamp(RADIUS, HEIGHT as f32 - RADIUS);
                let dx = px - cx;
                let dy = py - cy;
                let distance = (dx * dx + dy * dy).sqrt();

                if distance > RADIUS {
                    continue;
                }

                let vertical = y as f32 / HEIGHT as f32;
                let top_glow = (1.0 - vertical * 3.2).clamp(0.0, 1.0);
                let edge = if distance > RADIUS - 1.4 { 1.0 } else { 0.0 };

                let coverage = (RADIUS + 0.5 - distance).clamp(0.0, 1.0);
                if coverage <= 0.0 {
                    continue;
                }

                let alpha = (92.0 + 20.0 * (1.0 - vertical) + 22.0 * top_glow + 24.0 * edge)
                    .clamp(0.0, 160.0)
                    * coverage;

                let red = 91.0 + 28.0 * top_glow + 35.0 * edge;
                let green = 126.0 + 34.0 * top_glow + 42.0 * edge;
                let blue = 166.0 + 42.0 * top_glow + 52.0 * edge;

                let offset = ((y * WIDTH + x) * 4) as usize;
                let a = alpha / 255.0;

                // ARGB8888 is stored as BGRA bytes on little-endian systems.
                pixels[offset] = (blue * a).round() as u8;
                pixels[offset + 1] = (green * a).round() as u8;
                pixels[offset + 2] = (red * a).round() as u8;
                pixels[offset + 3] = alpha.round() as u8;
            }
        }

        Ok::<_, ()>(vec![Rectangle::from_size((WIDTH, HEIGHT).into())])
    });

    buffer
}

/// Small translucent glass triangle marking the Overview hot corner.

/// Small warm translucent glass surface for the minimize target.
pub(crate) fn create_minimize_glass_buffer() -> MemoryRenderBuffer {
    const WIDTH: i32 = 200;
    const HEIGHT: i32 = 80;
    const RADIUS: f32 = 16.0;

    let mut buffer = MemoryRenderBuffer::new(
        Fourcc::Argb8888,
        (WIDTH, HEIGHT),
        1,
        smithay::utils::Transform::Normal,
        None,
    );

    let _ = buffer.render().draw(|pixels| {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let cx = px.clamp(RADIUS, WIDTH as f32 - RADIUS);
                let cy = py.clamp(RADIUS, HEIGHT as f32 - RADIUS);
                let dx = px - cx;
                let dy = py - cy;
                let distance = (dx * dx + dy * dy).sqrt();

                if distance > RADIUS + 0.5 {
                    continue;
                }

                let coverage = (RADIUS + 0.5 - distance).clamp(0.0, 1.0);
                let vertical = y as f32 / HEIGHT as f32;
                let top_glow = (1.0 - vertical * 3.0).clamp(0.0, 1.0);
                let edge = if distance > RADIUS - 1.4 { 1.0 } else { 0.0 };
                let alpha = (48.0 + 22.0 * top_glow + 28.0 * edge) * coverage;

                let red = 255.0;
                let green = 155.0 + 55.0 * top_glow + 25.0 * edge;
                let blue = 75.0 + 35.0 * top_glow + 35.0 * edge;
                let a = alpha / 255.0;
                let offset = ((y * WIDTH + x) * 4) as usize;

                pixels[offset] = (blue * a).round() as u8;
                pixels[offset + 1] = (green * a).round() as u8;
                pixels[offset + 2] = (red * a).round() as u8;
                pixels[offset + 3] = alpha.round() as u8;
            }
        }

        Ok::<_, ()>(vec![Rectangle::from_size((WIDTH, HEIGHT).into())])
    });

    buffer
}

pub(crate) fn create_hot_corner_buffer() -> MemoryRenderBuffer {
    const SIZE: i32 = 24;

    let mut buffer = MemoryRenderBuffer::new(
        Fourcc::Argb8888,
        (SIZE, SIZE),
        1,
        smithay::utils::Transform::Normal,
        None,
    );

    let _ = buffer.render().draw(|pixels| {
        for y in 0..SIZE {
            for x in 0..SIZE {
                let distance = SIZE as f32 - (x as f32 + y as f32 + 1.0);
                let coverage = (distance + 0.5).clamp(0.0, 1.0);

                if coverage <= 0.0 {
                    continue;
                }

                let edge = if distance < 1.5 { 1.0 } else { 0.0 };
                let alpha = (17.0 + edge * 10.0) * coverage;
                let red = 105.0 + edge * 20.0;
                let green = 175.0 + edge * 20.0;
                let blue = 230.0 + edge * 18.0;
                let a = alpha / 255.0;

                let offset = ((y * SIZE + x) * 4) as usize;

                // ARGB8888 is stored as BGRA bytes on little-endian systems.
                pixels[offset] = (blue * a).round() as u8;
                pixels[offset + 1] = (green * a).round() as u8;
                pixels[offset + 2] = (red * a).round() as u8;
                pixels[offset + 3] = alpha.round() as u8;
            }
        }

        Ok::<_, ()>(vec![Rectangle::from_size((SIZE, SIZE).into())])
    });

    buffer
}

/// Fixed-width workspace indicator strip used while Overview is active.
pub(crate) fn create_overview_indicator_buffer(
    workspace_count: u32,
    active_workspace: u32,
) -> MemoryRenderBuffer {
    const WIDTH: i32 = 128;
    const HEIGHT: i32 = 20;
    const MAX_DIAMETER: f32 = 10.0;
    const MIN_DIAMETER: f32 = 2.5;

    let count = workspace_count.max(1);
    let diameter = (12.0 / (count as f32).sqrt()).clamp(MIN_DIAMETER, MAX_DIAMETER);
    let radius = diameter / 2.0;
    let first_center = 10.0_f32;
    let last_center = WIDTH as f32 - 10.0;

    let mut buffer = MemoryRenderBuffer::new(
        Fourcc::Argb8888,
        (WIDTH, HEIGHT),
        1,
        smithay::utils::Transform::Normal,
        None,
    );

    let _ = buffer.render().draw(|pixels| {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let mut color = None;

                for index in 0..count {
                    let center_x = if count == 1 {
                        WIDTH as f32 / 2.0
                    } else {
                        first_center
                            + (last_center - first_center) * index as f32 / (count - 1) as f32
                    };

                    let dx = x as f32 + 0.5 - center_x;
                    let dy = y as f32 + 0.5 - HEIGHT as f32 / 2.0;
                    let distance = (dx * dx + dy * dy).sqrt();
                    let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);

                    if coverage <= 0.0 {
                        continue;
                    }

                    let active = index == active_workspace.min(count - 1);
                    let alpha = (if active { 245.0 } else { 115.0 }) * coverage;

                    // Premultiplied BGRA for ARGB8888 on little-endian systems.
                    let (red, green, blue) = if active {
                        (132.0, 218.0, 245.0)
                    } else {
                        (205.0, 220.0, 238.0)
                    };

                    let a = alpha / 255.0;
                    color = Some((
                        (blue * a).round() as u8,
                        (green * a).round() as u8,
                        (red * a).round() as u8,
                        alpha.round() as u8,
                    ));
                    break;
                }

                if let Some((blue, green, red, alpha)) = color {
                    let offset = ((y * WIDTH + x) * 4) as usize;
                    pixels[offset] = blue;
                    pixels[offset + 1] = green;
                    pixels[offset + 2] = red;
                    pixels[offset + 3] = alpha;
                }
            }
        }

        Ok::<_, ()>(vec![Rectangle::from_size((WIDTH, HEIGHT).into())])
    });

    buffer
}

/// Shared geometry for the Strip overview, used by both rendering and input.
pub(crate) struct OverviewLayout {
    pub thumbnails: Vec<(u32, Rectangle<i32, Logical>)>,
    pub active_preview: Rectangle<i32, Logical>,
    pub new_workspace: Rectangle<i32, Logical>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TouchGesture {
    pub slot: i32,
    pub start: Point<f64, Logical>,
    pub current: Point<f64, Logical>,
}

pub struct Vanta {
    pub start_time: std::time::Instant,
    pub(crate) motion_last_tick: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub(crate) output: Option<Output>,
    pub(crate) snap_preview: Option<Rectangle<i32, Logical>>,
    pub(crate) snap_preview_buffer: SolidColorBuffer,
    pub(crate) minimize_preview: Option<Rectangle<i32, Logical>>,
    pub(crate) minimize_preview_buffer: MemoryRenderBuffer,
    pub(crate) overview_card_buffers: Vec<MemoryRenderBuffer>,
    pub(crate) overview_active_buffer: MemoryRenderBuffer,
    pub(crate) overview_indicator_buffer: MemoryRenderBuffer,
    pub(crate) hot_corner_buffer: MemoryRenderBuffer,
    pub(crate) overview_new_buffer: MemoryRenderBuffer,
    pub(crate) overview_plus_buffer: SolidColorBuffer,
    pub(crate) overview_plus_vertical_buffer: SolidColorBuffer,
    pub(crate) layout_mode: LayoutMode,
    pub(crate) floating_windows: HashSet<u64>,
    /// Windows awaiting explicit placement in Manual/Fractional mode.
    pub(crate) unplaced_windows: HashSet<u64>,
    pub(crate) snapped_windows: HashMap<u64, SnapTarget>,
    pub(crate) maximize_restore_rects: HashMap<u64, Rectangle<i32, Logical>>,
    pub(crate) layout_sizes: HashMap<u64, Size<i32, Logical>>,
    pub(crate) minimized_windows: HashMap<u64, MinimizedWindowState>,
    pub(crate) transitions: HashMap<u64, WindowTransition>,
    pub(crate) workspace_layouts: HashMap<u32, SplitTree>,
    pub(crate) workspace_count: u32,
    pub(crate) active_workspace: u32,
    pub(crate) overview_open: bool,
    pub(crate) hot_corner_armed: bool,
    pub(crate) hot_corner_hint_visible: bool,
    pub(crate) last_pointer_position: Option<Point<f64, Logical>>,
    pub(crate) touch_gesture: Option<TouchGesture>,
    pub(crate) app_touch_slots: HashSet<i32>,
    pub(crate) window_workspaces: HashMap<u64, u32>,
    pub(crate) window_locations: HashMap<u64, Point<i32, Logical>>,
    /// Vanta-assigned stable IDs mapped to Smithay window handles.
    pub windows: HashMap<u64, Window>,
    pub(crate) motion: crate::motion::MotionEngine,
    window_ids: WindowIdAllocator,
    pub loop_signal: LoopSignal,
    pub(crate) native_redraw_sender: Option<channel::Sender<()>>,
    pub(crate) native_renderer_registry: Option<crate::native::NativeRendererRegistry>,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub(crate) dmabuf_global: Option<DmabufGlobal>,
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
        let motion_last_tick = start_time;

        let dh = display.handle();

        // Here we initialize implementations of some wayland protocols
        // Some of them require us to implement traits on the Vanta state,
        // you can find those implementations in the `crate::handlers` module

        // Initialize protocols needed for displaying windows
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let dmabuf_state = DmabufState::new();
        let popups = PopupManager::default();

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        // Data device is responsible for clipboard and drag-and-drop
        let data_device_state = DataDeviceState::new::<Self>(&dh);

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "vanta");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        seat.add_keyboard(Default::default(), 200, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();
        // Advertise touchscreen capability to Wayland clients.
        seat.add_touch();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();
        let output = None;
        let snap_preview = None;
        let snap_preview_buffer = SolidColorBuffer::new((0, 0), [0.20, 0.48, 0.95, 0.24]);
        let minimize_preview = None;
        let minimize_preview_buffer = create_minimize_glass_buffer();
        let workspace_count = 1;
        let active_workspace = 0;
        let overview_card_buffers = Vec::new();
        let overview_active_buffer = create_overview_glass_buffer();
        let overview_indicator_buffer =
            create_overview_indicator_buffer(workspace_count, active_workspace);
        let hot_corner_buffer = create_hot_corner_buffer();
        let overview_new_buffer = create_overview_glass_buffer();
        let overview_plus_buffer = SolidColorBuffer::new((0, 0), [0.58, 0.84, 0.96, 0.92]);
        let overview_plus_vertical_buffer = SolidColorBuffer::new((0, 0), [0.58, 0.84, 0.96, 0.92]);
        let layout_mode = LayoutMode::Adaptive;
        let floating_windows = HashSet::new();
        let unplaced_windows = HashSet::new();
        let snapped_windows = HashMap::new();
        let maximize_restore_rects = HashMap::new();
        let layout_sizes = HashMap::new();
        let minimized_windows = HashMap::new();
        let transitions = HashMap::new();
        let workspace_layouts = HashMap::new();
        let overview_open = false;
        let hot_corner_armed = false;
        let hot_corner_hint_visible = false;
        let last_pointer_position: Option<Point<f64, Logical>> = None;
        let touch_gesture: Option<TouchGesture> = None;
        let hot_corner_hint_visible = false;
        let last_pointer_position: Option<Point<f64, Logical>> = None;
        let touch_gesture: Option<TouchGesture> = None;
        let app_touch_slots = HashSet::new();
        let window_workspaces = HashMap::new();
        let window_locations = HashMap::new();
        let windows = HashMap::new();
        let window_ids = WindowIdAllocator::default();

        // Setup a wayland socket that will be used to accept clients
        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();
        let native_redraw_sender = None;
        let native_renderer_registry = None;

        Self {
            start_time,
            motion_last_tick,
            display_handle: dh,

            space,
            output,
            snap_preview,
            snap_preview_buffer,
            minimize_preview,
            minimize_preview_buffer,
            overview_card_buffers,
            overview_active_buffer,
            overview_indicator_buffer,
            hot_corner_buffer,
            overview_new_buffer,
            overview_plus_buffer,
            overview_plus_vertical_buffer,
            layout_mode,
            floating_windows,
            unplaced_windows,
            snapped_windows,
            maximize_restore_rects,
            layout_sizes,
            minimized_windows,
            transitions,
            workspace_layouts,
            workspace_count,
            active_workspace,
            overview_open,
            hot_corner_armed,
            hot_corner_hint_visible,
            last_pointer_position,
            touch_gesture,
            app_touch_slots,
            window_workspaces,
            window_locations,
            windows,
            motion: crate::motion::MotionEngine::default(),
            window_ids,
            loop_signal,
            native_redraw_sender,
            native_renderer_registry,
            socket_name,

            compositor_state,
            xdg_shell_state,
            shm_state,
            dmabuf_state,
            dmabuf_global: None,
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
                    static DISPATCH_LOG_COUNT: std::sync::atomic::AtomicUsize =
                        std::sync::atomic::AtomicUsize::new(0);

                    // Safety: the display remains owned by the event source.
                    let dispatched = unsafe { display.get_mut().dispatch_clients(state).unwrap() };

                    let flush_result = state.display_handle.flush_clients();

                    let sequence =
                        DISPATCH_LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    if sequence < 30 {
                        eprintln!(
                            "[WAYLAND-DISPATCH] callback={} dispatched={} flush={:?}",
                            sequence, dispatched, flush_result
                        );
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
        self.window_workspaces.insert(id, self.active_workspace);
        self.window_locations.insert(id, (0, 0).into());
        if matches!(
            self.layout_mode,
            LayoutMode::Manual | LayoutMode::Fractional
        ) {
            self.unplaced_windows.insert(id);
        }
        self.motion.insert_window(id, 0.0, 0.0);
        self.windows.insert(id, window);
        Some(id)
    }

    /// Return the current workspace count and active workspace index.
    pub fn workspace_state(&self) -> (u32, u32) {
        (self.workspace_count, self.active_workspace)
    }

    /// Calculate the shared Strip overview geometry for rendering and hit-testing.
    pub(crate) fn overview_layout(&self) -> Option<OverviewLayout> {
        let output = self.output.as_ref()?;
        let geometry = self.space.output_geometry(output)?;

        let margin = 48_i32;
        let strip_y = geometry.loc.y + 72;
        let strip_height = 96_i32;
        let strip_gap = 12_i32;
        let available_width = (geometry.size.w - margin * 2).max(1);
        let slot_count = self.workspace_count.saturating_add(1).max(2) as i32;
        let thumbnail_width =
            ((available_width - strip_gap * (slot_count - 1)) / slot_count).clamp(64, 260);
        let thumbnail_height = 76_i32;
        let total_strip_width = slot_count * thumbnail_width + (slot_count - 1) * strip_gap;
        let strip_start_x = geometry.loc.x + (geometry.size.w - total_strip_width).max(0) / 2;
        let thumbnail_y = strip_y + (strip_height - thumbnail_height) / 2;

        let mut thumbnails = Vec::with_capacity(self.workspace_count as usize);

        for index in 0..self.workspace_count {
            let x = strip_start_x + index as i32 * (thumbnail_width + strip_gap);
            thumbnails.push((
                index,
                Rectangle::new(
                    (x, thumbnail_y).into(),
                    (thumbnail_width, thumbnail_height).into(),
                ),
            ));
        }

        let new_x = strip_start_x + self.workspace_count as i32 * (thumbnail_width + strip_gap);
        let new_workspace = Rectangle::new(
            (new_x, thumbnail_y).into(),
            (thumbnail_width, thumbnail_height).into(),
        );

        let preview_top = strip_y + strip_height + 32;
        let preview_bottom_margin = 104;
        let active_preview = Rectangle::new(
            (geometry.loc.x + margin, geometry.loc.y + preview_top).into(),
            (
                (geometry.size.w - margin * 2).max(1),
                (geometry.size.h - preview_top - preview_bottom_margin).max(1),
            )
                .into(),
        );

        Some(OverviewLayout {
            thumbnails,
            active_preview,
            new_workspace,
        })
    }

    /// Switch to an existing workspace, preserving each window's last location.
    pub fn switch_workspace(&mut self, index: u32) -> Result<(), String> {
        if index >= self.workspace_count {
            return Err(format!(
                "Workspace {index} does not exist (workspace count: {})",
                self.workspace_count
            ));
        }

        if index == self.active_workspace {
            return Ok(());
        }

        let current_windows: Vec<Window> = self.space.elements().cloned().collect();

        for window in &current_windows {
            if let Some(id) = self
                .windows
                .iter()
                .find_map(|(id, candidate)| (candidate == window).then_some(*id))
            {
                if let Some(location) = self.space.element_location(window) {
                    self.window_locations.insert(id, location);
                }
            }
        }

        self.clear_focus_with_serial(SERIAL_COUNTER.next_serial())?;

        for window in current_windows {
            self.space.unmap_elem(&window);
        }

        self.active_workspace = index;

        let mut destination: Vec<(u64, Window)> = self
            .windows
            .iter()
            .filter(|(id, window)| {
                self.window_workspaces.get(id) == Some(&index)
                    && !self.minimized_windows.contains_key(id)
                    && !self.transitions.contains_key(id)
                    && window.alive()
            })
            .map(|(id, window)| (*id, window.clone()))
            .collect();

        destination.sort_by_key(|(id, _)| *id);

        for (id, window) in destination {
            let location = self
                .window_locations
                .get(&id)
                .copied()
                .unwrap_or_else(|| (0, 0).into());
            self.space.map_element(window, location, false);
            self.motion
                .snap_window_position(id, f64::from(location.x), f64::from(location.y));
        }

        self.layout_sizes.clear();
        self.relayout_windows();
        self.overview_indicator_buffer =
            create_overview_indicator_buffer(self.workspace_count, self.active_workspace);
        Ok(())
    }

    /// Create a workspace and activate it.
    pub fn create_workspace(&mut self) -> Result<(), String> {
        let index = self.workspace_count;
        self.workspace_count = self
            .workspace_count
            .checked_add(1)
            .ok_or_else(|| "Workspace count limit reached".to_owned())?;

        if let Err(error) = self.switch_workspace(index) {
            self.workspace_count -= 1;
            return Err(error);
        }

        Ok(())
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

    /// Animate a mapped window out before completing its minimize operation.
    pub fn minimize_window(&mut self, id: u64) -> Result<(), String> {
        if self.minimized_windows.contains_key(&id) || self.transitions.contains_key(&id) {
            return Err(format!("Window {id} is already minimized or transitioning"));
        }

        let window = self
            .windows
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("Unknown window ID: {id}"))?;

        if !window.alive() {
            return Err(format!("Window {id} is no longer alive"));
        }

        let location = self
            .space
            .element_location(&window)
            .ok_or_else(|| format!("Window {id} is not currently mapped"))?;

        let size = self
            .space
            .element_geometry(&window)
            .map(|geometry| geometry.size)
            .unwrap_or_else(|| window.geometry().size);

        let surface = window
            .toplevel()
            .filter(|toplevel| toplevel.alive())
            .map(|toplevel| toplevel.wl_surface().clone())
            .ok_or_else(|| format!("Window {id} has no live toplevel surface"))?;

        let (title, app_id) = with_states(&surface, |states| {
            states.data_map.get::<XdgToplevelSurfaceData>().map(|data| {
                let role = data.lock().unwrap();
                (role.title.clone(), role.app_id.clone())
            })
        })
        .unwrap_or((None, None));

        let was_floating = self.floating_windows.contains(&id);
        let snap_target = self.snapped_windows.remove(&id);

        let keyboard = self
            .seat
            .get_keyboard()
            .ok_or_else(|| "No keyboard is available on the Vanta seat".to_string())?;

        let had_keyboard_focus = keyboard.current_focus().as_ref() == Some(&surface);

        // Move focus before beginning the exit animation.
        if had_keyboard_focus {
            let fallback_id = self.space.elements().rev().find_map(|candidate| {
                self.windows.iter().find_map(|(&candidate_id, registered)| {
                    (registered == candidate
                        && candidate_id != id
                        && !self.transitions.contains_key(&candidate_id)
                        && candidate.alive())
                    .then_some(candidate_id)
                })
            });

            if let Some(fallback_id) = fallback_id {
                self.focus_window_with_serial(fallback_id, SERIAL_COUNTER.next_serial())?;
            } else {
                self.clear_focus_with_serial(SERIAL_COUNTER.next_serial())?;
            }
        }

        window.set_activated(false);

        if let Some(toplevel) = window.toplevel() {
            toplevel.send_pending_configure();
        }

        let (visual_x, visual_y) = self
            .motion
            .window(id)
            .map(|motion| motion.position())
            .unwrap_or((f64::from(location.x), f64::from(location.y)));

        self.window_locations.insert(id, location);

        self.transitions.insert(
            id,
            WindowTransition::Minimizing(MinimizedWindowState {
                location,
                size,
                was_floating,
                title,
                app_id,
                snap_target,
            }),
        );

        // Keep the surface mapped while it shrinks and fades.
        self.motion
            .animate_window(id, visual_x, visual_y, 0.82, 0.0);

        self.relayout_windows();

        Ok(())
    }

    /// Restore a minimized window with a spring-driven entrance animation.
    pub fn restore_window(&mut self, id: u64) -> Result<(), String> {
        if self.transitions.contains_key(&id) {
            return Err(format!("Window {id} is already transitioning"));
        }

        let saved = self
            .minimized_windows
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("Window {id} is not minimized"))?;
        let window = self
            .windows
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("Unknown window ID: {id}"))?;

        if !window.alive() {
            return Err(format!("Window {id} is no longer alive"));
        }
        if self.space.elements().any(|mapped| mapped == &window) {
            return Err(format!("Window {id} is already mapped"));
        }

        let toplevel = window
            .toplevel()
            .filter(|toplevel| toplevel.alive())
            .ok_or_else(|| format!("Window {id} has no live toplevel surface"))?;
        let workspace = self
            .window_workspaces
            .get(&id)
            .copied()
            .ok_or_else(|| format!("Window {id} has no workspace assignment"))?;

        self.switch_workspace(workspace)?;
        self.space.map_element(window.clone(), saved.location, true);
        self.window_locations.insert(id, saved.location);

        self.motion.snap_window_position(
            id,
            f64::from(saved.location.x),
            f64::from(saved.location.y),
        );
        self.motion.snap_window_scale(id, 0.82);
        self.motion.set_window_opacity(id, 0.0);

        if saved.was_floating && saved.snap_target.is_none() {
            self.floating_windows.insert(id);
        } else {
            self.floating_windows.remove(&id);
        }

        if saved.was_floating || saved.snap_target.is_some() {
            toplevel.with_pending_state(|state| {
                state.size = Some(saved.size);
                if saved.snap_target == Some(SnapTarget::Maximize) {
                    state.states.set(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                    );
                } else {
                    state.states.unset(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                    );
                }
            });
            toplevel.send_pending_configure();
        }

        if let Some(target) = saved.snap_target {
            self.snapped_windows.insert(id, target);
        }

        self.layout_sizes.remove(&id);
        self.minimized_windows.remove(&id);
        self.transitions
            .insert(id, WindowTransition::Restoring(saved.clone()));

        self.motion.animate_window(
            id,
            f64::from(saved.location.x),
            f64::from(saved.location.y),
            1.0,
            1.0,
        );

        if saved.was_floating {
            self.raise_window_with_policy(&window);
        } else {
            self.relayout_windows();
        }

        Ok(())
    }

    /// Build a deterministic overview snapshot for every workspace.
    ///
    /// Live windows are read from their XDG toplevel metadata. Minimized
    /// windows use the metadata captured before they were unmapped.
    pub fn overview_data(&self) -> (Vec<OverviewWorkspaceInfo>, u32) {
        let mut workspaces = Vec::with_capacity(self.workspace_count as usize);

        for index in 0..self.workspace_count {
            let mut ids: Vec<u64> = self
                .window_workspaces
                .iter()
                .filter_map(|(id, workspace)| (*workspace == index).then_some(*id))
                .collect();
            ids.sort_unstable();

            let mut windows = Vec::with_capacity(ids.len());

            for id in ids {
                if let Some(minimized) = self.minimized_windows.get(&id) {
                    windows.push(OverviewWindowInfo {
                        id,
                        title: minimized.title.clone(),
                        app_id: minimized.app_id.clone(),
                        minimized: true,
                    });
                    continue;
                }

                let Some(window) = self.windows.get(&id).filter(|window| window.alive()) else {
                    continue;
                };

                let Some(surface) = window
                    .toplevel()
                    .filter(|toplevel| toplevel.alive())
                    .map(|toplevel| toplevel.wl_surface().clone())
                else {
                    continue;
                };

                let (title, app_id) = with_states(&surface, |states| {
                    states.data_map.get::<XdgToplevelSurfaceData>().map(|data| {
                        let role = data.lock().unwrap();
                        (role.title.clone(), role.app_id.clone())
                    })
                })
                .unwrap_or((None, None));

                windows.push(OverviewWindowInfo {
                    id,
                    title,
                    app_id,
                    minimized: false,
                });
            }

            workspaces.push(OverviewWorkspaceInfo { index, windows });
        }

        (workspaces, self.active_workspace)
    }

    /// Return minimized windows in stable ID order for shell thumbnail rendering.
    pub fn list_minimized_windows(&self) -> Vec<MinimizedWindowInfo> {
        let mut windows: Vec<MinimizedWindowInfo> = self
            .minimized_windows
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
        let removed_ids: Vec<u64> = self
            .windows
            .iter()
            .filter_map(|(id, window)| {
                window
                    .toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == surface)
                    .then_some(*id)
            })
            .collect();

        for id in &removed_ids {
            for tree in self.workspace_layouts.values_mut() {
                tree.remove(*id);
            }
            self.minimized_windows.remove(id);
            self.transitions.remove(id);
            self.floating_windows.remove(id);
            self.unplaced_windows.remove(id);
            self.snapped_windows.remove(id);
            self.maximize_restore_rects.remove(id);
            self.layout_sizes.remove(id);
            self.window_workspaces.remove(id);
            self.window_locations.remove(id);
            self.motion.remove_window(*id);
        }

        self.windows.retain(|_, window| {
            !window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == surface)
        });

        let window_to_unmap = self
            .space
            .elements()
            .find(|window| {
                window
                    .toplevel()
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
            Action::GetLayoutMode => response(
                RequestStatus::Completed,
                ActionResult::LayoutMode {
                    mode: self.layout_mode,
                },
            ),

            Action::SetLayoutMode { mode } => {
                let entering_user_managed =
                    matches!(mode, LayoutMode::Manual | LayoutMode::Fractional)
                        && !matches!(
                            self.layout_mode,
                            LayoutMode::Manual | LayoutMode::Fractional
                        );

                if entering_user_managed {
                    let pending_ids: Vec<u64> = self
                        .windows
                        .keys()
                        .copied()
                        .filter(|id| {
                            if self.floating_windows.contains(id) {
                                return false;
                            }

                            let already_placed = self
                                .window_workspaces
                                .get(id)
                                .and_then(|workspace| self.workspace_layouts.get(workspace))
                                .is_some_and(|tree| tree.contains(*id));

                            !already_placed
                        })
                        .collect();

                    self.unplaced_windows.extend(pending_ids);
                }

                // Automatic takes responsibility for inserting pending windows.
                if mode == LayoutMode::Adaptive {
                    self.unplaced_windows.clear();
                }

                self.layout_mode = mode;
                self.relayout_windows();
                response(RequestStatus::Completed, ActionResult::LayoutMode { mode })
            }

            Action::InsertWindowInLayout {
                id,
                target_id,
                axis,
                ratio,
            } => {
                if !matches!(
                    self.layout_mode,
                    LayoutMode::Manual | LayoutMode::Fractional
                ) {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message:
                                "Explicit insertion is only available in Manual or Fractional mode"
                                    .to_owned(),
                        },
                    );
                }

                let is_valid_pending = self.windows.get(&id).is_some_and(|window| {
                    window.alive()
                        && self.space.elements().any(|mapped| mapped == window)
                        && self.window_workspaces.get(&id) == Some(&self.active_workspace)
                        && self.unplaced_windows.contains(&id)
                        && !self.floating_windows.contains(&id)
                        && !self.snapped_windows.contains_key(&id)
                });

                if !is_valid_pending {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: format!("Window {id} is not an eligible pending window in the active workspace"),
                        },
                    );
                }

                let target_valid = target_id == 0
                    || (self.windows.get(&target_id).is_some_and(|window| {
                        window.alive()
                            && self.space.elements().any(|mapped| mapped == window)
                            && self.window_workspaces.get(&target_id)
                                == Some(&self.active_workspace)
                            && !self.floating_windows.contains(&target_id)
                            && !self.unplaced_windows.contains(&target_id)
                            && !self.snapped_windows.contains_key(&target_id)
                    }));

                if !target_valid {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: format!(
                                "Target window {target_id} is not eligible for insertion"
                            ),
                        },
                    );
                }

                let Some((bounds, gap, min_size)) = self.split_edit_bounds() else {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: "No usable layout bounds are available".to_owned(),
                        },
                    );
                };

                let result = {
                    let tree = self
                        .workspace_layouts
                        .entry(self.active_workspace)
                        .or_default();

                    if tree.is_empty() {
                        if target_id != 0 {
                            Err(crate::layout_engine::LayoutError::WindowNotFound(target_id))
                        } else {
                            tree.insert(id, None, bounds, gap, min_size)
                        }
                    } else if target_id == 0 {
                        Err(crate::layout_engine::LayoutError::WindowNotFound(0))
                    } else {
                        tree.insert_with_ratio(id, target_id, axis, ratio, bounds, gap, min_size)
                    }
                };

                match result {
                    Ok(()) => {
                        self.unplaced_windows.remove(&id);
                        self.relayout_windows();
                        response(RequestStatus::Completed, ActionResult::Ok)
                    }
                    Err(error) => response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: format!("Unable to insert window into split tree: {error:?}"),
                        },
                    ),
                }
            }

            Action::SetSplitRatio { window_id, ratio } => {
                if self.layout_mode != LayoutMode::Fractional {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: "Split ratios are editable only in Fractional mode".to_owned(),
                        },
                    );
                }

                let eligible = self.windows.get(&window_id).is_some_and(|window| {
                    window.alive()
                        && self.space.elements().any(|mapped| mapped == window)
                        && self.window_workspaces.get(&window_id) == Some(&self.active_workspace)
                        && !self.floating_windows.contains(&window_id)
                        && !self.unplaced_windows.contains(&window_id)
                        && !self.snapped_windows.contains_key(&window_id)
                });

                if !eligible {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: format!(
                                "Window {window_id} is not eligible for ratio editing"
                            ),
                        },
                    );
                }

                let Some((bounds, gap, min_size)) = self.split_edit_bounds() else {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: "No usable layout bounds are available".to_owned(),
                        },
                    );
                };

                let result = self
                    .workspace_layouts
                    .get_mut(&self.active_workspace)
                    .ok_or(crate::layout_engine::LayoutError::WindowNotFound(window_id))
                    .and_then(|tree| tree.set_split_ratio(window_id, ratio, bounds, gap, min_size));

                match result {
                    Ok(()) => {
                        self.relayout_windows();
                        response(RequestStatus::Completed, ActionResult::Ok)
                    }
                    Err(error) => response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: format!("Unable to change split ratio: {error:?}"),
                        },
                    ),
                }
            }

            Action::SetSplitAxis { window_id, axis } => {
                if self.layout_mode != LayoutMode::Manual {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: "Split orientation is editable only in Manual mode".to_owned(),
                        },
                    );
                }

                let eligible = self.windows.get(&window_id).is_some_and(|window| {
                    window.alive()
                        && self.space.elements().any(|mapped| mapped == window)
                        && self.window_workspaces.get(&window_id) == Some(&self.active_workspace)
                        && !self.floating_windows.contains(&window_id)
                        && !self.unplaced_windows.contains(&window_id)
                        && !self.snapped_windows.contains_key(&window_id)
                });

                if !eligible {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: format!(
                                "Window {window_id} is not eligible for split editing"
                            ),
                        },
                    );
                }

                let Some((bounds, gap, min_size)) = self.split_edit_bounds() else {
                    return response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: "No usable layout bounds are available".to_owned(),
                        },
                    );
                };

                let result = self
                    .workspace_layouts
                    .get_mut(&self.active_workspace)
                    .ok_or(crate::layout_engine::LayoutError::WindowNotFound(window_id))
                    .and_then(|tree| tree.set_split_axis(window_id, axis, bounds, gap, min_size));

                match result {
                    Ok(()) => {
                        self.relayout_windows();
                        response(RequestStatus::Completed, ActionResult::Ok)
                    }
                    Err(error) => response(
                        RequestStatus::Rejected,
                        ActionResult::Err {
                            message: format!("Unable to change split orientation: {error:?}"),
                        },
                    ),
                }
            }

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

            Action::ListWorkspaces => {
                let (count, active_index) = self.workspace_state();
                response(
                    RequestStatus::Completed,
                    ActionResult::Workspaces {
                        count,
                        active_index,
                    },
                )
            }

            Action::GetOverviewData => {
                let (workspaces, active_index) = self.overview_data();
                response(
                    RequestStatus::Completed,
                    ActionResult::OverviewData {
                        workspaces,
                        active_index,
                    },
                )
            }

            Action::CreateWorkspace => match self.create_workspace() {
                Ok(()) => {
                    let (count, active_index) = self.workspace_state();
                    response(
                        RequestStatus::Completed,
                        ActionResult::Workspaces {
                            count,
                            active_index,
                        },
                    )
                }
                Err(message) => response(RequestStatus::Rejected, ActionResult::Err { message }),
            },

            Action::SwitchWorkspace { index } => match self.switch_workspace(index) {
                Ok(()) => {
                    let (count, active_index) = self.workspace_state();
                    response(
                        RequestStatus::Completed,
                        ActionResult::Workspaces {
                            count,
                            active_index,
                        },
                    )
                }
                Err(message) => response(RequestStatus::Rejected, ActionResult::Err { message }),
            },

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
        let output = self.space.outputs().find(|output| {
            self.space
                .output_geometry(output)
                .is_some_and(|geometry| geometry.contains(pos.to_i32_round()))
        })?;
        let output_geometry = self.space.output_geometry(output)?;
        let output_pos = pos - output_geometry.loc.to_f64();
        let layers = layer_map_for_output(output);

        // Foreground layer-shell surfaces receive input before application windows.
        if let Some(layer) = layers
            .layer_under(Layer::Overlay, output_pos)
            .or_else(|| layers.layer_under(Layer::Top, output_pos))
        {
            if let Some(layer_geometry) = layers.layer_geometry(layer) {
                if let Some((surface, location)) = layer.surface_under(
                    output_pos - layer_geometry.loc.to_f64(),
                    WindowSurfaceType::ALL,
                ) {
                    return Some((
                        surface,
                        (location + layer_geometry.loc + output_geometry.loc).to_f64(),
                    ));
                }
            }
        }

        // Test application windows in Smithay's front-to-back stacking order.
        for window in self.space.elements().rev() {
            let Some(location) = self.space.element_location(window) else {
                continue;
            };
            let Some(bbox) = self.space.element_bbox(window) else {
                continue;
            };

            let id = self
                .windows
                .iter()
                .find_map(|(&id, candidate)| (candidate == window).then_some(id));

            let (visual_x, visual_y) = id
                .and_then(|id| self.motion.window(id))
                .map(|motion| motion.position())
                .unwrap_or((f64::from(location.x), f64::from(location.y)));

            let delta_x = visual_x - f64::from(location.x);
            let delta_y = visual_y - f64::from(location.y);

            let mut visual_bbox = bbox.to_f64();
            visual_bbox.loc.x += delta_x;
            visual_bbox.loc.y += delta_y;

            if !visual_bbox.contains(pos) {
                continue;
            }

            let render_location = location - window.geometry().loc;
            let visual_location = Point::<f64, Logical>::from((
                f64::from(render_location.x) + delta_x,
                f64::from(render_location.y) + delta_y,
            ));

            if let Some((surface, surface_location)) =
                window.surface_under(pos - visual_location, WindowSurfaceType::ALL)
            {
                return Some((surface, surface_location.to_f64() + visual_location));
            }
        }

        // Background layer-shell surfaces are checked after application windows.
        if let Some(layer) = layers
            .layer_under(Layer::Bottom, output_pos)
            .or_else(|| layers.layer_under(Layer::Background, output_pos))
        {
            if let Some(layer_geometry) = layers.layer_geometry(layer) {
                if let Some((surface, location)) = layer.surface_under(
                    output_pos - layer_geometry.loc.to_f64(),
                    WindowSurfaceType::ALL,
                ) {
                    return Some((
                        surface,
                        (location + layer_geometry.loc + output_geometry.loc).to_f64(),
                    ));
                }
            }
        }

        None
    }

    /// Advance window animations and complete lifecycle transitions.
    pub(crate) fn advance_motion(&mut self) -> bool {
        let now = std::time::Instant::now();
        let delta = now.duration_since(self.motion_last_tick).as_secs_f64();
        self.motion_last_tick = now;
        let mut active = self.motion.advance(delta);

        let completed: Vec<u64> = self
            .transitions
            .keys()
            .copied()
            .filter(|id| {
                self.motion
                    .window(*id)
                    .map_or(true, |motion| motion.is_settled())
            })
            .collect();

        let mut layout_changed = false;
        let mut focus_after = Vec::new();

        for id in completed {
            let Some(transition) = self.transitions.remove(&id) else {
                continue;
            };

            match transition {
                WindowTransition::Minimizing(saved) => {
                    let Some(window) = self.windows.get(&id).cloned() else {
                        continue;
                    };
                    if !window.alive() {
                        continue;
                    }
                    if self.space.elements().any(|mapped| mapped == &window) {
                        self.space.unmap_elem(&window);
                    }
                    self.layout_sizes.remove(&id);
                    self.minimized_windows.insert(id, saved);
                    layout_changed = true;
                }
                WindowTransition::Restoring(saved) => {
                    let Some(window) = self.windows.get(&id).cloned() else {
                        continue;
                    };
                    if !window.alive() {
                        continue;
                    }

                    let is_mapped = self.space.elements().any(|mapped| mapped == &window);
                    let is_active_workspace =
                        self.window_workspaces.get(&id) == Some(&self.active_workspace);

                    if !is_mapped || !is_active_workspace {
                        if is_mapped {
                            self.space.unmap_elem(&window);
                        }
                        self.snapped_windows.remove(&id);
                        self.minimized_windows.insert(id, saved);
                        continue;
                    }

                    if !saved.was_floating {
                        layout_changed = true;
                    }
                    focus_after.push(id);
                }
            }
        }

        if layout_changed {
            self.relayout_windows();
            active = true;
        }

        for id in focus_after {
            let _ = self.focus_window(id);
        }

        active || !self.transitions.is_empty()
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
