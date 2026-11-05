use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::{solid::SolidColorRenderElement, Kind},
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::Vanta;

pub fn init_winit(
    event_loop: &mut EventLoop<Vanta>,
    state: &mut Vanta,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;
    backend.window().set_title("Vanta");

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "vanta".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Sorabbyngo".into(),
            model: "Vanta".into(),
            serial_number: "0".into(),
        },
    );

    let _global = output.create_global::<Vanta>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));
    state.output = Some(output.clone());
    state.relayout_windows();

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| {
            match event {
                WinitEvent::Resized { size, .. } => {
                    output.change_current_state(
                        Some(Mode {
                            size,
                            refresh: 60_000,
                        }),
                        None,
                        None,
                        None,
                    );
                    state.relayout_windows();
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => {
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);
                    {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();

                        let scale = output.current_scale().fractional_scale();
                        let mut custom_elements = Vec::with_capacity(2);

                        if let Some(preview) = state.snap_preview {
                            state.snap_preview_buffer.resize(preview.size);
                            custom_elements.push(SolidColorRenderElement::from_buffer(
                                &state.snap_preview_buffer,
                                preview.loc.to_physical_precise_round(scale),
                                scale,
                                1.0,
                                Kind::Unspecified,
                            ));
                        }

                        if let Some(preview) = state.minimize_preview {
                            state.minimize_preview_buffer.resize(preview.size);
                            custom_elements.push(SolidColorRenderElement::from_buffer(
                                &state.minimize_preview_buffer,
                                preview.loc.to_physical_precise_round(scale),
                                scale,
                                1.0,
                                Kind::Unspecified,
                            ));
                        }

                        smithay::desktop::space::render_output::<_, SolidColorRenderElement, _, _>(
                            &output,
                            renderer,
                            &mut framebuffer,
                            1.0,
                            0,
                            [&state.space],
                            &custom_elements,
                            &mut damage_tracker,
                            [0.05, 0.06, 0.09, 1.0],
                        )
                        .unwrap();
                    }
                    backend.submit(Some(&[damage])).unwrap();

                    state.space.elements().for_each(|window| {
                        window.send_frame(
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        )
                    });

                    state.space.refresh();
                    state.popups.cleanup();
                    let _ = state.display_handle.flush_clients();
                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
            };
        })?;

    Ok(())
}
