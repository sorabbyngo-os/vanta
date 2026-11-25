use std::collections::HashMap;

const MAX_FRAME_DELTA: f64 = 0.05;
const MAX_SUBSTEP: f64 = 1.0 / 120.0;
const SETTLE_POSITION: f64 = 0.01;
const SETTLE_VELOCITY: f64 = 0.01;

#[derive(Debug, Clone, Copy)]
struct Spring {
    value: f64,
    velocity: f64,
    target: f64,
    stiffness: f64,
    damping: f64,
}

impl Spring {
    fn new(value: f64) -> Self {
        Self::with_parameters(value, 240.0, 28.0)
    }

    fn with_parameters(value: f64, stiffness: f64, damping: f64) -> Self {
        Self {
            value,
            velocity: 0.0,
            target: value,
            stiffness: if stiffness.is_finite() && stiffness > 0.0 {
                stiffness
            } else {
                240.0
            },
            damping: if damping.is_finite() && damping > 0.0 {
                damping
            } else {
                28.0
            },
        }
    }

    fn retarget(&mut self, target: f64) {
        self.target = target;
    }

    fn snap_to(&mut self, value: f64) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    fn advance(&mut self, delta: f64) -> bool {
        if !delta.is_finite() || delta <= 0.0 {
            return !self.is_settled();
        }

        let delta = delta.min(MAX_FRAME_DELTA);

        let steps = (delta / MAX_SUBSTEP).ceil().max(1.0) as usize;
        let step = delta / steps as f64;

        for _ in 0..steps {
            let displacement = self.target - self.value;
            let acceleration = displacement * self.stiffness - self.velocity * self.damping;

            self.velocity += acceleration * step;
            self.value += self.velocity * step;
        }

        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }

    fn is_settled(&self) -> bool {
        (self.target - self.value).abs() < SETTLE_POSITION && self.velocity.abs() < SETTLE_VELOCITY
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WindowMotion {
    x: Spring,
    y: Spring,
    scale: Spring,
    opacity: Spring,
}

impl WindowMotion {
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x: Spring::new(x),
            y: Spring::new(y),
            scale: Spring::new(1.0),
            opacity: Spring::with_parameters(1.0, 180.0, 24.0),
        }
    }

    pub fn retarget(&mut self, x: f64, y: f64, scale: f64, opacity: f64) {
        self.x.retarget(x);
        self.y.retarget(y);
        self.scale.retarget(scale.clamp(0.01, 4.0));
        self.opacity.retarget(opacity.clamp(0.0, 1.0));
    }

    pub fn advance(&mut self, delta: f64) -> bool {
        let moving = self.x.advance(delta);
        let moving = self.y.advance(delta) || moving;
        let moving = self.scale.advance(delta) || moving;
        self.opacity.advance(delta) || moving
    }

    pub fn snap_position(&mut self, x: f64, y: f64) {
        self.x.snap_to(x);
        self.y.snap_to(y);
    }

    pub fn snap_scale(&mut self, scale: f64) {
        self.scale.snap_to(scale.clamp(0.01, 4.0));
    }

    pub fn snap_opacity(&mut self, opacity: f64) {
        self.opacity.snap_to(opacity.clamp(0.0, 1.0));
    }

    pub fn position(&self) -> (f64, f64) {
        (self.x.value, self.y.value)
    }

    pub fn scale(&self) -> f64 {
        self.scale.value
    }

    pub fn opacity(&self) -> f64 {
        self.opacity.value
    }

    pub fn is_settled(&self) -> bool {
        self.x.is_settled()
            && self.y.is_settled()
            && self.scale.is_settled()
            && self.opacity.is_settled()
    }
}

#[derive(Debug, Default)]
pub struct MotionEngine {
    windows: HashMap<u64, WindowMotion>,
}

impl MotionEngine {
    pub fn insert_window(&mut self, id: u64, x: f64, y: f64) {
        self.windows.insert(id, WindowMotion::new(x, y));
    }

    pub fn animate_window(&mut self, id: u64, x: f64, y: f64, scale: f64, opacity: f64) -> bool {
        let Some(motion) = self.windows.get_mut(&id) else {
            return false;
        };

        motion.retarget(x, y, scale, opacity);
        true
    }

    pub fn snap_window_position(&mut self, id: u64, x: f64, y: f64) -> bool {
        let Some(motion) = self.windows.get_mut(&id) else {
            return false;
        };

        motion.snap_position(x, y);
        true
    }

