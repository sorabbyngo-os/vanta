//! Native session and DRM device lifecycle.
//!
//! This milestone handles libseat ownership, Udev discovery, DRM device
//! opening, GBM initialization, EGL/GLES renderer initialization, output
//! discovery, notifier registration, and device removal.
//! DRM modesetting, presentation, and input forwarding are separate milestones.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    error::Error,
    os::unix::io::OwnedFd,
    path::{Path, PathBuf},
    rc::Rc,
};

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{
            compositor::{DrmCompositor, FrameFlags, PrimaryPlaneElement},
            exporter::gbm::{GbmFramebufferExporter, NodeFilter},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode,
        },
        egl::{EGLContext, EGLDisplay},
        renderer::{element::surface::WaylandSurfaceRenderElement, gles::GlesRenderer, ImportDma},
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{UdevBackend, UdevEvent},
    },
    reexports::{
        calloop::{channel, EventLoop, LoopHandle, RegistrationToken},
        drm::control::{connector, crtc, Device as ControlDevice, Mode, ModeTypeFlags},
        rustix::fs::OFlags,
        wayland_server::backend::GlobalId,
    },
    utils::DeviceFd,
};
use smithay::{
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    utils::Transform,
};
use tracing::{error, info, warn};

use smithay::backend::renderer::element::{
    memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
    render_elements,
    solid::SolidColorRenderElement,
    surface::WaylandSurfaceRenderElement as NativeSurfaceElement,
    utils::{
        ConstrainAlign, ConstrainScaleBehavior, CropRenderElement, RelocateRenderElement,
        RescaleRenderElement,
    },
    AsRenderElements, Kind,
};
use smithay::utils::{IsAlive, Logical, Point, Rectangle, Scale};

use crate::state::create_overview_glass_buffer;
use crate::Vanta;

type NativeDrmCompositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

render_elements! {
    NativeSceneElements<=GlesRenderer>;
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Surface=NativeSurfaceElement<GlesRenderer>,
    Space=smithay::desktop::space::SpaceRenderElements<
        GlesRenderer,
        NativeSurfaceElement<GlesRenderer>
    >,
    Thumbnail=CropRenderElement<
        RelocateRenderElement<
            RescaleRenderElement<NativeSurfaceElement<GlesRenderer>>
        >
    >,
}

struct NativeOutput {
    connector: connector::Handle,
    crtc: crtc::Handle,
    mode: Mode,
    name: String,
    physical_size: Option<(u32, u32)>,
    serial_number: u32,
    compositor: Option<NativeDrmCompositor>,
}

struct NativeDrmDevice {
    drm: DrmDevice,
    _gbm: GbmDevice<DrmDeviceFd>,
    egl_display: Option<EGLDisplay>,
    renderer: Option<GlesRenderer>,
    wallpaper: Option<MemoryRenderBuffer>,
    outputs: Vec<NativeOutput>,
    session_fd: DeviceFd,
    notifier_token: RegistrationToken,
}

pub(crate) struct NativeSession {
    _session: Rc<RefCell<LibSeatSession>>,
}

fn close_session_device(session: &Rc<RefCell<LibSeatSession>>, fd: DeviceFd) {
    let owned_fd: OwnedFd = match fd.try_into() {
        Ok(fd) => fd,
        Err(fd) => {
            error!(
                raw_fd = std::os::unix::io::AsRawFd::as_raw_fd(&fd),
                "Cannot close DRM device: device FD still has outstanding owners"
            );
            return;
        }
    };

    if let Err(err) = session.borrow_mut().close(owned_fd) {
        warn!(?err, "Failed to close DRM device through libseat");
    }
}

