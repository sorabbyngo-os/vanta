use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyboardKeyEvent, MouseButton, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::{keysyms, FilterResult},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::SERIAL_COUNTER,
};

use crate::state::Vanta;

impl Vanta {
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

            InputEvent::PointerMotion { .. } => {}

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
                let in_hot_corner = pos.x <= output_geo.loc.x as f64 + 8.0
                    && pos.y <= output_geo.loc.y as f64 + 8.0;

                if !in_hot_corner {
                    self.hot_corner_armed = true;
                } else if self.hot_corner_armed && !pointer.is_grabbed() {
                    self.hot_corner_armed = false;

                    if !self.overview_open {
                        self.overview_open = true;
                        let _ = self.clear_focus_with_serial(serial);
                    }
                }

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
                        let location = pointer.current_location();
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
                            let _ = self.switch_workspace(index);
                        } else if create_workspace {
                            let _ = self.create_workspace();
                        } else {
                            self.overview_open = false;
                        }
                    }

                    return;
                }

                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let button_state = event.state();

                if button_state == ButtonState::Pressed && !pointer.is_grabbed() {
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(window, loc)| (window.clone(), loc))
                    {
                        if let Some(id) = self
                            .windows
                            .iter()
                            .find_map(|(&id, candidate)| (candidate == &window).then_some(id))
                        {
                            let _ = self.focus_window_with_serial(id, serial);
                        }
                    } else {
                        // Clear stale pointer focus before forwarding the click.
                        // This prevents an empty-desktop click from being delivered
                        // to a previously focused client.
                        pointer.motion(
                            self,
                            None,
                            &MotionEvent {
                                location: pointer.current_location(),
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
