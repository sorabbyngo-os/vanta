use smithay::{
    desktop::Window,
    utils::{IsAlive, Logical, Point, Rectangle, Size},
};

use crate::Vanta;

type LayoutPosition = Point<i32, Logical>;
type LayoutSize = Size<i32, Logical>;

/// Window edge-snap destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapTarget {
    Left,
    Right,
    Maximize,
}

/// Calculate a snap rectangle from the pointer position and output geometry.
///
/// The top edge takes priority over the left and right edges. Pointer
/// coordinates are global logical coordinates, independent of window size.
pub fn calculate_snap_target(
    pointer: Point<f64, Logical>,
    output: Rectangle<i32, Logical>,
    threshold: i32,
) -> Option<(SnapTarget, Rectangle<i32, Logical>)> {
    let threshold = threshold.max(1);
    let side_threshold = threshold.saturating_mul(3);

    let output_left = output.loc.x as f64;
    let output_top = output.loc.y as f64;
    let output_right = output.loc.x.saturating_add(output.size.w) as f64;
    let output_bottom = output.loc.y.saturating_add(output.size.h) as f64;

    let inside_horizontal = pointer.x >= output_left && pointer.x < output_right;
    let inside_vertical = pointer.y >= output_top && pointer.y < output_bottom;

    if inside_horizontal && pointer.y <= output_top + threshold as f64 {
        return Some((SnapTarget::Maximize, output));
    }

    let gap = if output.size.w >= 48 && output.size.h >= 48 {
        12
    } else {
        0
    };

    let inner_x = output.loc.x.saturating_add(gap);
    let inner_y = output.loc.y.saturating_add(gap);
    let inner_width = output.size.w.saturating_sub(gap.saturating_mul(2)).max(1);
    let inner_height = output.size.h.saturating_sub(gap.saturating_mul(2)).max(1);
    let half_width = inner_width.saturating_sub(gap).max(1) / 2;

    if inside_vertical && pointer.x <= output_left + side_threshold as f64 {
        return Some((
            SnapTarget::Left,
            Rectangle::new(
                (inner_x, inner_y).into(),
                (half_width.max(1), inner_height).into(),
            ),
        ));
    }

    if inside_vertical && pointer.x >= output_right - side_threshold as f64 {
        let right_x = inner_x.saturating_add(half_width).saturating_add(gap);
        let right_width = inner_width
            .saturating_sub(half_width)
            .saturating_sub(gap)
            .max(1);

        return Some((
            SnapTarget::Right,
            Rectangle::new(
                (right_x, inner_y).into(),
                (right_width, inner_height).into(),
            ),
        ));
    }

    None
}

/// Calculate positions and requested sizes in stable window-ID order.
///
/// The first window is the master. Remaining windows share the right stack.
fn calculate_layout(
    window_ids: &[u64],
    origin: LayoutPosition,
    area_size: LayoutSize,
) -> Vec<(LayoutPosition, LayoutSize)> {
    if window_ids.is_empty() {
        return Vec::new();
    }

    let gap = if area_size.w >= 48 && area_size.h >= 48 {
        12
    } else {
        0
    };

    let inner_width = area_size.w.saturating_sub(2 * gap).max(1);
    let inner_height = area_size.h.saturating_sub(2 * gap).max(1);
    let origin_x = origin.x.saturating_add(gap);
    let origin_y = origin.y.saturating_add(gap);

    if window_ids.len() == 1 {
        return vec![(
            (origin_x, origin_y).into(),
            (inner_width, inner_height).into(),
        )];
    }

    let master_width = (inner_width.saturating_mul(3) / 5).max(1).min(inner_width);

    let stack_width = inner_width
        .saturating_sub(master_width)
        .saturating_sub(gap)
        .max(1);

    let stack_x = origin_x.saturating_add(master_width).saturating_add(gap);

    let stack_count = window_ids.len() - 1;
    let stack_count_i32 = i32::try_from(stack_count).unwrap_or(i32::MAX);

    let available_stack_height = inner_height
        .saturating_sub(gap.saturating_mul(stack_count_i32.saturating_sub(1)))
        .max(1);

    let base_height = (available_stack_height / stack_count_i32).max(1);

    let mut targets = Vec::with_capacity(window_ids.len());

    targets.push((
        (origin_x, origin_y).into(),
        (master_width, inner_height).into(),
    ));

    for stack_index in 0..stack_count {
        let stack_index_i32 = i32::try_from(stack_index).unwrap_or(i32::MAX);

        let y = origin_y
            .saturating_add(stack_index_i32.saturating_mul(base_height.saturating_add(gap)));

        let height = if stack_index + 1 == stack_count {
            origin_y
                .saturating_add(inner_height)
                .saturating_sub(y)
                .max(1)
        } else {
            base_height
        };

        targets.push(((stack_x, y).into(), (stack_width, height).into()));
    }

    targets
}

impl Vanta {
    /// Toggle a mapped window between tiled and floating layouts.
    pub fn toggle_float(&mut self, id: u64) -> Result<(), String> {
        let window = self
            .windows
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("Unknown window ID: {id}"))?;

        if !window.alive() || !self.space.elements().any(|mapped| mapped == &window) {
            return Err(format!("Window {id} is not currently mapped"));
        }

        if !self.floating_windows.remove(&id) {
            // The window was tiled; switch it to floating.
            self.floating_windows.insert(id);
        }

        // Invalidate the cached size in either direction.
        self.layout_sizes.remove(&id);

