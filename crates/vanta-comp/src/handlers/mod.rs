mod compositor;
mod xdg_shell;

use crate::Vanta;

use smithay::{
    desktop::{layer_map_for_output, LayerSurface},
    input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source},
    input::pointer::Focus,
    input::{Seat, SeatHandler, SeatState},
    output::Output,
    reexports::wayland_server::{
        protocol::{wl_output, wl_surface::WlSurface},
        Resource,
    },
    utils::Serial,
    wayland::{
        output::OutputHandler,
        pointer_constraints::PointerConstraintsHandler,
        selection::{
            data_device::{
                set_data_device_focus, DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::wlr_layer::{
            Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState,
        },
    },
};

impl SeatHandler for Vanta {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Vanta> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|surface| dh.get_client(surface.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

impl PointerConstraintsHandler for Vanta {}

impl SelectionHandler for Vanta {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Vanta {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for Vanta {}

impl WaylandDndGrabHandler for Vanta {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let Some(pointer) = seat.get_pointer() else {
                    source.cancel();
                    return;
                };

                let Some(start_data) = pointer.grab_start_data() else {
                    source.cancel();
                    return;
                };

                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);

                pointer.set_grab(self, grab, serial, Focus::Keep);
            }

            GrabType::Touch => {
                // Vanta does not implement touch-based drag-and-drop yet.
                source.cancel();
            }
        }
    }
}

impl OutputHandler for Vanta {}

impl WlrLayerShellHandler for Vanta {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        eprintln!("vanta-comp: new layer surface namespace={namespace}");

        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned());

        let Some(output) = output else {
            eprintln!("vanta-comp: cannot map layer surface {namespace}: no output available");
            return;
        };

        let mut map = layer_map_for_output(&output);
        let layer = LayerSurface::new(surface, namespace.clone());

        if map.map_layer(&layer).is_err() {
            eprintln!("vanta-comp: failed to map layer surface");
        } else {
            let output_geometry = self.space.output_geometry(&output);
            let layer_geometry = map.layer_geometry(&layer);

            eprintln!(
                "vanta-comp: layer={namespace} output={output_geometry:?} mapped={layer_geometry:?}"
            );
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        if let Some((mut map, layer)) = self.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);

            let layer = map
                .layers()
                .find(|layer| layer.layer_surface() == &surface)
                .cloned();

            layer.map(|layer| (map, layer))
        }) {
            map.unmap_layer(&layer);
        }
    }
}

smithay::delegate_dispatch2!(Vanta);
