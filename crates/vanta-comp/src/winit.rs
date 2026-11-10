use std::{io::Cursor, time::Duration};

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                render_elements,
                solid::SolidColorRenderElement,
                surface::WaylandSurfaceRenderElement,
                utils::{
                    ConstrainAlign, ConstrainScaleBehavior, CropRenderElement,
                    RelocateRenderElement, RescaleRenderElement,
                },
                AsRenderElements, Kind,
            },
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{IsAlive, Logical, Point, Rectangle, Scale, Transform},
};

use crate::{state::create_overview_glass_buffer, Vanta};

render_elements! {
    OverviewRenderElements<=GlesRenderer>;
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Space=smithay::desktop::space::SpaceRenderElements<
        GlesRenderer,
        WaylandSurfaceRenderElement<GlesRenderer>
    >,
    Thumbnail=CropRenderElement<
        RelocateRenderElement<
            RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>
        >
    >,
}

fn load_wallpaper() -> Result<MemoryRenderBuffer, Box<dyn std::error::Error>> {
    let bytes = include_bytes!("../../../assets/wallpapers/vanta-default.png");
    let decoder = png::Decoder::new(Cursor::new(bytes.as_slice()));
    let mut reader = decoder.read_info()?;
    let mut decoded = vec![
        0;
        reader.output_buffer_size().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid wallpaper dimensions",
            )
        })?
    ];
    let info = reader.next_frame(&mut decoded)?;
    let pixels = &decoded[..info.buffer_size()];

    let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    match info.color_type {
        png::ColorType::Rgb => {
            for pixel in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Rgba => {
            for pixel in pixels.chunks_exact(4) {
                rgba.extend_from_slice(pixel);
            }
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Wallpaper must decode to RGB or RGBA",
            )
            .into());
        }
    }

    let width = i32::try_from(info.width)?;
    let height = i32::try_from(info.height)?;

    Ok(MemoryRenderBuffer::from_slice(
        &rgba,
        smithay::backend::allocator::Fourcc::Abgr8888,
        (width, height),
        1,
        Transform::Normal,
        None,
    ))
}

