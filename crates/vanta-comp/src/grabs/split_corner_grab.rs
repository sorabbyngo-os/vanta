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

pub struct SplitCornerGrab {
    start_data: PointerGrabStartData<Vanta>,
    outer_id: SplitId,
    inner_id: SplitId,
    outer_axis: SplitAxis,
    inner_axis: SplitAxis,
}

impl SplitCornerGrab {
    pub fn new(
        start_data: PointerGrabStartData<Vanta>,
        outer_id: SplitId,
        inner_id: SplitId,
        outer_axis: SplitAxis,
        inner_axis: SplitAxis,
    ) -> Self {
        Self {
            start_data,
            outer_id,
            inner_id,
            outer_axis,
            inner_axis,
        }
    }

    fn ratio_for_axis(
        axis: SplitAxis,
        parent: crate::layout_engine::Rect,
        gap: i32,
        location: Point<f64, Logical>,
    ) -> Option<f64> {
        let (origin, usable) = match axis {
            SplitAxis::LeftRight => (f64::from(parent.x), parent.width.saturating_sub(gap)),
            SplitAxis::TopBottom => (f64::from(parent.y), parent.height.saturating_sub(gap)),
        };

        if usable <= 0 {
            return None;
        }

        let coordinate = match axis {
            SplitAxis::LeftRight => location.x,
            SplitAxis::TopBottom => location.y,
        };

        let ratio = (coordinate - origin) / f64::from(usable);
        (ratio.is_finite() && ratio > 0.0 && ratio < 1.0).then_some(ratio)
    }

    fn update_ratios(&self, data: &mut Vanta, location: Point<f64, Logical>) {
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

        let Some(outer) = dividers.iter().find(|d| d.id == self.outer_id) else {
            return;
        };
        let Some(inner) = dividers.iter().find(|d| d.id == self.inner_id) else {
            return;
        };

        if outer.axis != self.outer_axis || inner.axis != self.inner_axis {
            return;
        }

        let Some(outer_ratio) =
            Self::ratio_for_axis(outer.axis, outer.parent_bounds, gap, location)
        else {
            return;
        };
        let Some(inner_ratio) =
            Self::ratio_for_axis(inner.axis, inner.parent_bounds, gap, location)
        else {
            return;
        };

        if let Some(tree) = data.workspace_layouts.get_mut(&data.active_workspace) {
            if tree
                .set_split_ratios_by_id(
                    (self.outer_id, outer_ratio),
                    (self.inner_id, inner_ratio),
                    bounds,
                    gap,
                    min_size,
                )
                .is_ok()
            {
                data.relayout_windows();
            }
        }
    }
}

impl PointerGrab<Vanta> for SplitCornerGrab {
    fn motion(
        &mut self,
        data: &mut Vanta,
        handle: &mut PointerInnerHandle<'_, Vanta>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        self.update_ratios(data, event.location);
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
