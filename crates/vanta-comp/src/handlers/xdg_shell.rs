use smithay::{
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, PopupKind, PopupManager, Space, Window,
    },
    input::{
        pointer::{Focus, GrabStartData as PointerGrabStartData},
        Seat,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::{
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    Vanta,
};

impl XdgShellHandler for Vanta {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);

        let Some(id) = self.register_window(window.clone()) else {
            tracing::error!(
                smithay_window_id = window.id(),
                "Vanta window ID space exhausted; refusing to map unregistered window"
            );
            return;
        };

        let location = self
            .output
            .as_ref()
            .and_then(|output| self.space.output_geometry(output))
            .map(|area| {
                let margin = if area.size.w >= 48 && area.size.h >= 48 {
                    12
                } else {
                    0
                };

                let available_width = area.size.w.saturating_sub(margin * 2).max(1);
                let available_height = area.size.h.saturating_sub(margin * 2).max(1);

                let geometry = window.geometry();

                let requested_width = if geometry.size.w >= 64 {
                    geometry.size.w
                } else {
                    960
                };

                let requested_height = if geometry.size.h >= 64 {
                    geometry.size.h
                } else {
                    640
                };

                let width = requested_width.min(available_width);
                let height = requested_height.min(available_height);

                let stagger = (id % 7) as i32 * 28;

                let min_x = area.loc.x.saturating_add(margin);
                let min_y = area.loc.y.saturating_add(margin);

                let max_x = area
                    .loc
                    .x
                    .saturating_add(area.size.w)
                    .saturating_sub(margin)
                    .saturating_sub(width)
                    .max(min_x);

                let max_y = area
                    .loc
                    .y
                    .saturating_add(area.size.h)
                    .saturating_sub(margin)
                    .saturating_sub(height)
                    .max(min_y);

                let x = area
                    .loc
                    .x
                    .saturating_add(area.size.w.saturating_sub(width) / 2)
                    .saturating_add(stagger)
                    .clamp(min_x, max_x);

                let y = area
                    .loc
                    .y
                    .saturating_add(area.size.h.saturating_sub(height) / 2)
                    .saturating_add(stagger)
                    .clamp(min_y, max_y);

                (x, y).into()
            })
            .unwrap_or_else(|| (0, 0).into());

        self.window_locations.insert(id, location);
        self.space.map_element(window, location, false);
        self.relayout_windows();
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);

        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            eprintln!("vanta-comp: failed to track popup: {error}");
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });

        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::from_resource(&seat) else {
            return;
        };

        let wl_surface = surface.wl_surface();

        let Some(start_data) = check_grab(&seat, wl_surface, serial) else {
            return;
        };

        let Some(pointer) = seat.get_pointer() else {
            return;
        };

        let Some(window) = self
            .space
            .elements()
            .find(|window| {
                window
                    .toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == wl_surface)
            })
            .cloned()
        else {
            return;
        };

        let Some(initial_window_location) = self.space.element_location(&window) else {
            return;
        };

        self.make_window_floating(&window);

        let grab = MoveSurfaceGrab {
            start_data,
            window,
            initial_window_location,
            snap_target: None,
            minimize_target: false,
        };

        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let Some(seat) = Seat::from_resource(&seat) else {
            return;
        };

        let wl_surface = surface.wl_surface();

        let Some(start_data) = check_grab(&seat, wl_surface, serial) else {
            return;
        };

        let Some(pointer) = seat.get_pointer() else {
            return;
        };

        let Some(window) = self
            .space
            .elements()
            .find(|window| {
                window
                    .toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == wl_surface)
            })
            .cloned()
        else {
            return;
        };

        let Some(initial_window_location) = self.space.element_location(&window) else {
            return;
        };

        let initial_window_size = window.geometry().size;

        self.make_window_floating(&window);

        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Resizing);
        });

        surface.send_pending_configure();

        let grab = ResizeSurfaceGrab::start(
            start_data,
            window,
            edges.into(),
            Rectangle::new(initial_window_location, initial_window_size),
        );

        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO: implement popup grabs.
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.unregister_window_for_surface(surface.wl_surface());
    }
}

fn check_grab(
    seat: &Seat<Vanta>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<Vanta>> {
    let pointer = seat.get_pointer()?;

    if !pointer.has_grab(serial) {
        return None;
    }

    let start_data = pointer.grab_start_data()?;
    let (focus, _) = start_data.focus.as_ref()?;

    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }

    Some(start_data)
}

/// Should be called on WlSurface::commit.
pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface) {
    if let Some(window) = space
        .elements()
        .find(|window| {
            window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == surface)
        })
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().ok())
                .is_some_and(|data| data.initial_configure_sent)
        });

        if !initial_configure_sent {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_configure();
            }
        }
    }

    popups.commit(surface);

    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    if let Err(error) = xdg.send_configure() {
                        eprintln!("vanta-comp: failed to send initial popup configure: {error}");
                    }
                }
            }

            PopupKind::InputMethod(_) => {}
        }
    }
}

impl Vanta {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };

        let Some(window) = self.space.elements().find(|window| {
            window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == &root)
        }) else {
            return;
        };

        let Some(output) = self.space.outputs().next() else {
            return;
        };

        let Some(output_geo) = self.space.output_geometry(output) else {
            return;
        };

        let Some(window_geo) = self.space.element_geometry(window) else {
            return;
        };

        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