fn discover_outputs(node: DrmNode, drm: &DrmDevice) -> Result<Vec<NativeOutput>, Box<dyn Error>> {
    let resources = drm.resource_handles()?;
    let mut outputs = Vec::new();
    let mut used_crtcs = HashSet::new();

    info!(
        ?node,
        connectors = resources.connectors().len(),
        crtcs = resources.crtcs().len(),
        "Enumerating native DRM display resources"
    );

    for connector_handle in resources.connectors() {
        let connector_info = match drm.get_connector(*connector_handle, true) {
            Ok(info) => info,
            Err(err) => {
                warn!(
                    ?node,
                    ?connector_handle,
                    ?err,
                    "Failed to probe DRM connector"
                );
                continue;
            }
        };

        if connector_info.state() != connector::State::Connected {
            tracing::debug!(
                ?node,
                connector = %connector_info,
                state = ?connector_info.state(),
                "Skipping disconnected DRM connector"
            );
            continue;
        }

        let mode = connector_info
            .modes()
            .iter()
            .copied()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| {
                connector_info.modes().iter().copied().max_by_key(|mode| {
                    let (width, height) = mode.size();
                    (u32::from(width) * u32::from(height), mode.vrefresh())
                })
            });

        let Some(mode) = mode else {
            warn!(
                ?node,
                connector = %connector_info,
                "Connected DRM connector reports no display modes"
            );
            continue;
        };

        let mut selected = None;

        'encoder_search: for encoder_handle in connector_info.encoders() {
            let encoder_info = match drm.get_encoder(*encoder_handle) {
                Ok(info) => info,
                Err(err) => {
                    warn!(?node, ?encoder_handle, ?err, "Failed to query DRM encoder");
                    continue;
                }
            };

            for crtc_handle in resources.filter_crtcs(encoder_info.possible_crtcs()) {
                if used_crtcs.contains(&crtc_handle) {
                    continue;
                }

                selected = Some(NativeOutput {
                    connector: *connector_handle,
                    crtc: crtc_handle,
                    mode,
                    name: connector_info.to_string(),
                    physical_size: connector_info.size(),
                    serial_number: connector_info.interface_id(),
                    compositor: None,
                });

                break 'encoder_search;
            }
        }

        match selected {
            Some(output) => {
                let (width, height) = output.mode.size();

                info!(
                    ?node,
                    connector = %connector_info,
                    ?output.crtc,
                    mode = %output.mode.name().to_string_lossy(),
                    width,
                    height,
                    refresh_hz = output.mode.vrefresh(),
                    "Selected native DRM display configuration"
                );

                used_crtcs.insert(output.crtc);
                outputs.push(output);
            }
            None => {
                warn!(
                    ?node,
                    connector = %connector_info,
                    "No compatible unused CRTC found"
                );
            }
        }
    }

    info!(
        ?node,
        output_count = outputs.len(),
        "Native DRM connector discovery completed"
    );

    Ok(outputs)
}

fn initialize_native_renderer(
    node: DrmNode,
    gbm: &GbmDevice<DrmDeviceFd>,
) -> Result<(EGLDisplay, GlesRenderer), Box<dyn Error>> {
    // The renderer is created and used on the compositor event-loop thread.
    // No other thread may make this EGL context current concurrently.
    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let renderer = unsafe { GlesRenderer::new(context)? };

    info!(
        ?node,
        software = renderer.is_software(),
        "Initialized native EGL/GLES renderer"
    );

    Ok((display, renderer))
}

