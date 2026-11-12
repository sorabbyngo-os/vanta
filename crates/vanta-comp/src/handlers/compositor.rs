use crate::{grabs::resize_grab, state::ClientState, Vanta};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::layer_map_for_output,
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_surface::WlSurface},
        Client,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler,
            CompositorState,
        },
        shell::wlr_layer::WlrLayerShellHandler,
        shm::{ShmHandler, ShmState},
    },
};

use super::xdg_shell;

impl CompositorHandler for Vanta {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

        let layer_surface = self
            .shell_state()
            .layer_surfaces()
            .find(|layer| layer.wl_surface() == surface);

        if let Some(layer_surface) = layer_surface {
            layer_surface.send_pending_configure();

            for output in self.space.outputs() {
                let mut map = layer_map_for_output(output);

                let layer = map
                    .layers()
                    .find(|layer| layer.layer_surface() == &layer_surface)
                    .cloned();

                if let Some(layer) = layer {
                    map.arrange();
                }
            }
        }
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            // Resolve against the persistent window registry, not Space:
            // inactive workspace windows are unmapped from Space but must
            // continue processing commits for live overview previews.
            if let Some(window) = self
                .windows
                .values()
                .find(|w| w.toplevel().unwrap().wl_surface() == &root)
                .cloned()
            {
                window.on_commit();
            }
        };

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
        resize_grab::handle_commit(&mut self.space, surface);

        if let Some(sender) = self.native_redraw_sender.as_ref() {
            let _ = sender.send(());
        }
    }
}

impl BufferHandler for Vanta {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Vanta {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
