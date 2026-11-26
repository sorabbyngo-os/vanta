use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent,
        GestureEndEvent, GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend,
        InputEvent, KeyboardKeyEvent, MouseButton, PointerAxisEvent, PointerButtonEvent,
        PointerMotionEvent, TouchEvent,
    },
    input::{
        keyboard::{keysyms, FilterResult},
        pointer::{AxisFrame, ButtonEvent, Focus, MotionEvent, RelativeMotionEvent},
        touch::{DownEvent as TouchDown, MotionEvent as TouchMotion, UpEvent as TouchUp},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::grabs::{SplitCornerGrab, SplitDividerGrab};
use crate::state::{TouchGesture, Vanta};

impl Vanta {
    fn select_overview_at(&mut self, location: Point<f64, Logical>) {
        let mut selected_workspace = None;
        let mut create_workspace = false;

        if let Some(layout) = self.overview_layout() {
            if let Some((index, _)) = layout.thumbnails.iter().find(|(_, rect)| {
                location.x >= rect.loc.x as f64
                    && location.y >= rect.loc.y as f64
                    && location.x < (rect.loc.x + rect.size.w) as f64
                    && location.y < (rect.loc.y + rect.size.h) as f64
            }) {
                selected_workspace = Some(*index);
            } else {
                let rect = layout.new_workspace;
                create_workspace = location.x >= rect.loc.x as f64
                    && location.y >= rect.loc.y as f64
                    && location.x < (rect.loc.x + rect.size.w) as f64
                    && location.y < (rect.loc.y + rect.size.h) as f64;
            }
        }

        if let Some(index) = selected_workspace {
            if self.switch_workspace(index).is_ok() {
                self.overview_open = false;
            }
        } else if create_workspace {
            if self.create_workspace().is_ok() {
                self.overview_open = false;
            }
        } else {
            self.overview_open = false;
        }
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let Some(keyboard) = self.seat.get_keyboard() else {
                    return;
                };

                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time(&event);
                let key_state = event.state();

                keyboard.input::<(), _>(
                    self,
                    event.key_code(),
                    key_state,
                    serial,
                    time,
                    |state, _, key| {
                        if state.overview_open
                            && key_state == smithay::backend::input::KeyState::Pressed
                            && key.modified_sym() == keysyms::KEY_Escape.into()
                        {
                            state.overview_open = false;
                            FilterResult::Intercept(())
                        } else {
                            FilterResult::Forward
                        }
                    },
                );
            }

            InputEvent::PointerMotion { event, .. } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };

                let Some(output_geo) = self.space.output_geometry(output) else {
                    return;
                };

                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };

                let delta = event.delta();
                let delta_unaccel = event.delta_unaccel();
                let current = pointer.current_location();

                let max_x = (output_geo.loc.x + output_geo.size.w - 1) as f64;
                let max_y = (output_geo.loc.y + output_geo.size.h - 1) as f64;

                let pos: Point<f64, Logical> = (
                    (current.x + delta.x).clamp(output_geo.loc.x as f64, max_x),
                    (current.y + delta.y).clamp(output_geo.loc.y as f64, max_y),
                )
                    .into();

                let serial = SERIAL_COUNTER.next_serial();

                // Evaluate the overview hot corner for relative mouse motion.
                let local_x = pos.x - output_geo.loc.x as f64;
                let local_y = pos.y - output_geo.loc.y as f64;
                let previous = self.last_pointer_position;
                let moving_toward_corner = previous
                    .map(|old| pos.x < old.x && pos.y < old.y)
                    .unwrap_or(false);

                let in_hint_zone =
                    (0.0..=88.0).contains(&local_x) && (0.0..=88.0).contains(&local_y);
                let in_hot_corner = local_x >= 0.0 && local_y >= 0.0 && local_x + local_y <= 24.0;

                if !in_hint_zone {
                    self.hot_corner_hint_visible = false;
                } else if moving_toward_corner {
                    self.hot_corner_hint_visible = true;
                }

                if !in_hot_corner {
                    self.hot_corner_armed = true;
                } else if self.hot_corner_armed && moving_toward_corner && !pointer.is_grabbed() {
                    self.hot_corner_armed = false;
                    self.overview_open = !self.overview_open;

                    if self.overview_open {
                        let _ = self.clear_focus_with_serial(serial);
                    }
                }

                let under = if self.overview_open {
                    None
                } else {
                    self.surface_under(pos)
                };

                pointer.motion(
                    self,
                    under.clone(),
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time(),
                    },
                );

                pointer.relative_motion(
                    self,
                    under,
                    &RelativeMotionEvent {
                        delta,
                        delta_unaccel,
                        time: event.time(),
                    },
                );

                self.last_pointer_position = Some(pos);
                pointer.frame(self);
            }

            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };

                let Some(output_geo) = self.space.output_geometry(output) else {
                    return;
                };

                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };

                let serial = SERIAL_COUNTER.next_serial();
                let local_x = pos.x - output_geo.loc.x as f64;
                let local_y = pos.y - output_geo.loc.y as f64;
                let previous = self.last_pointer_position;
                let moving_toward_corner = previous
                    .map(|old| pos.x < old.x && pos.y < old.y)
                    .unwrap_or(false);

                let in_hint_zone =
                    (0.0..=88.0).contains(&local_x) && (0.0..=88.0).contains(&local_y);
                let in_hot_corner = local_x >= 0.0 && local_y >= 0.0 && local_x + local_y <= 24.0;

                if !in_hint_zone {
                    self.hot_corner_hint_visible = false;
                } else if moving_toward_corner {
                    self.hot_corner_hint_visible = true;
                }

                if !in_hot_corner {
                    self.hot_corner_armed = true;
                } else if self.hot_corner_armed && moving_toward_corner && !pointer.is_grabbed() {
                    self.hot_corner_armed = false;
                    self.overview_open = !self.overview_open;

                    if self.overview_open {
                        let _ = self.clear_focus_with_serial(serial);
                    }
                }

                self.last_pointer_position = Some(pos);

                if self.overview_open {
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: pos,
                            serial,
                            time: event.time(),
                        },
                    );
                    pointer.frame(self);
                    return;
                }

                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time(),
                    },
                );
                pointer.frame(self);
            }

            InputEvent::PointerButton { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };

                if self.overview_open {
                    if event.state() == ButtonState::Pressed
                        && event.button() == Some(MouseButton::Left)
                        && !pointer.is_grabbed()
                    {
                        self.select_overview_at(pointer.current_location());
                    }

                    return;
                }

                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let button_state = event.state();

                // Fractional-mode split dividers are interactive only in empty
                // compositor space. Never intercept clicks over mapped surfaces.
                if button_state == ButtonState::Pressed
                    && event.button() == Some(MouseButton::Left)
                    && !pointer.is_grabbed()
                    && self.layout_mode == vanta_ipc::LayoutMode::Fractional
                {
                    let pointer_location = pointer.current_location();

                    if self.surface_under(pointer_location).is_none() {
                        if let Some((bounds, gap, min_size)) = self.split_edit_bounds() {
                            if let Some(tree) = self.workspace_layouts.get(&self.active_workspace) {
                                // Junction handles take priority over individual dividers.
                                if let Ok(corners) = tree.corners(bounds, gap, min_size) {
                                    let location = pointer_location.to_i32_round::<i32>();

                                    if let Some(corner) = corners.iter().find(|corner| {
                                        location.x >= corner.rect.x
                                            && location.y >= corner.rect.y
                                            && location.x
                                                < corner.rect.x.saturating_add(corner.rect.width)
                                            && location.y
                                                < corner.rect.y.saturating_add(corner.rect.height)
                                    }) {
                                        if let Some(start_data) = pointer.grab_start_data() {
                                            let grab = SplitCornerGrab::new(
                                                start_data,
                                                corner.outer_id,
                                                corner.inner_id,
                                                corner.outer_axis,
                                                corner.inner_axis,
                                            );
                                            pointer.set_grab(self, grab, serial, Focus::Clear);
                                            return;
                                        }
                                    }
                                }

                                if let Ok(dividers) = tree.dividers(bounds, gap, min_size) {
                                    let location = pointer_location.to_i32_round::<i32>();

                                    if let Some(divider) = dividers.iter().find(|divider| {
                                        location.x >= divider.rect.x
                                            && location.y >= divider.rect.y
                                            && location.x
                                                < divider.rect.x.saturating_add(divider.rect.width)
                                            && location.y
                                                < divider.rect.y.saturating_add(divider.rect.height)
                                    }) {
                                        if let Some(start_data) = pointer.grab_start_data() {
                                            let grab = SplitDividerGrab::new(
                                                start_data,
                                                divider.id,
                                                divider.axis,
                                            );
                                            pointer.set_grab(self, grab, serial, Focus::Clear);
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if button_state == ButtonState::Pressed && !pointer.is_grabbed() {
                    let pointer_location = pointer.current_location();
                    // Dismiss Quick Settings when clicking outside it without consuming the click.
                    if let Some(output) = self.space.outputs().find(|output| {
                        self.space.output_geometry(output).is_some_and(|geometry| {
                            geometry.contains(pointer_location.to_i32_round())
                        })
                    }) {
                        if let Some(output_geometry) = self.space.output_geometry(output) {
                            let output_pos = pointer_location - output_geometry.loc.to_f64();
                            let layers = smithay::desktop::layer_map_for_output(output);

                            let quick_settings = layers
                                .layers()
                                .find(|layer| layer.namespace() == "vanta-quick-settings")
                                .cloned();

                            if let Some(quick_settings) = quick_settings {
                                let inside = layers.layer_geometry(&quick_settings).is_some_and(
                                    |geometry| geometry.contains(output_pos.to_i32_round()),
                                );

                                if !inside {
                                    quick_settings.layer_surface().send_close();
                                }
                            }
                        }
                    }

                    let focused_window = self.space.elements().rev().find_map(|window| {
                        let location = self.space.element_location(window)?;
                        let bbox = self.space.element_bbox(window)?;

                        let id = self
                            .windows
                            .iter()
                            .find_map(|(&id, candidate)| (candidate == window).then_some(id));

                        let (visual_x, visual_y) = id
                            .and_then(|id| self.motion.window(id))
                            .map(|motion| motion.position())
                            .unwrap_or((f64::from(location.x), f64::from(location.y)));

                        let delta_x = visual_x - f64::from(location.x);
                        let delta_y = visual_y - f64::from(location.y);

                        let mut visual_bbox = bbox.to_f64();
                        visual_bbox.loc.x += delta_x;
                        visual_bbox.loc.y += delta_y;

                        visual_bbox
                            .contains(pointer_location)
                            .then(|| window.clone())
                    });

                    if let Some(window) = focused_window {
                        if let Some(id) = self
                            .windows
                            .iter()
                            .find_map(|(&id, candidate)| (candidate == &window).then_some(id))
                        {
                            let _ = self.focus_window_with_serial(id, serial);
                        }
                    } else if self.surface_under(pointer_location).is_none() {
                        // Clear focus only when no layer-shell or app surface is beneath
                        // the pointer. Preserve focus for panel and dock clicks.
                        pointer.motion(
                            self,
                            None,
                            &MotionEvent {
                                location: pointer_location,
                                serial: SERIAL_COUNTER.next_serial(),
                                time: event.time(),
                            },
                        );

                        let _ = self.clear_focus_with_serial(serial);
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time(),
                    },
                );
            }

            InputEvent::GestureSwipeBegin { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    let normalized = smithay::input::pointer::GestureSwipeBeginEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: Event::time(&event),
                        fingers: event.fingers(),
                    };
                    pointer.gesture_swipe_begin(self, &normalized);
                }
            }

            InputEvent::GestureSwipeUpdate { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    let normalized = smithay::input::pointer::GestureSwipeUpdateEvent {
                        time: Event::time(&event),
                        delta: event.delta(),
                    };
                    pointer.gesture_swipe_update(self, &normalized);
                }
            }

            InputEvent::GestureSwipeEnd { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    let normalized = smithay::input::pointer::GestureSwipeEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: Event::time(&event),
                        cancelled: event.cancelled(),
                    };
                    pointer.gesture_swipe_end(self, &normalized);
                }
            }

            InputEvent::GesturePinchBegin { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    let normalized = smithay::input::pointer::GesturePinchBeginEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: Event::time(&event),
                        fingers: event.fingers(),
                    };
                    pointer.gesture_pinch_begin(self, &normalized);
                }
            }

            InputEvent::GesturePinchUpdate { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    let normalized = smithay::input::pointer::GesturePinchUpdateEvent {
                        time: Event::time(&event),
                        delta: event.delta(),
                        scale: event.scale(),
                        rotation: event.rotation(),
                    };
                    pointer.gesture_pinch_update(self, &normalized);
                }
            }

            InputEvent::GesturePinchEnd { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    let normalized = smithay::input::pointer::GesturePinchEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: Event::time(&event),
                        cancelled: event.cancelled(),
                    };
                    pointer.gesture_pinch_end(self, &normalized);
                }
            }

            InputEvent::GestureHoldBegin { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    let normalized = smithay::input::pointer::GestureHoldBeginEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: Event::time(&event),
                        fingers: event.fingers(),
                    };
                    pointer.gesture_hold_begin(self, &normalized);
                }
            }

            InputEvent::GestureHoldEnd { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    let normalized = smithay::input::pointer::GestureHoldEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: Event::time(&event),
                        cancelled: event.cancelled(),
                    };
                    pointer.gesture_hold_end(self, &normalized);
                }
            }

            InputEvent::TouchDown { event, .. } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };
                let Some(output_geo) = self.space.output_geometry(output) else {
                    return;
                };

                let position =
                    event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                let slot = event.slot();
                let slot_id = i32::from(slot);

                // A compositor-owned gesture reserves its touch sequence at
                // touch-down. Do not steal an application sequence mid-stream.
                if self.touch_gesture.is_some() {
                    return;
                }

                let local_x = position.x - output_geo.loc.x as f64;
                let local_y = position.y - output_geo.loc.y as f64;
                let starts_in_gesture_corner =
                    local_x >= 0.0 && local_y >= 0.0 && local_x + local_y <= 64.0;

                if self.app_touch_slots.is_empty()
                    && (self.overview_open || starts_in_gesture_corner)
                {
                    self.touch_gesture = Some(TouchGesture {
                        slot: slot_id,
                        start: position,
                        current: position,
                    });
                    return;
                }

                let Some(touch) = self.seat.get_touch() else {
                    return;
                };

                let focus = self.surface_under(position);
                touch.down(
                    self,
                    focus,
                    &TouchDown {
                        slot,
                        location: position,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time(),
                    },
                );
                touch.frame(self);
                self.app_touch_slots.insert(slot_id);
            }

            InputEvent::TouchMotion { event, .. } => {
                let slot = event.slot();
                let slot_id = i32::from(slot);
                let Some(output) = self.space.outputs().next() else {
                    return;
                };
                let Some(output_geo) = self.space.output_geometry(output) else {
                    return;
                };
                let position =
                    event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                if let Some(gesture) = self.touch_gesture.as_mut() {
                    if gesture.slot == slot_id {
                        gesture.current = position;
                    }
                    return;
                }

                if self.app_touch_slots.contains(&slot_id) {
                    if let Some(touch) = self.seat.get_touch() {
                        touch.motion(
                            self,
                            self.surface_under(position),
                            &TouchMotion {
                                slot,
                                location: position,
                                time: event.time(),
                            },
                        );
                        touch.frame(self);
                    }
                }
            }

            InputEvent::TouchUp { event, .. } => {
                let slot = event.slot();
                let slot_id = i32::from(slot);

                if self.touch_gesture.as_ref().map(|gesture| gesture.slot) == Some(slot_id) {
                    let Some(gesture) = self.touch_gesture.take() else {
                        return;
                    };

                    let dx = gesture.current.x - gesture.start.x;
                    let dy = gesture.current.y - gesture.start.y;
                    let moved_sq = dx * dx + dy * dy;

                    if self.overview_open && moved_sq <= 144.0 {
                        self.select_overview_at(gesture.current);
                    } else {
                        let Some(output) = self.space.outputs().next() else {
                            return;
                        };
                        let Some(output_geo) = self.space.output_geometry(output) else {
                            return;
                        };

                        let start_x = gesture.start.x - output_geo.loc.x as f64;
                        let start_y = gesture.start.y - output_geo.loc.y as f64;
                        let from_top_left_edge =
                            start_x >= 0.0 && start_y >= 0.0 && start_x + start_y <= 64.0;
                        let inward_swipe = dx >= 56.0 && dy >= 56.0;

                        if from_top_left_edge && inward_swipe {
                            self.overview_open = !self.overview_open;
                            if self.overview_open {
                                let serial = SERIAL_COUNTER.next_serial();
                                let _ = self.clear_focus_with_serial(serial);
                            }
                        }
                    }
                    return;
                }

                if self.app_touch_slots.remove(&slot_id) {
                    if let Some(touch) = self.seat.get_touch() {
                        touch.up(
                            self,
                            &TouchUp {
                                slot,
                                serial: SERIAL_COUNTER.next_serial(),
                                time: event.time(),
                            },
                        );
                        touch.frame(self);
                    }
                }
            }

            InputEvent::TouchCancel { event, .. } => {
                let slot = event.slot();
                let slot_id = i32::from(slot);

                if self.touch_gesture.as_ref().map(|gesture| gesture.slot) == Some(slot_id) {
                    self.touch_gesture = None;
                } else if self.app_touch_slots.remove(&slot_id) {
                    // This backend reports cancellation per finger. Wayland's
                    // wl_touch.cancel is session-wide, so end only this point.
                    if let Some(touch) = self.seat.get_touch() {
                        touch.up(
                            self,
                            &TouchUp {
                                slot,
                                serial: SERIAL_COUNTER.next_serial(),
                                time: event.time(),
                            },
                        );
                        touch.frame(self);
                    }
                }
            }

            InputEvent::TouchFrame { .. } => {
                if !self.app_touch_slots.is_empty() {
                    if let Some(touch) = self.seat.get_touch() {
                        touch.frame(self);
                    }
                }
            }

            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.0
                });

                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0
                });

                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time()).source(source);

                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);

                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }

                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);

                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }

                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };

                pointer.axis(self, frame);
                pointer.frame(self);
            }

            _ => {}
        }
    }
}