fn sync_native_output(
    state: &mut Vanta,
    devices: &mut HashMap<DrmNode, NativeDrmDevice>,
    global: &mut Option<GlobalId>,
) {
    // Drop existing DRM compositors before replacing the output or its surface.
    for device in devices.values_mut() {
        for native_output in &mut device.outputs {
            native_output.compositor.take();
        }
    }

    if let Some(id) = global.take() {
        state.display_handle.remove_global::<Vanta>(id);
    }

    if let Some(previous) = state.output.take() {
        state.space.unmap_output(&previous);
    }

    let candidate = devices.iter().find_map(|(node, device)| {
        if device.renderer.is_none() {
            return None;
        }

        device.outputs.first().map(|native_output| {
            (
                *node,
                native_output.connector,
                native_output.crtc,
                native_output.mode,
                native_output.name.clone(),
                native_output.physical_size,
                native_output.serial_number,
            )
        })
    });

    let Some((node, connector, crtc, drm_mode, name, physical_size, serial_number)) = candidate
    else {
        state.relayout_windows();
        info!("No active native DRM output is currently available");
        return;
    };

    let (width, height) = drm_mode.size();
    let refresh = i32::try_from(drm_mode.vrefresh().saturating_mul(1000)).unwrap_or(60_000);

    let physical_size = physical_size
        .map(|(width, height)| {
            (
                i32::try_from(width).unwrap_or(0),
                i32::try_from(height).unwrap_or(0),
            )
        })
        .unwrap_or((0, 0));

    let mode = OutputMode {
        size: (i32::from(width), i32::from(height)).into(),
        refresh,
    };

    let output = Output::new(
        name.clone(),
        PhysicalProperties {
            size: physical_size.into(),
            subpixel: Subpixel::Unknown,
            make: "Sorabbyngo".into(),
            model: name.clone(),
            serial_number: serial_number.to_string(),
        },
    );

    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    let Some(device) = devices.get_mut(&node) else {
        warn!(
            ?node,
            "Selected native DRM device disappeared during output setup"
        );
        state.relayout_windows();
        return;
    };

    let surface = match device.drm.create_surface(crtc, drm_mode, &[connector]) {
        Ok(surface) => surface,
        Err(err) => {
            warn!(?node, ?err, connector = %name, "Failed to create native DRM surface");
            state.relayout_windows();
            return;
        }
    };

    let Some(renderer) = device.renderer.as_ref() else {
        warn!(?node, "Native renderer disappeared during output setup");
        state.relayout_windows();
        return;
    };

    let renderer_formats = renderer.dmabuf_formats();
    let allocator = GbmAllocator::new(
        device._gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let framebuffer_exporter = GbmFramebufferExporter::new(device._gbm.clone(), NodeFilter::None);

    let mut compositor = match DrmCompositor::new(
        &output,
        surface,
        None,
        allocator,
        framebuffer_exporter,
        [Fourcc::Argb8888],
        renderer_formats.into_iter(),
        device.drm.cursor_size(),
        Some(device._gbm.clone()),
    ) {
        Ok(compositor) => compositor,
        Err(err) => {
            warn!(?node, ?err, connector = %name, "Failed to initialize native DRM compositor");
            state.relayout_windows();
            return;
        }
    };

    let Some(native_output) = device
        .outputs
        .iter_mut()
        .find(|native_output| native_output.connector == connector && native_output.crtc == crtc)
    else {
        warn!(?node, connector = %name, "Selected native output disappeared during compositor setup");
        state.relayout_windows();
        return;
    };

    native_output.compositor = Some(compositor);

    *global = Some(output.create_global::<Vanta>(&state.display_handle));
    state.space.map_output(&output, (0, 0));

    // Arrange layer-shell surfaces against the newly mapped native output.
    {
        let mut layer_map = smithay::desktop::layer_map_for_output(&output);
        layer_map.arrange();
    }

    state.output = Some(output);
    state.relayout_windows();

    render_native_scene(state, node, crtc, devices);

    info!(
        ?node,
        connector = %name,
        width,
        height,
        refresh_millihz = refresh,
        "Initialized native DRM compositor and published Wayland output"
    );
}
fn render_native_scene(
    state: &mut Vanta,
    node: DrmNode,
    crtc: crtc::Handle,
    devices: &mut HashMap<DrmNode, NativeDrmDevice>,
) {
    static LAYER_DIAGNOSTIC_FRAME: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    let log_layer_diagnostics =
        LAYER_DIAGNOSTIC_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 120 == 0;

    let Some(output) = state.output.as_ref() else {
        return;
    };
    let Some(output_geometry) = state.space.output_geometry(output) else {
        return;
    };

    let scale = output.current_scale().fractional_scale();

    let Some(device) = devices.get_mut(&node) else {
        return;
    };
    let Some(renderer) = device.renderer.as_mut() else {
        return;
    };
    let Some(native_output) = device.outputs.iter_mut().find(|item| item.crtc == crtc) else {
        return;
    };
    let Some(compositor) = native_output.compositor.as_mut() else {
        return;
    };

    let mut overlay_elements = Vec::<NativeSceneElements>::with_capacity(8);
    let mut desktop_elements = Vec::<NativeSceneElements>::with_capacity(32);
    let mut wallpaper_element = None;

    if let Some(wallpaper) = device.wallpaper.as_ref() {
        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::<i32, Logical>::from((0, 0)).to_physical_precise_round(scale),
            wallpaper,
            None,
            None,
            Some(output_geometry.size),
            Kind::Unspecified,
        ) {
            wallpaper_element = Some(NativeSceneElements::Memory(element));
        }
    }

    if let Some(preview) = state.snap_preview {
        state.snap_preview_buffer.resize(preview.size);
        overlay_elements.push(NativeSceneElements::Solid(
            SolidColorRenderElement::from_buffer(
                &state.snap_preview_buffer,
                preview.loc.to_physical_precise_round(scale),
                scale,
                1.0,
                Kind::Unspecified,
            ),
        ));
    }

    if let Some(preview) = state.minimize_preview {
        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            preview.loc.to_physical_precise_round(scale),
            &state.minimize_preview_buffer,
            None,
            None,
            Some(preview.size),
            Kind::Unspecified,
        ) {
            overlay_elements.push(NativeSceneElements::Memory(element));
        }
    }

    if state.overview_open {
        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::<i32, Logical>::from((12, 12)).to_physical_precise_round(scale),
            &state.overview_indicator_buffer,
            None,
            Some(Rectangle::from_size((128.0, 20.0).into())),
            Some((128, 20).into()),
            Kind::Unspecified,
        ) {
            overlay_elements.push(NativeSceneElements::Memory(element));
        }

        if let Some(layout) = state.overview_layout() {
            while state.overview_card_buffers.len() < layout.thumbnails.len() {
                state
                    .overview_card_buffers
                    .push(create_overview_glass_buffer());
            }

            for (workspace, rect) in &layout.thumbnails {
                if let Some(buffer) = state.overview_card_buffers.get(*workspace as usize) {
                    if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        rect.loc.to_physical_precise_round(scale),
                        buffer,
                        None,
                        None,
                        Some(rect.size),
                        Kind::Unspecified,
                    ) {
                        overlay_elements.push(NativeSceneElements::Memory(element));
                    }
                }
            }

            if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                layout.active_preview.loc.to_physical_precise_round(scale),
                &state.overview_active_buffer,
                None,
                None,
                Some(layout.active_preview.size),
                Kind::Unspecified,
            ) {
                overlay_elements.push(NativeSceneElements::Memory(element));
            }

            if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                layout.new_workspace.loc.to_physical_precise_round(scale),
                &state.overview_new_buffer,
                None,
                None,
                Some(layout.new_workspace.size),
                Kind::Unspecified,
            ) {
                overlay_elements.push(NativeSceneElements::Memory(element));
            }
        }
    }

    if state.overview_open {
        info!(
            active_workspace = state.active_workspace,
            "Native overview scene rendering entered"
        );

        let mut thumbnail_elements = Vec::<NativeSceneElements>::new();

        if let Some(layout) = state.overview_layout() {
            info!(
                thumbnail_workspaces = layout.thumbnails.len(),
                "Native overview layout generated"
            );
            let overview_surfaces = layout
                .thumbnails
                .iter()
                .map(|(workspace, rect)| (*workspace, *rect))
                .chain(std::iter::once((
                    state.active_workspace,
                    layout.active_preview,
                )));

            for (workspace, rect) in overview_surfaces {
                let is_active = workspace == state.active_workspace;
                let inset = if is_active { 16 } else { 5 };
                let reserved_height = if is_active { 38 } else { 12 };

                let preview: Rectangle<i32, Logical> = Rectangle::new(
                    (rect.loc.x + inset, rect.loc.y + inset).into(),
                    (
                        (rect.size.w - inset * 2).max(1),
                        (rect.size.h - inset - reserved_height).max(1),
                    )
                        .into(),
                );

                let mut windows: Vec<(
                    u64,
                    smithay::desktop::Window,
                    Point<i32, Logical>,
                    Rectangle<i32, Logical>,
                )> = state
                    .windows
                    .iter()
                    .filter(|(id, window)| {
                        state.window_workspaces.get(id) == Some(&workspace)
                            && !state.minimized_windows.contains_key(id)
                            && window.alive()
                    })
                    .filter_map(|(id, window)| {
                        let location = state
                            .space
                            .element_location(window)
                            .or_else(|| state.window_locations.get(id).copied())?;

                        let geometry = window.geometry();
                        let bbox = window.bbox_with_popups();
                        let bounds = Rectangle::new(location + bbox.loc - geometry.loc, bbox.size);

                        Some((*id, window.clone(), location, bounds))
                    })
                    .collect();

                info!(
                    workspace,
                    matched_windows = windows.len(),
                    "Native overview workspace thumbnail input"
                );

                windows.sort_by_key(|(id, _, _, _)| *id);

                let stacking: Vec<smithay::desktop::Window> =
                    state.space.elements().cloned().collect();

                windows.sort_by_key(|(_, window, _, _)| {
                    stacking
                        .iter()
                        .position(|candidate| candidate == window)
                        .unwrap_or(usize::MAX)
                });

                if windows.is_empty() {
                    continue;
                }

                let min_x = windows
                    .iter()
                    .map(|(_, _, _, bounds)| bounds.loc.x)
                    .min()
                    .unwrap();
                let min_y = windows
                    .iter()
                    .map(|(_, _, _, bounds)| bounds.loc.y)
                    .min()
                    .unwrap();
                let max_x = windows
                    .iter()
                    .map(|(_, _, _, bounds)| bounds.loc.x + bounds.size.w)
                    .max()
                    .unwrap();
                let max_y = windows
                    .iter()
                    .map(|(_, _, _, bounds)| bounds.loc.y + bounds.size.h)
                    .max()
                    .unwrap();

                let workspace_width = (max_x - min_x).max(1);
                let workspace_height = (max_y - min_y).max(1);

                let uniform_scale = (preview.size.w as f64 / workspace_width as f64)
                    .min(preview.size.h as f64 / workspace_height as f64);

                let rendered_width = (workspace_width as f64 * uniform_scale).round() as i32;
                let rendered_height = (workspace_height as f64 * uniform_scale).round() as i32;

                let offset_x = preview.loc.x + (preview.size.w - rendered_width) / 2;
                let offset_y = preview.loc.y + (preview.size.h - rendered_height) / 2;

                for (_, window, _location, bounds) in windows {
                    let target = Rectangle::new(
                        (
                            offset_x
                                + ((bounds.loc.x - min_x) as f64 * uniform_scale).round() as i32,
                            offset_y
                                + ((bounds.loc.y - min_y) as f64 * uniform_scale).round() as i32,
                        )
                            .into(),
                        (
                            ((bounds.size.w as f64 * uniform_scale).round() as i32).max(1),
                            ((bounds.size.h as f64 * uniform_scale).round() as i32).max(1),
                        )
                            .into(),
                    );

                    let elements = smithay::desktop::space::constrain_space_element::<
                        GlesRenderer,
                        smithay::desktop::Window,
                        NativeSceneElements,
                        _,
                        _,
                    >(
                        renderer,
                        &window,
                        target.loc,
                        1.0,
                        scale,
                        target,
                        smithay::desktop::space::ConstrainBehavior {
                            reference: smithay::desktop::space::ConstrainReference::BoundingBox,
                            behavior: ConstrainScaleBehavior::Fit,
                            align: ConstrainAlign::CENTER,
                        },
                    );

                    let elements: Vec<_> = elements.collect();
                    info!(
                        workspace,
                        generated_elements = elements.len(),
                        "Native overview window thumbnail elements"
                    );
                    thumbnail_elements.extend(elements);
                }
            }
        }

        // Indicator remains first; glass cards render over thumbnail content.
        let insert_at = usize::from(!overlay_elements.is_empty());
        overlay_elements.splice(insert_at..insert_at, thumbnail_elements);
    }

    let (front_layers, back_layers) = {
        let layer_map = smithay::desktop::layer_map_for_output(output);

        if log_layer_diagnostics {
            info!(
                layer_count = layer_map.layers().count(),
                output_geometry = ?output_geometry,
                "Native layer map snapshot"
            );
        }
        let mut front = Vec::new();
        let mut back = Vec::new();

        for layer in layer_map.layers().rev() {
            if let Some(geometry) = layer_map.layer_geometry(layer) {
                let entry = (layer.clone(), geometry);
                match layer.layer() {
                    smithay::wayland::shell::wlr_layer::Layer::Overlay
                    | smithay::wayland::shell::wlr_layer::Layer::Top => front.push(entry),
                    smithay::wayland::shell::wlr_layer::Layer::Bottom
                    | smithay::wayland::shell::wlr_layer::Layer::Background => back.push(entry),
                }
            }
        }

        (front, back)
    };

    for (layer, geometry) in front_layers {
        if state.overview_open {
            continue;
        }
        let location = (geometry.loc - output_geometry.loc).to_physical_precise_round(scale);
        let rendered: Vec<_> = layer
            .render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer,
                location,
                Scale::from(scale),
                1.0,
            )
            .into_iter()
            .collect();

        if log_layer_diagnostics {
            info!(
                layer = ?layer.layer(),
                geometry = ?geometry,
                rendered_elements = rendered.len(),
                "Native front layer render"
            );
        }

        desktop_elements.extend(rendered.into_iter().map(NativeSceneElements::Surface));
    }

    if !state.overview_open {
        let windows =
            state
                .space
                .render_elements_for_region(renderer, &output_geometry, scale, 1.0);
        desktop_elements.extend(windows.into_iter().map(NativeSceneElements::Surface));
    }

    for (layer, geometry) in back_layers {
        if state.overview_open {
            continue;
        }
        let location = (geometry.loc - output_geometry.loc).to_physical_precise_round(scale);
        let rendered: Vec<_> = layer
            .render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer,
                location,
                Scale::from(scale),
                1.0,
            )
            .into_iter()
            .collect();

        if log_layer_diagnostics {
            info!(
                layer = ?layer.layer(),
                geometry = ?geometry,
                rendered_elements = rendered.len(),
                "Native back layer render"
            );
        }

        desktop_elements.extend(rendered.into_iter().map(NativeSceneElements::Surface));
    }

    if state.hot_corner_hint_visible {
        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            output_geometry.loc.to_physical_precise_round(scale),
            &state.hot_corner_buffer,
            None,
            Some(Rectangle::from_size((24.0, 24.0).into())),
            Some((24, 24).into()),
            Kind::Unspecified,
        ) {
            desktop_elements.push(NativeSceneElements::Memory(element));
        }
    }

    let elements =
        crate::scene::compose_front_to_back(overlay_elements, desktop_elements, wallpaper_element);

    let clear_color = if state.overview_open {
        [0.035, 0.052, 0.078, 1.0]
    } else {
        [0.05, 0.06, 0.09, 1.0]
    };

    let frame_result = match compositor.render_frame::<_, _>(
        renderer,
        &elements,
        clear_color,
        FrameFlags::empty(),
    ) {
        Ok(result) => result,
        Err(err) => {
            warn!(?node, ?crtc, ?err, "Failed to render native desktop scene");
            return;
        }
    };

    if frame_result.is_empty {
        return;
    }

    let sync_result = match &frame_result.primary_element {
        PrimaryPlaneElement::Swapchain(primary) => primary.sync.wait(),
        PrimaryPlaneElement::Element(_) => Ok(()),
    };

    if let Err(err) = sync_result {
        warn!(
            ?node,
            ?crtc,
            ?err,
            "Native desktop frame synchronization failed"
        );
        return;
    }

    drop(frame_result);

    if let Err(err) = compositor.queue_frame(()) {
        warn!(?node, ?crtc, ?err, "Failed to queue native desktop scene");
    } else {
        tracing::trace!(?node, ?crtc, "Queued native desktop scene");
    }
}