pub fn init_winit(
    event_loop: &mut EventLoop<Vanta>,
    state: &mut Vanta,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;
    backend.window().set_title("Vanta");
    backend.window().set_decorations(false);

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
    let wallpaper_buffer = load_wallpaper()?;

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
                        let mut custom_elements =
                            Vec::<OverviewRenderElements>::with_capacity(state.windows.len() + 16);
                        let mut wallpaper_element = None;

                        if let Some(geometry) = state.space.output_geometry(&output) {
                            if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                                renderer,
                                Point::<i32, Logical>::from((0, 0))
                                    .to_physical_precise_round(scale),
                                &wallpaper_buffer,
                                None,
                                None,
                                Some(geometry.size),
                                Kind::Unspecified,
                            ) {
                                wallpaper_element = Some(element.into());
                            }
                        }

                        if let Some(preview) = state.snap_preview {
                            state.snap_preview_buffer.resize(preview.size);
                            custom_elements.push(
                                SolidColorRenderElement::from_buffer(
                                    &state.snap_preview_buffer,
                                    preview.loc.to_physical_precise_round(scale),
                                    scale,
                                    1.0,
                                    Kind::Unspecified,
                                )
                                .into(),
                            );
                        }

                        if let Some(preview) = state.minimize_preview {
                            state.minimize_preview_buffer.resize(preview.size);
                            custom_elements.push(
                                SolidColorRenderElement::from_buffer(
                                    &state.minimize_preview_buffer,
                                    preview.loc.to_physical_precise_round(scale),
                                    scale,
                                    1.0,
                                    Kind::Unspecified,
                                )
                                .into(),
                            );
                        }

                        if state.overview_open {
                            if let Some(layout) = state.overview_layout() {
                                while state.overview_card_buffers.len() < layout.thumbnails.len() {
                                    state.overview_card_buffers.push(create_overview_glass_buffer());
                                }

                                let overview_surfaces = layout
                                    .thumbnails
                                    .iter()
                                    .map(|(workspace, rect)| (*workspace, *rect, false))
                                    .chain(std::iter::once((
                                        state.active_workspace,
                                        layout.active_preview,
                                        true,
                                    )));

                                for (workspace, rect, is_main_preview) in overview_surfaces {
                                    let buffer = if is_main_preview {
                                        &mut state.overview_active_buffer
                                    } else {
                                        &mut state.overview_card_buffers[workspace as usize]
                                    };

                                    if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                                        renderer,
                                        rect.loc.to_physical_precise_round(scale),
                                        buffer,
                                        None,
                                        None,
                                        Some(rect.size),
                                        Kind::Unspecified,
                                    ) {
                                        custom_elements.push(element.into());
                                    }

                                    let inset = if is_main_preview { 16 } else { 5 };
                                    let reserved_height = if is_main_preview { 38 } else { 12 };
                                    let preview: Rectangle<i32, Logical> = Rectangle::new(
                                        (rect.loc.x + inset, rect.loc.y + inset).into(),
                                        (
                                            (rect.size.w - inset * 2).max(1),
                                            (rect.size.h - inset - reserved_height).max(1),
                                        )
                                            .into(),
                                    );

                                    let mut windows: Vec<(u64, smithay::desktop::Window)> =
                                        state.windows.iter()
                                            .filter(|(id, window)| {
                                                state.window_workspaces.get(id) == Some(&workspace)
                                                    && !state.minimized_windows.contains_key(id)
                                                    && window.alive()
                                            })
                                            .map(|(id, window)| (*id, window.clone()))
                                            .collect();
                                    windows.sort_by_key(|(id, _)| *id);

                                    let count = windows.len();
                                    if count > 0 {
                                        let columns = (count as f64).sqrt().ceil() as usize;
                                        let rows = count.div_ceil(columns);
                                        let cell_width =
                                            (preview.size.w / columns as i32).max(1);
                                        let cell_height =
                                            (preview.size.h / rows as i32).max(1);

                                        for (index, (id, window)) in
                                            windows.into_iter().enumerate()
                                        {
                                            let column = index % columns;
                                            let row = index / columns;
                                            let cell = Rectangle::new(
                                                (
                                                    preview.loc.x + column as i32 * cell_width,
                                                    preview.loc.y + row as i32 * cell_height,
                                                )
                                                    .into(),
                                                (
                                                    if column + 1 == columns {
                                                        preview.size.w - column as i32 * cell_width
                                                    } else {
                                                        cell_width
                                                    }
                                                    .max(1),
                                                    if row + 1 == rows {
                                                        preview.size.h - row as i32 * cell_height
                                                    } else {
                                                        cell_height
                                                    }
                                                    .max(1),
                                                )
                                                    .into(),
                                            );
                                            let location = state.window_locations
                                                .get(&id)
                                                .copied()
                                                .unwrap_or_else(|| (0, 0).into());

                                            custom_elements.extend(
                                                smithay::desktop::space::constrain_space_element::<
                                                    GlesRenderer,
                                                    smithay::desktop::Window,
                                                    OverviewRenderElements,
                                                    _,
                                                    _,
                                                >(
                                                    renderer,
                                                    &window,
                                                    location,
                                                    1.0,
                                                    scale,
                                                    cell,
                                                    smithay::desktop::space::ConstrainBehavior {
                                                        reference: smithay::desktop::space::ConstrainReference::BoundingBox,
                                                        behavior: ConstrainScaleBehavior::Fit,
                                                        align: ConstrainAlign::CENTER,
                                                    },
                                                ),
                                            );
                                        }
                                    }
                                }

                                let new_card = layout.new_workspace;
                                if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                                    renderer,
                                    new_card.loc.to_physical_precise_round(scale),
                                    &state.overview_new_buffer,
                                    None,
                                    None,
                                    Some(new_card.size),
                                    Kind::Unspecified,
                                ) {
                                    custom_elements.push(element.into());
                                }

                                let plus_width = 32_i32;
                                let plus_height = 4_i32;
                                let center_x = new_card.loc.x + new_card.size.w / 2;
                                let center_y = new_card.loc.y + new_card.size.h / 2;

                                state.overview_plus_buffer.resize((
                                    plus_height,
                                    plus_width,
                                ));
                                custom_elements.push(
                                    SolidColorRenderElement::from_buffer(
                                        &state.overview_plus_buffer,
                                        Point::<i32, Logical>::from((
                                            center_x - plus_height / 2,
                                            center_y - plus_width / 2,
                                        ))
                                        .to_physical_precise_round(scale),
                                        scale,
                                        1.0,
                                        Kind::Unspecified,
                                    )
                                    .into(),
                                );

                                state.overview_plus_vertical_buffer.resize((
                                    plus_width,
                                    plus_height,
                                ));
                                custom_elements.push(
                                    SolidColorRenderElement::from_buffer(
                                        &state.overview_plus_vertical_buffer,
                                        Point::<i32, Logical>::from((
                                            center_x - plus_width / 2,
                                            center_y - plus_height / 2,
                                        ))
                                        .to_physical_precise_round(scale),
                                        scale,
                                        1.0,
                                        Kind::Unspecified,
                                    )
                                    .into(),
                                );
                            }
                        }
                        let alpha = if state.overview_open { 0.55 } else { 1.0 };

                        // OutputDamageTracker expects front-to-back elements.
                        // It draws them in reverse: background first, foreground last.
                        let mut elements = Vec::with_capacity(custom_elements.len() + 32);

                        // Vanta overlays: overview cards, previews and other foreground UI.
                        elements.append(&mut custom_elements);

                        // Render layer-shell surfaces using their arranged LayerMap
                        // positions. Smithay's output helper currently drops those offsets.
                        let output_geometry =
                            state.space.output_geometry(&output).unwrap();

                        let (front_layers, back_layers) = {
                            let layer_map =
                                smithay::desktop::layer_map_for_output(&output);
                            let mut front = Vec::new();
                            let mut back = Vec::new();

                            for layer in layer_map.layers().rev() {
                                if let Some(geometry) = layer_map.layer_geometry(layer) {
                                    let entry = (layer.clone(), geometry);
                                    match layer.layer() {
                                        smithay::wayland::shell::wlr_layer::Layer::Overlay
                                        | smithay::wayland::shell::wlr_layer::Layer::Top => {
                                            front.push(entry);
                                        }
                                        smithay::wayland::shell::wlr_layer::Layer::Bottom
                                        | smithay::wayland::shell::wlr_layer::Layer::Background => {
                                            back.push(entry);
                                        }
                                    }
                                }
                            }

                            (front, back)
                        };

                        let mut space_elements =
                            Vec::<OverviewRenderElements>::with_capacity(32);

                        // Top and overlay surfaces are in front of application windows.
                        for (layer, geometry) in front_layers {
                            let location = (geometry.loc - output_geometry.loc)
                                .to_physical_precise_round(scale);

                            let rendered = layer.render_elements::<
                                WaylandSurfaceRenderElement<GlesRenderer>,
                            >(
                                renderer,
                                location,
                                Scale::from(scale),
                                alpha,
                            );

                            space_elements.extend(rendered.into_iter().map(|element| {
                                OverviewRenderElements::Space(
                                    smithay::desktop::space::SpaceRenderElements::Surface(element),
                                )
                            }));
                        }

                        // Ordinary windows retain Smithay's existing output-region rendering.
                        let window_elements = state.space.render_elements_for_region(
                            renderer,
                            &output_geometry,
                            scale,
                            alpha,
                        );

                        space_elements.extend(window_elements.into_iter().map(|element| {
                            OverviewRenderElements::Space(
                                smithay::desktop::space::SpaceRenderElements::Surface(element),
                            )
                        }));

                        // Bottom and background surfaces remain behind application windows.
                        for (layer, geometry) in back_layers {
                            let location = (geometry.loc - output_geometry.loc)
                                .to_physical_precise_round(scale);

                            let rendered = layer.render_elements::<
                                WaylandSurfaceRenderElement<GlesRenderer>,
                            >(
                                renderer,
                                location,
                                Scale::from(scale),
                                alpha,
                            );

                            space_elements.extend(rendered.into_iter().map(|element| {
                                OverviewRenderElements::Space(
                                    smithay::desktop::space::SpaceRenderElements::Surface(element),
                                )
                            }));
                        }

                        elements.extend(space_elements);

                        // Wallpaper must be behind both the desktop and Vanta overlays.
                        if let Some(wallpaper) = wallpaper_element {
                            elements.push(wallpaper);
                        }

                        let clear_color = if state.overview_open {
                            [0.035, 0.052, 0.078, 1.0]
                        } else {
                            [0.05, 0.06, 0.09, 1.0]
                        };

                        damage_tracker
                            .render_output(
                                renderer,
                                &mut framebuffer,
                                0,
                                &elements,
                                clear_color,
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
