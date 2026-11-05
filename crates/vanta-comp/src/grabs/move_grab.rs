//! Move grab is the state of a composer during which the client window is being dragged around.
//!
//! eg. Usually whenever a user clicks on the app's titlebar and starts dragging, the compositors
//! enters a MoveSurfaceGrab state.

use crate::{
    layout::{calculate_snap_target, SnapTarget},
    Vanta,
};
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle},
};

const SNAP_THRESHOLD: i32 = 24;
const MINIMIZE_THRESHOLD: i32 = 48;
const MINIMIZE_PREVIEW_SIZE: (i32, i32) = (200, 80);

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<Vanta>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
    pub snap_target: Option<(SnapTarget, Rectangle<i32, Logical>)>,
    pub minimize_target: bool,
}

impl PointerGrab<Vanta> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        data.space
            .map_element(self.window.clone(), new_location.to_i32_round(), true);

        // Minimize activates when the dragged window's bottom edge enters
        // the bottom zone while the window is being moved downward.
        let minimize_preview = if delta.y > 0.0 {
            data.output
                .as_ref()
                .and_then(|output| data.space.output_geometry(output))
                .and_then(|output_geometry| {
                    data.space.element_bbox(&self.window).and_then(|window_bounds| {
                        let window_bottom = window_bounds.loc.y + window_bounds.size.h;
                        let output_bottom = output_geometry.loc.y + output_geometry.size.h;

                        (window_bottom >= output_bottom - MINIMIZE_THRESHOLD).then(|| {
                            Rectangle::new(
                                (
                                    output_bottom - MINIMIZE_PREVIEW_SIZE.0 - 16,
                                    output_bottom - MINIMIZE_PREVIEW_SIZE.1 - 16,
                                )
                                    .into(),
                                MINIMIZE_PREVIEW_SIZE.into(),
                            )
                        })
                    })
                })
        } else {
            None
        };

        self.minimize_target = minimize_preview.is_some();
        data.minimize_preview = minimize_preview;

        // Minimize takes priority; otherwise preserve normal edge snapping.
        let next_snap_target = if self.minimize_target {
            None
        } else {
            data.output
                .as_ref()
                .and_then(|output| data.space.output_geometry(output))
                .and_then(|output_geometry| {
                    data.space
                        .element_bbox(&self.window)
                        .and_then(|window_bounds| {
                            calculate_snap_target(window_bounds, output_geometry, SNAP_THRESHOLD)
                        })
                })
        };

        if self.snap_target != next_snap_target {
            tracing::info!(
                pointer_x = event.location.x,
                pointer_y = event.location.y,
                snap_target = ?next_snap_target.as_ref().map(|(target, _)| target),
                "Vanta snap target changed"
            );
        }

        self.snap_target = next_snap_target;

        // Keep the compositor preview synchronized with the current snap target.
        data.snap_preview = self.snap_target.map(|(_, rect)| rect);
    }

    fn relative_motion(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // The button is a button code as defined in the
        // Linux kernel's linux/input-event-codes.h header file, e.g. BTN_LEFT.
        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            if self.minimize_target {
                let window_id = data.windows.iter()
                    .find_map(|(id, window)| (window == &self.window).then_some(*id));

                if let Some(id) = window_id {
                    if let Err(error) = data.minimize_window(id) {
                        tracing::warn!(window_id = id, %error, "Failed to minimize dragged window");
                    }
                } else {
                    tracing::warn!("Could not resolve stable ID for dragged window");
                }
            } else if let Some((target, rect)) = self.snap_target.take() {
                // Apply the last valid snap target before ending the move grab.
                data.space.map_element(self.window.clone(), rect.loc, true);

                if let Some(toplevel) = self.window.toplevel().filter(|toplevel| toplevel.alive()) {
                    toplevel.with_pending_state(|state| {
                        state.size = Some(rect.size);

                        match target {
                            SnapTarget::Maximize => {
                                state.states.set(xdg_toplevel::State::Maximized);
                            }
                            SnapTarget::Left | SnapTarget::Right => {
                                state.states.unset(xdg_toplevel::State::Maximized);
                            }
                        }
                    });

                    toplevel.send_pending_configure();
                }
            }

            // Remove both previews before releasing the grab.
            data.snap_preview = None;
            data.minimize_preview = None;

            // No more buttons are pressed, release the grab.
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Vanta, handle: &mut PointerInnerHandle<'_, Vanta>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<Vanta> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut Vanta) {
        // Clear both previews if the grab is cancelled or replaced.
        data.snap_preview = None;
        data.minimize_preview = None;
    }
}
