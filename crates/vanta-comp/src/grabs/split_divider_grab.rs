use crate::{layout_engine::SplitId, Vanta};
use smithay::{
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};
use vanta_ipc::SplitAxis;

pub struct SplitDividerGrab {
    start_data: PointerGrabStartData<Vanta>,
    split_id: SplitId,
    axis: SplitAxis,
}

impl SplitDividerGrab {
    pub fn new(
        start_data: PointerGrabStartData<Vanta>,
        split_id: SplitId,
        axis: SplitAxis,
    ) -> Self {
        Self {
            start_data,
            split_id,
            axis,
        }
    }

    fn update_ratio(&self, data: &mut Vanta, location: Point<f64, Logical>) {
        if data.layout_mode != vanta_ipc::LayoutMode::Fractional {
            return;
        }

        let Some((bounds, gap, min_size)) = data.split_edit_bounds() else {
            return;
        };

        let Some(tree) = data.workspace_layouts.get(&data.active_workspace) else {
            return;
        };

        let Ok(dividers) = tree.dividers(bounds, gap, min_size) else {
            return;
        };

        let Some(divider) = dividers.iter().find(|divider| divider.id == self.split_id) else {
            return;
        };

        let parent = divider.parent_bounds;
        let ratio = match self.axis {
            SplitAxis::LeftRight => {
                let usable = parent.width.saturating_sub(gap);
                if usable <= 0 {
                    return;
                }
                (location.x - f64::from(parent.x)) / f64::from(usable)
            }
            SplitAxis::TopBottom => {
                let usable = parent.height.saturating_sub(gap);
                if usable <= 0 {
                    return;
                }
                (location.y - f64::from(parent.y)) / f64::from(usable)
            }
        };

        if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
            return;
        }

        if let Some(tree) = data.workspace_layouts.get_mut(&data.active_workspace) {
            if tree
                .set_split_ratio_by_id(self.split_id, ratio, bounds, gap, min_size)
                .is_ok()
            {
                data.relayout_windows();
            }
        }
    }
}

impl PointerGrab<Vanta> for SplitDividerGrab {
    fn motion(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        self.update_ratio(data, event.location);
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

        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
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
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &PointerGrabStartData<Vanta> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Vanta) {}
}
