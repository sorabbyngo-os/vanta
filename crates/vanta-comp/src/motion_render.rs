use smithay::{
    backend::renderer::{
        element::{
            surface::WaylandSurfaceRenderElement,
            utils::{Relocate, RelocateRenderElement, RescaleRenderElement},
            AsRenderElements,
        },
        gles::GlesRenderer,
    },
    utils::{Logical, Physical, Point, Rectangle, Scale},
};

use crate::Vanta;

pub(crate) type AnimatedWindowElement =
    RelocateRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>;

/// Render mapped application windows using their current motion state.
///
/// Logical Space positions remain unchanged; only the rendered surface trees
/// receive visual translation, scale, and opacity.
pub(crate) fn render_animated_windows(
    state: &Vanta,
    renderer: &mut GlesRenderer,
    region: &Rectangle<i32, Logical>,
    scale: Scale<f64>,
    alpha: f32,
) -> Vec<AnimatedWindowElement> {
    let mut elements = Vec::new();

    for window in state.space.elements().rev() {
        let Some(bbox) = state.space.element_bbox(window) else {
            continue;
        };

        if !region.overlaps(bbox) {
            continue;
        }

        let Some(location) = state.space.element_location(window) else {
            continue;
        };

        let render_location = location - window.geometry().loc;
        let base_location = (render_location - region.loc).to_physical_precise_round(scale);

        let id = state
            .windows
            .iter()
            .find_map(|(id, candidate)| (candidate == window).then_some(*id));

        let (visual_x, visual_y, visual_scale, opacity) = id
            .and_then(|id| state.motion.window(id))
            .map(|motion| {
                let (x, y) = motion.position();
                (x, y, motion.scale(), motion.opacity())
            })
            .unwrap_or((f64::from(location.x), f64::from(location.y), 1.0, 1.0));

        let delta = Point::<f64, Logical>::from((
            visual_x - f64::from(location.x),
            visual_y - f64::from(location.y),
        ))
        .to_physical_precise_round(scale);

        let rendered = window.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
            renderer,
            base_location,
            scale,
            alpha * opacity as f32,
        );

        for element in rendered {
            let scaled = RescaleRenderElement::from_element(element, base_location, visual_scale);

            elements.push(RelocateRenderElement::from_element(
                scaled,
                delta,
                Relocate::Relative,
            ));
        }
    }

    elements
}
