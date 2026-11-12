use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyboardKeyEvent, MouseButton, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        TouchCancelEvent, TouchDownEvent, TouchEvent, TouchMotionEvent, TouchUpEvent,
    },
    input::{
        keyboard::{keysyms, FilterResult},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

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

                if button_state == ButtonState::Pressed && !pointer.is_grabbed() {
                    let pointer_location = pointer.current_location();

                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer_location)
                        .map(|(window, loc)| (window.clone(), loc))
                    {
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
                pointer.frame(self);
            }

            InputEvent::TouchDown { event, .. } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };
                let Some(output_geo) = self.space.output_geometry(output) else {
                    return;
                };

                if self.touch_gesture.is_none() {
                    let position =
                        event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                    self.touch_gesture = Some(TouchGesture {
                        slot: i32::from(event.slot()),
                        start: position,
                        current: position,
                    });
                }
            }

            InputEvent::TouchMotion { event, .. } => {
                let slot = i32::from(event.slot());
                if let Some(gesture) = self.touch_gesture.as_mut() {
                    if gesture.slot == slot {
                        let Some(output) = self.space.outputs().next() else {
                            return;
                        };
                        let Some(output_geo) = self.space.output_geometry(output) else {
                            return;
                        };
                        gesture.current =
                            event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                    }
                }
            }

            InputEvent::TouchUp { event, .. } => {
                let slot = i32::from(event.slot());
                let Some(gesture) = self.touch_gesture.take() else {
                    return;
                };
                if gesture.slot != slot {
                    self.touch_gesture = Some(gesture);
                    return;
                }

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
            }

            InputEvent::TouchCancel { event, .. } => {
                let slot = i32::from(event.slot());
                if self.touch_gesture.as_ref().map(|gesture| gesture.slot) == Some(slot) {
                    self.touch_gesture = None;
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
