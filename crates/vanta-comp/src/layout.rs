use crate::{
    layout_engine::{MinSize, Rect},
    state::WindowTransition,
};
use smithay::{
    desktop::Window,
    utils::{IsAlive, Logical, Point, Rectangle, Size},
};
use std::collections::HashSet;

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

/// Constrain the tiled workspace to the area not occupied by snapped windows.
/// Returns `None` when both sides are reserved or no usable area remains.
fn bounds_with_snap_reservations(
    bounds: Rect,
    reserve_left: bool,
    reserve_right: bool,
) -> Option<Rect> {
    const GAP: i32 = 12;

    if reserve_left && reserve_right {
        return None;
    }

    if !reserve_left && !reserve_right {
        return (bounds.width > 0 && bounds.height > 0).then_some(bounds);
    }

    let half_width = bounds.width.saturating_sub(GAP).max(0) / 2;
    let remaining_width = bounds.width.saturating_sub(half_width).saturating_sub(GAP);

    if bounds.height <= 0 || remaining_width <= 0 || half_width <= 0 {
        return None;
    }

    if reserve_left {
        Some(Rect::new(
            bounds.x.saturating_add(half_width).saturating_add(GAP),
            bounds.y,
            remaining_width,
            bounds.height,
        ))
    } else {
        Some(Rect::new(bounds.x, bounds.y, half_width, bounds.height))
    }
}

pub(crate) fn set_window_maximized_protocol_state(window: &Window, maximized: bool) {
    if let Some(toplevel) = window.toplevel().filter(|toplevel| toplevel.alive()) {
        toplevel.with_pending_state(|state| {
            if maximized {
                state.states.set(
                    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                );
            } else {
                state.states.unset(
                    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                );
            }
        });
        toplevel.send_pending_configure();
    }
}

impl Vanta {
    /// Restore a window's geometry saved before it was maximized.
    fn restore_window_geometry(
        &mut self,
        id: u64,
        rect: Rectangle<i32, Logical>,
    ) {
        let Some(window) = self.windows.get(&id).cloned() else {
            return;
        };

        self.space.map_element(window.clone(), rect.loc, true);
        self.window_locations.insert(id, rect.loc);
        self.motion.snap_window_position(
            id,
            f64::from(rect.loc.x),
            f64::from(rect.loc.y),
        );

        if let Some(toplevel) = window.toplevel().filter(|surface| surface.alive()) {
            toplevel.with_pending_state(|state| {
                state.size = Some(rect.size);
                state.states.unset(
                    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                );
            });
            toplevel.send_pending_configure();
        }

        self.layout_sizes.remove(&id);
    }

    /// Commit a snapped window and relayout the remaining tiled windows.
    pub fn set_window_snapped(
        &mut self,
        id: u64,
        target: SnapTarget,
        rect: Rectangle<i32, Logical>,
    ) {
        let displaced_ids: Vec<u64> = self
            .snapped_windows
            .iter()
            .filter_map(|(&other_id, &current)| {
                let conflicts = match target {
                    SnapTarget::Maximize => true,
                    SnapTarget::Left => matches!(current, SnapTarget::Left | SnapTarget::Maximize),
                    SnapTarget::Right => {
                        matches!(current, SnapTarget::Right | SnapTarget::Maximize)
                    }
                };
                (other_id != id && conflicts).then_some(other_id)
            })
            .collect();

        for displaced_id in displaced_ids {
            let displaced_target = self.snapped_windows.remove(&displaced_id);
            if displaced_target == Some(SnapTarget::Maximize) {
                if let Some(saved_rect) = self.maximize_restore_rects.remove(&displaced_id) {
                    self.restore_window_geometry(displaced_id, saved_rect);
                } else if let Some(window) = self.windows.get(&displaced_id) {
                    set_window_maximized_protocol_state(window, false);
                }
            }
        }

        let Some(window) = self.windows.get(&id).cloned() else {
            return;
        };

        if target == SnapTarget::Maximize {
            if !self.maximize_restore_rects.contains_key(&id) {
                if let Some(location) = self.space.element_location(&window) {
                    self.maximize_restore_rects.insert(
                        id,
                        Rectangle::new(location, window.geometry().size),
                    );
                }
            }
        } else {
            self.maximize_restore_rects.remove(&id);
        }

        self.floating_windows.remove(&id);
        self.snapped_windows.insert(id, target);
        self.layout_sizes.remove(&id);

        self.space.map_element(window.clone(), rect.loc, true);
        self.window_locations.insert(id, rect.loc);
        self.motion.snap_window_position(
            id,
            f64::from(rect.loc.x),
            f64::from(rect.loc.y),
        );

        if let Some(toplevel) = window.toplevel().filter(|surface| surface.alive()) {
            toplevel.with_pending_state(|state| {
                state.size = Some(rect.size);
                if target == SnapTarget::Maximize {
                    state.states.set(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                    );
                } else {
                    state.states.unset(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                    );
                }
            });
            toplevel.send_pending_configure();
        }

        self.relayout_windows();
    }

