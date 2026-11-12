//! Vanta shell — top panel and bottom dock via wlr-layer-shell.

mod control_worker;
mod ipc;
mod render;

use std::{
    num::NonZeroU32,
    time::{Duration, Instant},
};

use smithay_client_toolkit::reexports::{
    calloop::{channel, EventLoop},
    calloop_wayland_source::WaylandSource,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use vanta_ipc::{Action, ActionResult, MinimizedWindowInfo};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

const BAR_H: u32 = render::theme::PANEL_HEIGHT;
const DOCK_SURFACE_HEIGHT: u32 = render::theme::DOCK_HEIGHT + 16;
const INITIAL_WIDTH: u32 = 1280;
const QUICK_SETTINGS_WIDTH: u32 = 376;
const QUICK_SETTINGS_HEIGHT: u32 = 520;

fn main() {
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY is not set");
    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let mut event_loop: EventLoop<VantaShell> =
        EventLoop::try_new().expect("Failed to initialize the shell event loop");

    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .expect("Failed to register the Wayland event source");

    let (ipc_sender, ipc_receiver) = channel::channel();

    event_loop
        .handle()
        .insert_source(ipc_receiver, |event, _, shell| match event {
            channel::Event::Msg(event) => shell.handle_ipc_event(event),
            channel::Event::Closed => {
                eprintln!("vanta-shell: IPC response channel closed");
            }
        })
        .expect("Failed to register the IPC response channel");

    let (control_sender, control_receiver) = channel::channel();

    event_loop
        .handle()
        .insert_source(control_receiver, |event, _, shell| match event {
            channel::Event::Msg(event) => shell.handle_control_event(event),
            channel::Event::Closed => {
                eprintln!("vanta-shell: Controls worker channel closed");
            }
        })
        .expect("Failed to register the Controls worker channel");

    let ipc_client = ipc::IpcClient::new(ipc_sender).expect("Failed to start the IPC worker");
    let control_worker = control_worker::ControlWorker::new(control_sender)
        .expect("Failed to start the Controls worker");
    let font_renderer = render::font::FontRenderer::load().expect("Failed to load Vanta's UI font");

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("zwlr_layer_shell_v1");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");

    // Top panel: reserves its height from the workspace.
    let panel_surface = compositor.create_surface(&qh);
    let panel_layer =
        layer_shell.create_layer_surface(&qh, panel_surface, Layer::Top, Some("vanta-panel"), None);
    panel_layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    panel_layer.set_exclusive_zone(0);
    panel_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    panel_layer.set_size(0, BAR_H);
    panel_layer.commit();

    // Dock: overlays normal windows without reserving workspace space.
    let dock_surface = compositor.create_surface(&qh);
    let dock_layer =
        layer_shell.create_layer_surface(&qh, dock_surface, Layer::Top, Some("vanta-dock"), None);
    dock_layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    dock_layer.set_exclusive_zone(0);
    dock_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    dock_layer.set_size(0, DOCK_SURFACE_HEIGHT);
    dock_layer.commit();

    let panel_pool = SlotPool::new(INITIAL_WIDTH as usize * BAR_H as usize * 4, &shm).unwrap();
    let dock_pool = SlotPool::new(
        INITIAL_WIDTH as usize * DOCK_SURFACE_HEIGHT as usize * 4,
        &shm,
    )
    .unwrap();

    let mut shell = VantaShell {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        pointer: None,
        compositor: compositor.clone(),
        layer_shell,
        shm,
        font_renderer,
        panel_pool,
        dock_pool,
        panel_layer: Some(panel_layer),
        dock_layer: Some(dock_layer),
        quick_settings_layer: None,
        quick_settings_pool: None,
        quick_settings_width: QUICK_SETTINGS_WIDTH,
        quick_settings_height: QUICK_SETTINGS_HEIGHT,
        quick_settings_first_configure: true,
        control_state: render::quick_settings::ControlState::default(),
        volume_slider_dragging: false,
        last_volume_slider_percent: None,
        brightness_slider_dragging: false,
        last_brightness_slider_percent: None,
        controls_hovered: false,
        panel_revealed: true,
        panel_width: INITIAL_WIDTH,
        panel_height: BAR_H,
        dock_width: INITIAL_WIDTH,
        dock_height: DOCK_SURFACE_HEIGHT,
        panel_first_configure: true,
        dock_first_configure: true,
        exit: false,
        workspace_count: 1,
        active_workspace: 0,
        minimized_windows: Vec::new(),
        control_worker,
        ipc_client,
    };

    if let Err(error) = shell.ipc_client.request(Action::ListMinimizedWindows) {
        eprintln!("vanta-shell: cannot request minimized windows: {error}");
    }

    if let Err(error) = shell.ipc_client.request(Action::ListWorkspaces) {
        eprintln!("vanta-shell: cannot request workspace state: {error}");
    }

    let mut last_workspace_sync = Instant::now();

    while !shell.exit {
        event_loop
            .dispatch(Duration::from_millis(16), &mut shell)
            .unwrap();

        if last_workspace_sync.elapsed() >= Duration::from_millis(500) {
            if let Err(error) = shell.ipc_client.request(Action::ListWorkspaces) {
                eprintln!("vanta-shell: workspace sync request failed: {error}");
            }
            last_workspace_sync = Instant::now();
        }
    }
}

struct VantaShell {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    pointer: Option<wl_pointer::WlPointer>,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    font_renderer: render::font::FontRenderer,

    panel_pool: SlotPool,
    dock_pool: SlotPool,

    panel_layer: Option<LayerSurface>,
    dock_layer: Option<LayerSurface>,
    quick_settings_layer: Option<LayerSurface>,
    quick_settings_pool: Option<SlotPool>,
    quick_settings_width: u32,
    quick_settings_height: u32,
    quick_settings_first_configure: bool,
    control_state: render::quick_settings::ControlState,
    control_worker: control_worker::ControlWorker,
    volume_slider_dragging: bool,
    last_volume_slider_percent: Option<u32>,
    brightness_slider_dragging: bool,
    last_brightness_slider_percent: Option<u32>,
    controls_hovered: bool,
    panel_revealed: bool,

    panel_width: u32,
    panel_height: u32,
    dock_width: u32,
    dock_height: u32,

    panel_first_configure: bool,
    dock_first_configure: bool,

    exit: bool,
    workspace_count: u32,
    active_workspace: u32,
    minimized_windows: Vec<MinimizedWindowInfo>,
    ipc_client: ipc::IpcClient,
}

impl VantaShell {
    fn update_volume_slider(&mut self, x: f64) {
        let percent = (((x - 112.0) / 176.0) * 100.0).round().clamp(0.0, 100.0) as u32;

        if self.last_volume_slider_percent == Some(percent) {
            return;
        }

        self.last_volume_slider_percent = Some(percent);

        let action = render::quick_settings::ControlAction::SetVolumePercent(percent);

        self.control_state.apply_optimistic(action);
        if self.quick_settings_layer.is_some() {
            self.draw_quick_settings();
        }

        if let Err(error) = self.control_worker.perform(action) {
            eprintln!("vanta-shell: cannot queue volume update: {error}");
        }
    }

    fn update_brightness_slider(&mut self, x: f64) {
        let percent = (((x - 28.0) / 320.0) * 100.0).round().clamp(0.0, 100.0) as u32;

        if self.last_brightness_slider_percent == Some(percent) {
            return;
        }

        self.last_brightness_slider_percent = Some(percent);
        self.control_state.brightness_percent = Some(percent);

        if self.quick_settings_layer.is_some() {
            self.draw_quick_settings();
        }
    }

    fn handle_control_event(&mut self, event: control_worker::ControlEvent) {
        match event {
            control_worker::ControlEvent::State(state) => {
                if self.control_state != state {
                    self.control_state = state;
                    if self.quick_settings_layer.is_some() {
                        self.draw_quick_settings();
                    }
                }
            }
        }
    }

    fn handle_ipc_event(&mut self, event: ipc::IpcEvent) {
        match event.result {
            Ok(response) => match response.result {
                Some(ActionResult::Workspaces {
                    count,
                    active_index,
                }) => {
                    if count > 0 && active_index < count {
                        let changed =
                            self.workspace_count != count || self.active_workspace != active_index;

                        self.workspace_count = count;
                        self.active_workspace = active_index;

                        if changed {
                            if !self.panel_first_configure {
                                self.draw_panel();
                            }

                            if !self.dock_first_configure {
                                self.draw_dock();
                            }

                            if self.quick_settings_layer.is_some() {
                                self.draw_quick_settings();
                            }
                        }
                    }
                }

                Some(ActionResult::MinimizedWindows { windows }) => {
                    self.minimized_windows = windows;

                    if !self.dock_first_configure {
                        self.draw_dock();
                    }
                }

                Some(other) => {
                    eprintln!(
                        "vanta-shell: IPC request {} returned {:?}",
                        event.request_id, other
                    );
                }

                None => {}
            },

            Err(error) => {
                eprintln!(
                    "vanta-shell: IPC request {} failed: {}",
                    event.request_id, error
                );
            }
        }
    }

    fn draw_panel(&mut self) {
        let Some(layer) = self.panel_layer.as_ref() else {
            return;
        };

        let stride = self.panel_width as i32 * 4;
        let (buffer, canvas) = self
            .panel_pool
            .create_buffer(
                self.panel_width as i32,
                self.panel_height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("panel shm buffer");

        {
            let mut frame =
                render::primitives::Canvas::new(canvas, self.panel_width, self.panel_height);

            frame.clear(render::theme::COLOR_PANEL);
            render::panel::draw(
                &mut frame,
                &self.font_renderer,
                self.panel_width,
                self.panel_height,
                self.workspace_count,
                self.active_workspace,
                self.controls_hovered,
            );
        }

        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, self.panel_width as i32, self.panel_height as i32);
        buffer.attach_to(surface).unwrap();
        layer.commit();
    }

    fn draw_dock(&mut self) {
        let Some(layer) = self.dock_layer.as_ref() else {
            return;
        };

        let stride = self.dock_width as i32 * 4;
        let (buffer, canvas) = self
            .dock_pool
            .create_buffer(
                self.dock_width as i32,
                self.dock_height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("dock shm buffer");

        {
            let mut frame =
                render::primitives::Canvas::new(canvas, self.dock_width, self.dock_height);

            frame.clear(0x00000000);
            render::dock::draw(
                &mut frame,
                &self.font_renderer,
                self.dock_width,
                self.dock_height,
                self.workspace_count,
                self.active_workspace,
                &self.minimized_windows,
            );
        }

        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, self.dock_width as i32, self.dock_height as i32);
        buffer.attach_to(surface).unwrap();
        layer.commit();
    }

    fn open_quick_settings(&mut self, qh: &QueueHandle<Self>) {
        if self.quick_settings_layer.is_some() {
            return;
        }

        if let Err(error) = self.control_worker.set_visible(true) {
            eprintln!("vanta-shell: cannot refresh Controls: {error}");
        }

        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("vanta-quick-settings"),
            None,
        );

        layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
        layer.set_margin(BAR_H as i32 + 8, 12, 0, 0);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        layer.set_size(self.quick_settings_width, self.quick_settings_height);
        layer.commit();

        let pool_size =
            self.quick_settings_width as usize * self.quick_settings_height as usize * 4;

        self.quick_settings_pool =
            Some(SlotPool::new(pool_size, &self.shm).expect("quick settings shm pool"));
        self.quick_settings_first_configure = true;
        self.quick_settings_layer = Some(layer);
    }

    fn close_quick_settings(&mut self) {
        if let Err(error) = self.control_worker.set_visible(false) {
            eprintln!("vanta-shell: cannot pause Controls polling: {error}");
        }

        self.quick_settings_layer = None;
        self.quick_settings_pool = None;
        self.quick_settings_first_configure = true;
    }

    fn draw_quick_settings(&mut self) {
        eprintln!(
            "VANTA_QS: draw requested; size={}x{}; layer_present={}",
            self.quick_settings_width,
            self.quick_settings_height,
            self.quick_settings_layer.is_some()
        );

        let Some(layer) = self.quick_settings_layer.as_ref() else {
            return;
        };
        let Some(pool) = self.quick_settings_pool.as_mut() else {
            return;
        };

        let width = self.quick_settings_width;
        let height = self.quick_settings_height;
        if width == 0 || height == 0 {
            return;
        }

        let stride = width as i32 * 4;
        let (buffer, canvas) = pool
            .create_buffer(
                width as i32,
                height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("quick settings shm buffer");

        {
            let mut frame = render::primitives::Canvas::new(canvas, width, height);
            frame.clear(0x00000000);
            render::quick_settings::draw(
                &mut frame,
                &self.font_renderer,
                width,
                height,
                self.workspace_count,
                self.active_workspace,
                &self.control_state,
            );
        }

        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        buffer.attach_to(surface).unwrap();
        layer.commit();
        eprintln!("VANTA_QS: buffer attached and layer committed");
    }
}

impl CompositorHandler for VantaShell {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self
            .panel_layer
            .as_ref()
            .is_some_and(|layer| surface == layer.wl_surface())
        {
            self.draw_panel();
        } else if self
            .dock_layer
            .as_ref()
            .is_some_and(|layer| surface == layer.wl_surface())
        {
            self.draw_dock();
        } else if self
            .quick_settings_layer
            .as_ref()
            .is_some_and(|layer| surface == layer.wl_surface())
        {
            self.draw_quick_settings();
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for VantaShell {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for VantaShell {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        eprintln!(
            "VANTA_INPUT: new seat detected: {:?}",
            self.seat_state.info(&seat)
        );
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        eprintln!(
            "VANTA_INPUT: capability added: {:?}, seat={:?}",
            capability,
            self.seat_state.info(&seat)
        );

        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => {
                    eprintln!("VANTA_INPUT: pointer successfully registered");
                    self.pointer = Some(pointer);
                }
                Err(error) => eprintln!("VANTA_INPUT: pointer registration failed: {error}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        eprintln!("VANTA_INPUT: capability removed: {:?}", capability);

        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
                eprintln!("VANTA_INPUT: pointer released");
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for VantaShell {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            // Track Controls hover before filtering for click events.
            let is_panel_surface = self
                .panel_layer
                .as_ref()
                .is_some_and(|layer| &event.surface == layer.wl_surface());

            if is_panel_surface {
                match &event.kind {
                    PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. }
                        if !self.panel_revealed =>
                    {
                        self.panel_revealed = true;
                        if let Some(layer) = self.panel_layer.as_ref() {
                            layer.set_size(0, BAR_H);
                            layer.commit();
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        self.controls_hovered = false;
                        self.draw_panel();
                    }
                    _ => {}
                }

                let hovered = match &event.kind {
                    PointerEventKind::Leave { .. } => false,
                    PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                        render::panel::quick_settings_hit_test(
                            self.panel_width,
                            self.panel_height,
                            event.position.0,
                            event.position.1,
                        )
                    }
                    _ => self.controls_hovered,
                };

                if hovered != self.controls_hovered {
                    self.controls_hovered = hovered;
                    self.draw_panel();
                }
            }

            let is_dock_surface = self
                .dock_layer
                .as_ref()
                .is_some_and(|layer| &event.surface == layer.wl_surface());

            if is_dock_surface && matches!(&event.kind, PointerEventKind::Enter { .. }) {
                if let Err(error) = self.ipc_client.request(Action::ListMinimizedWindows) {
                    eprintln!("vanta-shell: minimized-window refresh failed: {error}");
                }
            }

            let is_quick_settings_surface = self
                .quick_settings_layer
                .as_ref()
                .is_some_and(|layer| &event.surface == layer.wl_surface());

            let (x, y) = event.position;

            if is_quick_settings_surface {
                match &event.kind {
                    PointerEventKind::Press {
                        button: BTN_LEFT, ..
                    } => {
                        match render::quick_settings::hit_test(
                            self.quick_settings_width,
                            self.quick_settings_height,
                            x,
                            y,
                            self.workspace_count,
                        ) {
                            Some(render::quick_settings::ControlAction::SetVolumePercent(_)) => {
                                self.volume_slider_dragging = true;
                                self.last_volume_slider_percent = None;
                                self.update_volume_slider(x);
                            }
                            Some(render::quick_settings::ControlAction::SetBrightnessPercent(
                                _,
                            )) => {
                                self.brightness_slider_dragging = true;
                                self.last_brightness_slider_percent = None;
                                self.update_brightness_slider(x);
                            }
                            _ => {}
                        }
                    }
                    PointerEventKind::Motion { .. } if self.volume_slider_dragging => {
                        self.update_volume_slider(x);
                    }
                    PointerEventKind::Motion { .. } if self.brightness_slider_dragging => {
                        self.update_brightness_slider(x);
                    }
                    _ => {}
                }
            }

            if matches!(
                &event.kind,
                PointerEventKind::Release {
                    button: BTN_LEFT,
                    ..
                }
            ) {
                if self.brightness_slider_dragging {
                    if let Some(percent) = self.last_brightness_slider_percent {
                        let action =
                            render::quick_settings::ControlAction::SetBrightnessPercent(percent);
                        if let Err(error) = self.control_worker.perform(action) {
                            eprintln!("vanta-shell: cannot queue brightness update: {error}");
                        }
                    }
                }

                self.volume_slider_dragging = false;
                self.last_volume_slider_percent = None;
                self.brightness_slider_dragging = false;
                self.last_brightness_slider_percent = None;
            }

            let PointerEventKind::Release {
                button: BTN_LEFT, ..
            } = event.kind
            else {
                continue;
            };

            // Quick Settings controls.
            if self
                .quick_settings_layer
                .as_ref()
                .is_some_and(|layer| &event.surface == layer.wl_surface())
            {
                match render::quick_settings::hit_test(
                    self.quick_settings_width,
                    self.quick_settings_height,
                    x,
                    y,
                    self.workspace_count,
                ) {
                    Some(render::quick_settings::ControlAction::Close) => {
                        self.close_quick_settings();
                    }
                    Some(render::quick_settings::ControlAction::SwitchWorkspace(index)) => {
                        if index != self.active_workspace {
                            if let Err(error) =
                                self.ipc_client.request(Action::SwitchWorkspace { index })
                            {
                                eprintln!("vanta-shell: workspace switch request failed: {error}");
                            }
                        }
                    }
                    Some(render::quick_settings::ControlAction::CreateWorkspace) => {
                        if let Err(error) = self.ipc_client.request(Action::CreateWorkspace) {
                            eprintln!("vanta-shell: workspace creation request failed: {error}");
                        }
                    }
                    Some(render::quick_settings::ControlAction::SetVolumePercent(_))
                    | Some(render::quick_settings::ControlAction::SetBrightnessPercent(_)) => {}
                    Some(
                        action @ (render::quick_settings::ControlAction::ToggleWifi
                        | render::quick_settings::ControlAction::ToggleBluetooth
                        | render::quick_settings::ControlAction::ToggleMute),
                    ) => {
                        // Update the visible control in this event cycle.
                        self.control_state.apply_optimistic(action);
                        self.draw_quick_settings();

                        if let Err(error) = self.control_worker.perform(action) {
                            eprintln!("vanta-shell: cannot queue Controls action: {error}");
                        }
                    }
                    None => {}
                }
                continue;
            }

            // Panel Controls button.
            if self
                .panel_layer
                .as_ref()
                .is_some_and(|layer| &event.surface == layer.wl_surface())
            {
                if render::panel::quick_settings_hit_test(self.panel_width, self.panel_height, x, y)
                {
                    if self.quick_settings_layer.is_some() {
                        self.close_quick_settings();
                    } else {
                        self.open_quick_settings(qh);
                    }
                }
                continue;
            }

            // Dock and minimized-window cards share one full-width bottom surface.
            let Some(layer) = self.dock_layer.as_ref() else {
                continue;
            };

            if &event.surface != layer.wl_surface() {
                continue;
            }

            // Only process clicks below this point.
            if let Some(id) = render::dock::minimized_window_at(
                self.dock_width,
                self.dock_height,
                &self.minimized_windows,
                x,
                y,
            ) {
                if let Err(error) = self.ipc_client.request(Action::RestoreWindow { id }) {
                    eprintln!("vanta-shell: restore request failed: {error}");
                } else if let Err(error) = self.ipc_client.request(Action::ListMinimizedWindows) {
                    eprintln!("vanta-shell: minimized-window refresh failed: {error}");
                }

                continue;
            }

            if let Some(index) = render::dock::workspace_at(
                self.dock_width,
                self.dock_height,
                self.workspace_count,
                x,
                y,
            ) {
                if index != self.active_workspace {
                    if let Err(error) = self.ipc_client.request(Action::SwitchWorkspace { index }) {
                        eprintln!("vanta-shell: workspace switch request failed: {error}");
                    }
                }
            }
        }
    }
}

impl LayerShellHandler for VantaShell {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self
            .panel_layer
            .as_ref()
            .is_some_and(|current| current == layer)
        {
            self.panel_layer = None;
            self.exit = true;
        } else if self
            .dock_layer
            .as_ref()
            .is_some_and(|current| current == layer)
        {
            self.dock_layer = None;
            eprintln!("vanta-shell: dock surface closed");
        } else if self
            .quick_settings_layer
            .as_ref()
            .is_some_and(|current| current == layer)
        {
            self.close_quick_settings();
            eprintln!("vanta-shell: quick settings surface closed");
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (suggested_width, suggested_height) = configure.new_size;

        eprintln!(
            "VANTA_QS: configure callback received: {}x{}",
            suggested_width, suggested_height
        );

        if self
            .panel_layer
            .as_ref()
            .is_some_and(|current| current == layer)
        {
            let old_size = (self.panel_width, self.panel_height);

            if let Some(width) = NonZeroU32::new(suggested_width) {
                self.panel_width = width.get();
            }
            if let Some(height) = NonZeroU32::new(suggested_height) {
                self.panel_height = height.get();
            }

            let size_changed = old_size != (self.panel_width, self.panel_height);
            let should_draw = self.panel_first_configure || size_changed;

            eprintln!(
                "vanta-shell: panel configure {}x{}{}",
                self.panel_width,
                self.panel_height,
                if size_changed { " (resized)" } else { "" }
            );

            if should_draw {
                self.panel_first_configure = false;
                self.draw_panel();
            }
        } else if self
            .dock_layer
            .as_ref()
            .is_some_and(|current| current == layer)
        {
            let old_size = (self.dock_width, self.dock_height);

            if let Some(width) = NonZeroU32::new(suggested_width) {
                self.dock_width = width.get();
            }
            if let Some(height) = NonZeroU32::new(suggested_height) {
                self.dock_height = height.get();
            }

            let size_changed = old_size != (self.dock_width, self.dock_height);
            let should_draw = self.dock_first_configure || size_changed;

            if size_changed {
                let pool_size = self.dock_width as usize * self.dock_height as usize * 4;

                self.dock_pool =
                    SlotPool::new(pool_size, &self.shm).expect("resized dock shm pool");
            }

            eprintln!(
                "vanta-shell: dock configure {}x{}{}",
                self.dock_width,
                self.dock_height,
                if size_changed { " (resized)" } else { "" }
            );

            if should_draw {
                self.dock_first_configure = false;
                self.draw_dock();
            }
        } else if self
            .quick_settings_layer
            .as_ref()
            .is_some_and(|current| current == layer)
        {
            eprintln!("VANTA_QS: configure matched quick-settings layer");

            let old_size = (self.quick_settings_width, self.quick_settings_height);

            if let Some(width) = NonZeroU32::new(suggested_width) {
                self.quick_settings_width = width.get();
            }
            if let Some(height) = NonZeroU32::new(suggested_height) {
                self.quick_settings_height = height.get();
            }

            let size_changed = old_size != (self.quick_settings_width, self.quick_settings_height);

            if size_changed {
                let pool_size =
                    self.quick_settings_width as usize * self.quick_settings_height as usize * 4;
                self.quick_settings_pool = Some(
                    SlotPool::new(pool_size, &self.shm).expect("resized quick settings shm pool"),
                );
            }

            if self.quick_settings_first_configure || size_changed {
                self.quick_settings_first_configure = false;
                self.draw_quick_settings();
            }
        }
    }
}

impl ShmHandler for VantaShell {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(VantaShell);

impl ProvidesRegistryState for VantaShell {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(VantaShell);