    pub fn snap_window_scale(&mut self, id: u64, scale: f64) -> bool {
        let Some(motion) = self.windows.get_mut(&id) else {
            return false;
        };
        motion.snap_scale(scale);
        true
    }

    pub fn set_window_opacity(&mut self, id: u64, opacity: f64) -> bool {
        let Some(motion) = self.windows.get_mut(&id) else {
            return false;
        };

        motion.snap_opacity(opacity);
        true
    }

    pub fn advance(&mut self, delta: f64) -> bool {
        let mut active = false;

        for motion in self.windows.values_mut() {
            active |= motion.advance(delta);
        }

        active
    }

    pub fn window(&self, id: u64) -> Option<&WindowMotion> {
        self.windows.get(&id)
    }

    pub fn remove_window(&mut self, id: u64) {
        self.windows.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_converges_to_target() {
        let mut spring = Spring::new(0.0);
        spring.retarget(500.0);

        for _ in 0..600 {
            spring.advance(1.0 / 60.0);
        }

        assert!((spring.value - 500.0).abs() < SETTLE_POSITION);
        assert!(spring.is_settled());
    }

    #[test]
    fn retarget_preserves_current_position_and_velocity() {
        let mut spring = Spring::new(0.0);
        spring.retarget(300.0);

        for _ in 0..8 {
            spring.advance(1.0 / 60.0);
        }

        let value_before = spring.value;
        let velocity_before = spring.velocity;

        spring.retarget(-100.0);

        assert_eq!(spring.value, value_before);
        assert_eq!(spring.velocity, velocity_before);
        assert_eq!(spring.target, -100.0);
    }

    #[test]
    fn invalid_delta_does_not_corrupt_spring_state() {
        let mut spring = Spring::new(0.0);
        spring.retarget(100.0);

        assert!(spring.advance(f64::NAN));
        assert!(spring.advance(f64::INFINITY));
        assert!(spring.advance(-1.0));
        assert_eq!(spring.value, 0.0);
        assert_eq!(spring.velocity, 0.0);
        assert_eq!(spring.target, 100.0);
    }

    #[test]
    fn invalid_spring_parameters_use_safe_defaults() {
        let spring = Spring::with_parameters(0.0, f64::NAN, -1.0);

        assert_eq!(spring.stiffness, 240.0);
        assert_eq!(spring.damping, 28.0);
    }

    #[test]
    fn large_delta_is_capped() {
        let mut capped = Spring::new(0.0);
        let mut reference = Spring::new(0.0);

        capped.retarget(100.0);
        reference.retarget(100.0);

        capped.advance(1.0);
        reference.advance(MAX_FRAME_DELTA);

        assert_eq!(capped.value, reference.value);
        assert_eq!(capped.velocity, reference.velocity);
    }

    #[test]
    fn window_motion_converges_on_all_axes() {
        let mut motion = WindowMotion::new(0.0, 0.0);
        motion.retarget(800.0, 400.0, 0.85, 0.0);

        for _ in 0..600 {
            motion.advance(1.0 / 60.0);
        }

        let (x, y) = motion.position();
        assert!((x - 800.0).abs() < SETTLE_POSITION);
        assert!((y - 400.0).abs() < SETTLE_POSITION);
        assert!((motion.scale() - 0.85).abs() < SETTLE_POSITION);
        assert!(motion.opacity().abs() < SETTLE_POSITION);
        assert!(motion.is_settled());
    }

    #[test]
    fn removing_window_stops_its_animation() {
        let mut engine = MotionEngine::default();
        engine.insert_window(7, 0.0, 0.0);

        assert!(engine.animate_window(7, 500.0, 200.0, 0.8, 0.5));
        assert!(engine.advance(1.0 / 60.0));

        engine.remove_window(7);

        assert!(engine.window(7).is_none());
        assert!(!engine.advance(1.0 / 60.0));
    }

    #[test]
    fn idle_engine_does_not_request_redraw() {
        let mut engine = MotionEngine::default();
        engine.insert_window(1, 0.0, 0.0);

        assert!(!engine.advance(1.0 / 60.0));
    }

    #[test]
    fn engine_tracks_windows_by_stable_id() {
        let mut engine = MotionEngine::default();
        engine.insert_window(42, 0.0, 0.0);

        assert!(engine.animate_window(42, 100.0, 50.0, 1.0, 1.0));
        assert!(!engine.animate_window(99, 100.0, 50.0, 1.0, 1.0));

        engine.remove_window(42);
        assert!(engine.window(42).is_none());
    }
}