    /// Toggle a mapped window between tiled and floating layouts.
    fn clear_snap_state_for_window(&mut self, id: u64) {
        let previous_target = self.snapped_windows.remove(&id);

        if previous_target == Some(SnapTarget::Maximize) {
            if let Some(saved_rect) = self.maximize_restore_rects.remove(&id) {
                self.restore_window_geometry(id, saved_rect);
            } else if let Some(window) = self.windows.get(&id) {
                set_window_maximized_protocol_state(window, false);
            }
        } else {
            self.maximize_restore_rects.remove(&id);
        }
    }

    pub fn toggle_float(&mut self, id: u64) -> Result<(), String> {
        let window = self
            .windows
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("Unknown window ID: {id}"))?;

        if !window.alive() || !self.space.elements().any(|mapped| mapped == &window) {
            return Err(format!("Window {id} is not currently mapped"));
        }

        self.clear_snap_state_for_window(id);

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

        self.clear_snap_state_for_window(id);
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
        self.snapped_windows
            .retain(|id, _| self.windows.contains_key(id));
    }

    /// Recalculate the active workspace's adaptive split-tree layout.
    ///
    /// The workspace tree is persistent: relayout changes geometry without
    /// rebuilding window topology. Client sizes are requested through XDG
    /// configure events and are not assumed to have been accepted.
    /// Recalculate the active workspace using its selected layout strategy.
    ///
    /// Adaptive mode uses a temporary projection of the persistent split tree
    /// so floating and minimized windows retain their original tree positions.
    /// MasterStack calculates geometry independently without changing topology.
    /// Floating mode leaves all window geometry under direct user control.
    pub fn relayout_windows(&mut self) {
        if self.layout_mode == vanta_ipc::LayoutMode::Floating {
            return;
        }

        let Some(output) = self.output.clone() else {
            return;
        };

        let Some(area) = self.space.output_geometry(&output) else {
            return;
        };

        let active_snaps: Vec<(u64, SnapTarget)> = self
            .snapped_windows
            .iter()
            .filter_map(|(&id, &target)| {
                let window = self.windows.get(&id)?;
                (window.alive()
                    && self.space.elements().any(|mapped| mapped == window)
                    && self.window_workspaces.get(&id) == Some(&self.active_workspace))
                .then_some((id, target))
            })
            .collect();

        // A maximized window owns the complete output until it is moved,
        // resized, or replaced by another snap target.
        if active_snaps
            .iter()
            .any(|(_, target)| *target == SnapTarget::Maximize)
        {
            return;
        }

        let reserve_left = active_snaps
            .iter()
            .any(|(_, target)| *target == SnapTarget::Left);
        let reserve_right = active_snaps
            .iter()
            .any(|(_, target)| *target == SnapTarget::Right);

        let mut tiled: Vec<(u64, Window)> = self
            .windows
            .iter()
            .filter_map(|(&id, window)| {
                if self.floating_windows.contains(&id)
                    || self.snapped_windows.contains_key(&id)
                    || self
                        .transitions
                        .get(&id)
                        .is_some_and(WindowTransition::is_minimizing)
                    || !window.alive()
                    || !self.space.elements().any(|mapped| mapped == window)
                    || self.window_workspaces.get(&id) != Some(&self.active_workspace)
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

        let margin = if area.size.w >= 48 && area.size.h >= 48 {
            12
        } else {
            0
        };

        let full_bounds = Rect::new(
            area.loc.x.saturating_add(margin),
            area.loc.y.saturating_add(margin),
            area.size.w.saturating_sub(margin.saturating_mul(2)),
            area.size.h.saturating_sub(margin.saturating_mul(2)),
        );

        let Some(bounds) = bounds_with_snap_reservations(full_bounds, reserve_left, reserve_right)
        else {
            tracing::warn!(?active_snaps, reserve_left, reserve_right, "Layout diagnostic: no usable bounds");
            return;
        };

        tracing::info!(workspace = self.active_workspace, ?active_snaps, reserve_left, reserve_right, ?bounds, tiled_ids = ?tiled.iter().map(|(id, _)| *id).collect::<Vec<_>>(), "Layout diagnostic: snap state and tiled windows");

        let gap = 12;
        let min_size = MinSize::new(320, 240);

        let focused_window = self.seat.get_keyboard().and_then(|keyboard| {
            let focused_surface = keyboard.current_focus()?;

            self.windows.iter().find_map(|(&id, window)| {
                window
                    .toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == &focused_surface)
                    .then_some(id)
            })
        });

        // Normalize both layout engines into (window ID, rectangle) pairs.
        let placements: Vec<(u64, Rect)> = match self.layout_mode {
            vanta_ipc::LayoutMode::Floating => Vec::new(),

            vanta_ipc::LayoutMode::MasterStack => {
                let origin: LayoutPosition = (bounds.x, bounds.y).into();
                let size: LayoutSize = (bounds.width, bounds.height).into();

                calculate_layout(
                    &tiled.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                    origin,
                    size,
                )
                .into_iter()
                .enumerate()
                .map(|(index, (location, size))| {
                    (
                        tiled[index].0,
                        Rect::new(location.x, location.y, size.w, size.h),
                    )
                })
                .collect()
            }

            vanta_ipc::LayoutMode::Adaptive => {
                let tiled_ids: HashSet<u64> = tiled.iter().map(|(id, _)| *id).collect();

                let excluded_ids: Vec<u64> = self
                    .windows
                    .keys()
                    .copied()
                    .filter(|id| {
                        self.window_workspaces.get(id) == Some(&self.active_workspace)
                            && !tiled_ids.contains(id)
                    })
                    .collect();

                let tree = self
                    .workspace_layouts
                    .entry(self.active_workspace)
                    .or_default();

                // Add new windows to the persistent topology. Existing nodes
                // and split ratios are retained across ordinary relayouts.
                for (id, _) in &tiled {
                    if !tree.contains(*id) {
                        if let Err(error) = tree.insert_projected(
                            *id,
                            focused_window,
                            &excluded_ids,
                            bounds,
                            gap,
                            min_size,
                        ) {
                            tracing::warn!(
                                window_id = *id,
                                ?error,
                                "Unable to insert window into adaptive layout"
                            );
                        }
                    }
                }

                // Project only currently tiled windows. Do not remove floating
                // or minimized windows from the persistent workspace tree.
                let mut projection = tree.clone();
                for id in excluded_ids {
                    projection.remove(id);
                }

                match projection.layout(bounds, gap, min_size) {
                    Ok(items) => items
                        .into_iter()
                        .map(|item| (item.window_id, item.rect))
                        .collect(),
                    Err(error) => {
                        tracing::warn!(
                            workspace = self.active_workspace,
                            ?error,
                            "Unable to calculate adaptive workspace layout"
                        );
                        Vec::new()
                    }
                }
            }
        };

        tracing::info!(workspace = self.active_workspace, placements = ?placements, "Layout diagnostic: computed placements");

        for (id, rect) in placements {
            let Some(window) = self.windows.get(&id).cloned() else {
                continue;
            };

            // Ignore stale or excluded projection entries.
            if !tiled.iter().any(|(tiled_id, _)| *tiled_id == id) {
                continue;
            }

            let location: LayoutPosition = (rect.x, rect.y).into();
            let requested_size: LayoutSize = (rect.width, rect.height).into();

            self.motion
                .animate_window(id, f64::from(location.x), f64::from(location.y), 1.0, 1.0);

            if self.space.element_location(&window) != Some(location) {
                self.space.relocate_element(&window, location);
            }

            self.window_locations.insert(id, location);

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

    #[test]
    fn left_snap_reserves_left_half_for_snapped_window() {
        let bounds = Rect::new(12, 12, 1896, 1056);
        let remaining = bounds_with_snap_reservations(bounds, true, false).unwrap();

        assert_eq!(remaining, Rect::new(966, 12, 942, 1056));
    }

    #[test]
    fn right_snap_reserves_right_half_for_snapped_window() {
        let bounds = Rect::new(12, 12, 1896, 1056);
        let remaining = bounds_with_snap_reservations(bounds, false, true).unwrap();

        assert_eq!(remaining, Rect::new(12, 12, 942, 1056));
    }

    #[test]
    fn both_side_snaps_leave_no_tiled_region() {
        let bounds = Rect::new(12, 12, 1896, 1056);

        assert!(bounds_with_snap_reservations(bounds, true, true).is_none());
    }

    #[test]
    fn adaptive_windows_reflow_into_region_after_left_snap() {
        use crate::layout_engine::SplitTree;

        const GAP: i32 = 12;
        let min_size = MinSize::new(320, 240);
        let full_bounds = Rect::new(12, 12, 1896, 1056);
        let remaining = bounds_with_snap_reservations(full_bounds, true, false).unwrap();

        let mut tree = SplitTree::new();
        tree.insert(1, None, full_bounds, GAP, min_size).unwrap();
        tree.insert(2, Some(1), full_bounds, GAP, min_size).unwrap();
        tree.insert(3, Some(1), full_bounds, GAP, min_size).unwrap();

        // Window 3 is snapped; only windows 1 and 2 should tile in the remainder.
        let mut visible = tree.clone();
        assert!(visible.remove(3));
        let placements = visible.layout(remaining, GAP, min_size).unwrap();

        assert_eq!(placements.len(), 2);
        assert_eq!(
            placements.iter().map(|p| p.window_id).collect::<Vec<_>>(),
            vec![1, 2]
        );

        assert!(placements.iter().all(|p| {
            p.rect.x >= remaining.x
                && p.rect.y >= remaining.y
                && p.rect.x + p.rect.width <= remaining.x + remaining.width
                && p.rect.y + p.rect.height <= remaining.y + remaining.height
                && p.rect.width >= min_size.width
                && p.rect.height >= min_size.height
        }));
    }

    #[test]
    fn no_snap_preserves_full_layout_bounds() {
        let bounds = Rect::new(12, 12, 1896, 1056);

        assert_eq!(
            bounds_with_snap_reservations(bounds, false, false),
            Some(bounds)
        );
    }
}