fn refresh_device_outputs(node: DrmNode, device: &mut NativeDrmDevice) {
    // Discard previous connector/CRTC metadata before rescanning topology.
    device.outputs.clear();

    if !device.drm.is_active() {
        return;
    }

    match discover_outputs(node, &device.drm) {
        Ok(outputs) => {
            device.outputs = outputs;
        }
        Err(err) => {
            warn!(?node, ?err, "Native DRM output discovery failed");
        }
    }
}

fn add_drm_device(
    node: DrmNode,
    path: &Path,
    session: &Rc<RefCell<LibSeatSession>>,
    handle: &LoopHandle<'_, Vanta>,
    devices_shared: &Rc<RefCell<HashMap<DrmNode, NativeDrmDevice>>>,
    devices: &mut HashMap<DrmNode, NativeDrmDevice>,
) -> Result<(), Box<dyn Error>> {
    if devices.contains_key(&node) {
        info!(?node, "DRM device already registered");
        return Ok(());
    }

    let owned_fd = session.borrow_mut().open(
        path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
    )?;

    let session_fd = DeviceFd::from(owned_fd);
    let drm_fd = DrmDeviceFd::new(session_fd.clone());

    let (drm, notifier) = match DrmDevice::new(drm_fd.clone(), true) {
        Ok(result) => result,
        Err(err) => {
            drop(drm_fd);
            close_session_device(session, session_fd);
            return Err(Box::new(err));
        }
    };

    let gbm = match GbmDevice::new(drm_fd.clone()) {
        Ok(gbm) => gbm,
        Err(err) => {
            drop(notifier);
            drop(drm);
            drop(drm_fd);
            close_session_device(session, session_fd);
            return Err(Box::new(err));
        }
    };

    let (egl_display, renderer) = match initialize_native_renderer(node, &gbm) {
        Ok((display, renderer)) => (Some(display), Some(renderer)),
        Err(err) => {
            warn!(
                ?node,
                ?err,
                "Native EGL/GLES initialization failed; device will remain unavailable for output publication"
            );
            (None, None)
        }
    };

    let wallpaper = match crate::winit::load_wallpaper() {
        Ok(buffer) => Some(buffer),
        Err(err) => {
            warn!(?node, ?err, "Failed to load the Vanta desktop wallpaper");
            None
        }
    };

    let outputs = match discover_outputs(node, &drm) {
        Ok(outputs) => outputs,
        Err(err) => {
            warn!(?node, ?err, "Initial native DRM output discovery failed");
            Vec::new()
        }
    };

    drop(drm_fd);

    let devices_for_vblank = Rc::clone(devices_shared);
    let notifier_token = match handle.insert_source(
        notifier,
        move |event, _metadata, state: &mut Vanta| match event {
            DrmEvent::VBlank(crtc) => {
                static VBLANK_DIAGNOSTIC_COUNT: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(0);
                let vblank_sequence =
                    VBLANK_DIAGNOSTIC_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if vblank_sequence < 10 {
                    eprintln!(
                        "[NATIVE-VBLANK] event={} node={:?} crtc={:?}",
                        vblank_sequence, node, crtc
                    );
                }

                let mut devices = devices_for_vblank.borrow_mut();
                let completed = if let Some(device) = devices.get_mut(&node) {
                    if let Some(output) =
                        device.outputs.iter_mut().find(|output| output.crtc == crtc)
                    {
                        if let Some(compositor) = output.compositor.as_mut() {
                            match compositor.frame_submitted() {
                                Ok(_) => {
                                    tracing::trace!(
                                        ?node,
                                        ?crtc,
                                        "Completed native DRM frame on vblank"
                                    );
                                    true
                                }
                                Err(err) => {
                                    error!(
                                        ?node,
                                        ?crtc,
                                        ?err,
                                        "Failed to complete native DRM frame"
                                    );
                                    false
                                }
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if completed {
                    render_native_scene(state, node, crtc, &mut devices);
                }
            }
            DrmEvent::Error(err) => {
                error!(?node, ?err, "Native DRM event error");
            }
        },
    ) {
        Ok(token) => token,
        Err(err) => {
            // Release dependent GPU resources before dropping the
            // underlying DRM/GBM device handles.
            drop(renderer);
            drop(egl_display);
            drop(outputs);
            drop(drm);
            drop(gbm);
            close_session_device(session, session_fd);
            return Err(Box::new(err));
        }
    };

    devices.insert(
        node,
        NativeDrmDevice {
            drm,
            _gbm: gbm,
            egl_display,
            renderer,
            wallpaper,
            outputs,
            session_fd,
            notifier_token,
        },
    );

    info!(?node, path = %path.display(), "Registered native DRM/GBM device");
    Ok(())
}

fn remove_drm_device(
    node: DrmNode,
    session: &Rc<RefCell<LibSeatSession>>,
    handle: &LoopHandle<'_, Vanta>,
    devices: &mut HashMap<DrmNode, NativeDrmDevice>,
) {
    let Some(device) = devices.remove(&node) else {
        return;
    };

    handle.remove(device.notifier_token);

    let NativeDrmDevice {
        drm,
        _gbm,
        egl_display,
        renderer,
        outputs,
        session_fd,
        notifier_token: _,
        ..
    } = device;

    // Destroy DRM compositors and surfaces before their renderer and devices.
    drop(outputs);
    drop(renderer);
    drop(egl_display);
    drop(drm);
    drop(_gbm);

    close_session_device(session, session_fd);
    info!(?node, "Removed native DRM/GBM device");
}

pub(crate) fn init_native(
    event_loop: &mut EventLoop<Vanta>,
    state: &mut Vanta,
) -> Result<NativeSession, Box<dyn Error>> {
    let (session, notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();
    let session = Rc::new(RefCell::new(session));

    info!(seat = %seat_name, "Acquired native libseat session");

    let udev_backend = UdevBackend::new(&seat_name)?;
    info!(seat = %seat_name, "Initialized native Udev backend");

    let devices: Rc<RefCell<HashMap<DrmNode, NativeDrmDevice>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let device_paths: Rc<RefCell<HashMap<DrmNode, PathBuf>>> =
        Rc::new(RefCell::new(HashMap::new()));

    let (redraw_sender, redraw_receiver) = channel::channel::<()>();
    state.native_redraw_sender = Some(redraw_sender);

    let devices_for_redraw = Rc::clone(&devices);
    event_loop
        .handle()
        .insert_source(redraw_receiver, move |event, _, state: &mut Vanta| {
            if let channel::Event::Msg(()) = event {
                let mut devices = devices_for_redraw.borrow_mut();
                let targets: Vec<(DrmNode, crtc::Handle)> = devices
                    .iter()
                    .flat_map(|(node, device)| {
                        device
                            .outputs
                            .iter()
                            .map(move |output| (*node, output.crtc))
                    })
                    .collect();

                for (node, crtc) in targets {
                    render_native_scene(state, node, crtc, &mut devices);
                }
            }
        })?;

    let session_for_events = Rc::clone(&session);
    let devices_for_events = Rc::clone(&devices);
    let paths_for_events = Rc::clone(&device_paths);
    let native_output_global: Rc<RefCell<Option<GlobalId>>> = Rc::new(RefCell::new(None));
    let output_global_for_session = Rc::clone(&native_output_global);
    let output_global_for_udev = Rc::clone(&native_output_global);
    let handle = event_loop.handle();

    event_loop
        .handle()
        .insert_source(
            notifier,
            move |event, &mut (), state: &mut Vanta| match event {
                SessionEvent::PauseSession => {
                    {
                        let mut devices = devices_for_events.borrow_mut();

                        for (node, device) in devices.iter_mut() {
                            // Drop DRM surfaces/compositors before releasing EGL resources.
                            device.outputs.clear();
                            device.renderer.take();
                            device.egl_display.take();

                            if device.drm.is_active() {
                                device.drm.pause();
                                info!(?node, "Paused native DRM device");
                            }
                        }
                    }

                    sync_native_output(
                        state,
                        &mut devices_for_events.borrow_mut(),
                        &mut output_global_for_session.borrow_mut(),
                    );
                    warn!("Native libseat session paused");
                }
                SessionEvent::ActivateSession => {
                    {
                        let mut devices = devices_for_events.borrow_mut();

                        for (node, device) in devices.iter_mut() {
                            if !device.drm.is_active() {
                                match device.drm.activate(false) {
                                    Ok(()) => {
                                        info!(?node, "Reactivated native DRM device");

                                        match initialize_native_renderer(*node, &device._gbm) {
                                            Ok((display, renderer)) => {
                                                device.egl_display = Some(display);
                                                device.renderer = Some(renderer);
                                                refresh_device_outputs(*node, device);
                                            }
                                            Err(err) => {
                                                device.renderer = None;
                                                device.egl_display = None;
                                                device.outputs.clear();
                                                error!(
                                                    ?node,
                                                    ?err,
                                                    "Failed to recreate native EGL/GLES renderer"
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        error!(
                                            ?node,
                                            ?err,
                                            "Failed to reactivate native DRM device"
                                        );
                                    }
                                }
                            }
                        }
                    }

                    // Devices added while the session was inactive have a path
                    // but no open DRM/GBM resources yet.
                    let pending_devices: Vec<(DrmNode, PathBuf)> = {
                        let paths = paths_for_events.borrow();
                        let devices = devices_for_events.borrow();

                        paths
                            .iter()
                            .filter(|(node, _)| !devices.contains_key(node))
                            .map(|(node, path)| (*node, path.clone()))
                            .collect()
                    };

                    {
                        let mut devices = devices_for_events.borrow_mut();

                        for (node, path) in pending_devices {
                            if let Err(err) = add_drm_device(
                                node,
                                &path,
                                &session_for_events,
                                &handle,
                                &devices_for_events,
                                &mut devices,
                            ) {
                                warn!(
                                    ?node,
                                    path = %path.display(),
                                    ?err,
                                    "Failed to initialize deferred native DRM device"
                                );
                            }
                        }
                    }

                    sync_native_output(
                        state,
                        &mut devices_for_events.borrow_mut(),
                        &mut output_global_for_session.borrow_mut(),
                    );

                    info!("Native libseat session activated");
                }
            },
        )?;

    let session_for_udev = Rc::clone(&session);
    let devices_for_udev = Rc::clone(&devices);
    let paths_for_udev = Rc::clone(&device_paths);
    let device_event_loop = event_loop.handle();

    // Initialize DRM devices already present when Vanta starts.
    // UdevBackend only emits events for changes after its initial snapshot.
    let initial_devices: Vec<_> = udev_backend
        .device_list()
        .map(|(device_id, path)| (device_id, path.to_path_buf()))
        .collect();

    for (device_id, path) in initial_devices {
        match DrmNode::from_dev_id(device_id) {
            Ok(node) => {
                paths_for_udev.borrow_mut().insert(node, path.clone());

                if session_for_udev.borrow().is_active() {
                    let mut devices = devices_for_udev.borrow_mut();

                    if let Err(err) = add_drm_device(
                        node,
                        &path,
                        &session_for_udev,
                        &device_event_loop,
                        &devices_for_udev,
                        &mut devices,
                    ) {
                        warn!(
                            ?node,
                            path = %path.display(),
                            ?err,
                            "Failed to initialize startup DRM device"
                        );
                    }
                } else {
                    info!(
                        ?node,
                        path = %path.display(),
                        "Deferred startup DRM device until session activation"
                    );
                }
            }
            Err(err) => {
                warn!(?device_id, ?err, "Startup device is not a usable DRM node");
            }
        }
    }

    sync_native_output(
        state,
        &mut devices_for_udev.borrow_mut(),
        &mut native_output_global.borrow_mut(),
    );

    event_loop.handle().insert_source(
        udev_backend,
        move |event, _, state: &mut Vanta| match event {
            UdevEvent::Added { device_id, path } => match DrmNode::from_dev_id(device_id) {
                Ok(node) => {
                    paths_for_udev.borrow_mut().insert(node, path.clone());

                    if session_for_udev.borrow().is_active() {
                        let mut devices = devices_for_udev.borrow_mut();

                        if let Err(err) = add_drm_device(
                            node,
                            &path,
                            &session_for_udev,
                            &device_event_loop,
                            &devices_for_udev,
                            &mut devices,
                        ) {
                            warn!(
                                ?node,
                                path = %path.display(),
                                ?err,
                                "Failed to initialize native DRM device"
                            );
                        }
                    } else {
                        info!(
                            ?node,
                            path = %path.display(),
                            "Deferred native DRM device until session activation"
                        );
                    }

                    sync_native_output(
                        state,
                        &mut devices_for_udev.borrow_mut(),
                        &mut output_global_for_udev.borrow_mut(),
                    );
                }
                Err(err) => {
                    warn!(?device_id, ?err, "Udev device is not a usable DRM node");
                }
            },
            UdevEvent::Changed { device_id } => match DrmNode::from_dev_id(device_id) {
                Ok(node) => {
                    if !paths_for_udev.borrow().contains_key(&node) {
                        tracing::debug!(?node, "Change event for unregistered DRM device");
                        return;
                    }

                    {
                        let mut devices = devices_for_udev.borrow_mut();

                        if let Some(device) = devices.get_mut(&node) {
                            if device.drm.is_active() {
                                info!(?node, "Refreshing native DRM connector topology");
                                refresh_device_outputs(node, device);
                            }
                        } else {
                            tracing::debug!(
                                ?node,
                                "DRM device is not open; skipping connector refresh"
                            );
                        }
                    }

                    sync_native_output(
                        state,
                        &mut devices_for_udev.borrow_mut(),
                        &mut output_global_for_udev.borrow_mut(),
                    );
                }
                Err(err) => {
                    warn!(?device_id, ?err, "Unable to identify changed DRM device");
                }
            },
            UdevEvent::Removed { device_id } => match DrmNode::from_dev_id(device_id) {
                Ok(node) => {
                    paths_for_udev.borrow_mut().remove(&node);
                    remove_drm_device(
                        node,
                        &session_for_udev,
                        &device_event_loop,
                        &mut devices_for_udev.borrow_mut(),
                    );
                    sync_native_output(
                        state,
                        &mut devices_for_udev.borrow_mut(),
                        &mut output_global_for_udev.borrow_mut(),
                    );
                }
                Err(err) => {
                    warn!(?device_id, ?err, "Unable to identify removed DRM device");
                }
            },
        },
    )?;

    Ok(NativeSession { _session: session })
}