        self.relayout_windows();
        self.raise_window_with_policy(&window);
        Ok(())
    }

    /// Mark a window as floating when the user starts a move or resize operation.
    pub fn make_window_floating(&mut self, window: &Window) {
        let Some(id) = self
            .windows
            .iter()
            .find_map(|(&id, candidate)| (candidate == window).then_some(id))
        else {
            return;
        };

        self.floating_windows.insert(id);
        self.layout_sizes.remove(&id);
        self.relayout_windows();
        self.raise_window_with_policy(window);
    }

    /// Remove stale layout metadata after a toplevel is destroyed.
    pub fn prune_layout_state(&mut self) {
        self.floating_windows
            .retain(|id| self.windows.contains_key(id));
        self.layout_sizes
            .retain(|id, _| self.windows.contains_key(id));
    }

    /// Recalculate the single-output master-and-stack layout.
    ///
    /// Client sizes are requested through XDG configure events. Vanta does not
    /// assume that a client has accepted or committed a requested size.
    pub fn relayout_windows(&mut self) {
        let Some(output) = self.output.clone() else {
            return;
        };

        let Some(area) = self.space.output_geometry(&output) else {
            return;
        };

        let mut tiled: Vec<(u64, Window)> = self
            .windows
            .iter()
            .filter_map(|(&id, window)| {
                if self.floating_windows.contains(&id)
                    || !window.alive()
                    || !self.space.elements().any(|mapped| mapped == window)
                {
                    return None;
                }

                Some((id, window.clone()))
            })
            .collect();

        tiled.sort_by_key(|(id, _)| *id);

        if tiled.is_empty() {
            return;
        }

        let window_ids: Vec<u64> = tiled.iter().map(|(id, _)| *id).collect();
        let targets = calculate_layout(&window_ids, area.loc, area.size);

        for ((id, window), (location, requested_size)) in tiled.into_iter().zip(targets.into_iter())
        {
            if self.space.element_location(&window) != Some(location) {
                self.space.relocate_element(&window, location);
            }

            if self.layout_sizes.get(&id) == Some(&requested_size) {
                continue;
            }

            let Some(toplevel) = window.toplevel().filter(|surface| surface.alive()) else {
                continue;
            };

            toplevel.with_pending_state(|state| {
                state.size = Some(requested_size);
            });
            toplevel.send_pending_configure();

            self.layout_sizes.insert(id, requested_size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_at_left_edge_produces_left_half_snap() {
        let output = Rectangle::new((0, 0).into(), (1920, 1080).into());
        let (target, rect) = calculate_snap_target((60.0, 500.0).into(), output, 24).unwrap();

        assert_eq!(target, SnapTarget::Left);
        assert_eq!(rect.loc, (12, 12).into());
        assert_eq!(rect.size, (942, 1056).into());
    }

    #[test]
    fn pointer_at_right_edge_produces_right_half_snap() {
        let output = Rectangle::new((0, 0).into(), (1920, 1080).into());
        let (target, rect) = calculate_snap_target((1860.0, 500.0).into(), output, 24).unwrap();

        assert_eq!(target, SnapTarget::Right);
        assert_eq!(rect.loc, (966, 12).into());
        assert_eq!(rect.size, (942, 1056).into());
    }

    #[test]
    fn pointer_at_top_edge_produces_maximize_target() {
        let output = Rectangle::new((0, 0).into(), (1920, 1080).into());
        let (target, rect) = calculate_snap_target((960.0, 20.0).into(), output, 24).unwrap();

        assert_eq!(target, SnapTarget::Maximize);
        assert_eq!(rect, output);
    }

    #[test]
    fn pointer_at_top_left_prefers_maximize() {
        let output = Rectangle::new((0, 0).into(), (1920, 1080).into());
        let (target, _) = calculate_snap_target((10.0, 10.0).into(), output, 24).unwrap();

        assert_eq!(target, SnapTarget::Maximize);
    }

    #[test]
    fn pointer_away_from_snap_edges_has_no_target() {
        let output = Rectangle::new((0, 0).into(), (1920, 1080).into());
        assert!(calculate_snap_target((960.0, 500.0).into(), output, 24).is_none());
    }

    #[test]
    fn empty_layout_has_no_targets() {
        let targets = calculate_layout(&[], (0, 0).into(), (1920, 1080).into());

        assert!(targets.is_empty());
    }

    #[test]
    fn single_window_uses_entire_output_inside_gap() {
        let targets = calculate_layout(&[1], (0, 0).into(), (1920, 1080).into());

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, (12, 12).into());
        assert_eq!(targets[0].1, (1896, 1056).into());
    }

    #[test]
    fn two_windows_use_master_and_full_height_stack() {
        let targets = calculate_layout(&[1, 2], (0, 0).into(), (1920, 1080).into());

        assert_eq!(targets.len(), 2);

        assert_eq!(targets[0].0, (12, 12).into());
        assert_eq!(targets[0].1, (1137, 1056).into());

        assert_eq!(targets[1].0, (1161, 12).into());
        assert_eq!(targets[1].1, (747, 1056).into());
    }

    #[test]
    fn three_windows_split_the_stack_evenly() {
        let targets = calculate_layout(&[1, 2, 3], (0, 0).into(), (1920, 1080).into());

        assert_eq!(targets.len(), 3);

        assert_eq!(targets[0].0, (12, 12).into());
        assert_eq!(targets[0].1, (1137, 1056).into());

        assert_eq!(targets[1].0, (1161, 12).into());
        assert_eq!(targets[1].1, (747, 522).into());

        assert_eq!(targets[2].0, (1161, 546).into());
        assert_eq!(targets[2].1, (747, 522).into());
    }

    #[test]
    fn layout_respects_nonzero_output_origin() {
        let targets = calculate_layout(&[1], (100, 50).into(), (1280, 720).into());

        assert_eq!(targets[0].0, (112, 62).into());
        assert_eq!(targets[0].1, (1256, 696).into());
    }
}
