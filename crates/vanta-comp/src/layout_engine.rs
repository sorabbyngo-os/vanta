//! Pure adaptive split-tree layout engine.
//!
//! This module deliberately has no Smithay or compositor dependencies.
//! It owns persistent window topology and calculates logical rectangles.

use vanta_ipc::SplitAxis;

/// Stable identity for a split node within a layout tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SplitId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinSize {
    pub width: i32,
    pub height: i32,
}

impl MinSize {
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub window_id: u64,
    pub rect: Rect,
}

/// Geometry and configuration for one persistent split divider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitDivider {
    pub id: SplitId,
    pub axis: SplitAxis,
    pub ratio: f64,
    pub parent_bounds: Rect,
    pub rect: Rect,
}

/// Geometry for resizing two perpendicular splits at their junction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitCorner {
    pub outer_id: SplitId,
    pub inner_id: SplitId,
    pub outer_axis: SplitAxis,
    pub inner_axis: SplitAxis,
    pub inner_bounds: Rect,
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Leaf(u64),
    Split {
        id: SplitId,
        axis: SplitAxis,
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SplitTree {
    root: Option<Node>,
    next_split_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    DuplicateWindow(u64),
    WindowNotFound(u64),
    NoSplitForWindow(u64),
    SplitNotFound(SplitId),
    InvalidRatio,
    InvalidBounds,
    InsufficientSpace,
}

/// Split proportions must leave positive space for both children.
#[inline]
fn is_valid_split_ratio(ratio: f64) -> bool {
    ratio.is_finite() && ratio > 0.0 && ratio < 1.0
}

impl SplitTree {
    pub fn new() -> Self {
        Self::default()
    }

    fn allocate_split_id(&mut self) -> SplitId {
        let id = SplitId(self.next_split_id);
        self.next_split_id = self.next_split_id.saturating_add(1);
        id
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn len(&self) -> usize {
        fn count(node: &Node) -> usize {
            match node {
                Node::Leaf(_) => 1,
                Node::Split { first, second, .. } => count(first) + count(second),
            }
        }

        self.root.as_ref().map_or(0, count)
    }

    pub fn contains(&self, window_id: u64) -> bool {
        fn find(node: &Node, id: u64) -> bool {
            match node {
                Node::Leaf(candidate) => *candidate == id,
                Node::Split { first, second, .. } => find(first, id) || find(second, id),
            }
        }

        self.root.as_ref().is_some_and(|root| find(root, window_id))
    }

    /// Set the ratio of the split immediately enclosing `window_id`.
    ///
    /// The ratio represents the fraction assigned to the split's first child.
    /// The operation is transactional and preserves the original tree if the
    /// resulting geometry violates minimum window sizes.
    pub fn set_split_ratio(
        &mut self,
        window_id: u64,
        ratio: f64,
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<(), LayoutError> {
        if !is_valid_split_ratio(ratio) {
            return Err(LayoutError::InvalidRatio);
        }

        if !self.contains(window_id) {
            return Err(LayoutError::WindowNotFound(window_id));
        }

        // Validate the existing layout before modifying its topology.
        self.layout(bounds, gap, min_size)?;

        fn update_parent_ratio(node: &mut Node, target_id: u64, ratio: f64) -> bool {
            let Node::Split {
                ratio: current_ratio,
                first,
                second,
                ..
            } = node
            else {
                return false;
            };

            if matches!(first.as_ref(), Node::Leaf(id) if *id == target_id)
                || matches!(second.as_ref(), Node::Leaf(id) if *id == target_id)
            {
                *current_ratio = ratio;
                return true;
            }

            update_parent_ratio(first, target_id, ratio)
                || update_parent_ratio(second, target_id, ratio)
        }

        let previous = self
            .root
            .clone()
            .expect("tree contains the requested window");

        let updated = update_parent_ratio(
            self.root
                .as_mut()
                .expect("tree contains the requested window"),
            window_id,
            ratio,
        );

        if !updated {
            return Err(LayoutError::NoSplitForWindow(window_id));
        }

        if let Err(error) = self.layout(bounds, gap, min_size) {
            self.root = Some(previous);
            return Err(error);
        }

        Ok(())
    }

    /// Set the ratio of a split using its stable split ID.
    ///
    /// The update is transactional: invalid geometry restores the original tree.
    pub fn set_split_ratio_by_id(
        &mut self,
        split_id: SplitId,
        ratio: f64,
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<(), LayoutError> {
        if !is_valid_split_ratio(ratio) {
            return Err(LayoutError::InvalidRatio);
        }

        self.layout(bounds, gap, min_size)?;

        fn update(node: &mut Node, split_id: SplitId, ratio: f64) -> bool {
            match node {
                Node::Leaf(_) => false,
                Node::Split {
                    id,
                    ratio: current_ratio,
                    first,
                    second,
                    ..
                } => {
                    if *id == split_id {
                        *current_ratio = ratio;
                        true
                    } else {
                        update(first, split_id, ratio) || update(second, split_id, ratio)
                    }
                }
            }
        }

        let previous = self.root.clone();

        let Some(root) = self.root.as_mut() else {
            return Err(LayoutError::InsufficientSpace);
        };

        if !update(root, split_id, ratio) {
            return Err(LayoutError::SplitNotFound(split_id));
        }

        if let Err(error) = self.layout(bounds, gap, min_size) {
            self.root = previous;
            return Err(error);
        }

        Ok(())
    }

    /// Atomically update two distinct split ratios.
    ///
    /// Both updates are applied to a candidate tree and validated together.
    /// The live tree is replaced only when the resulting layout is valid.
    pub fn set_split_ratios_by_id(
        &mut self,
        first: (SplitId, f64),
        second: (SplitId, f64),
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<(), LayoutError> {
        if first.0 == second.0 || !is_valid_split_ratio(first.1) || !is_valid_split_ratio(second.1)
        {
            return Err(LayoutError::InvalidRatio);
        }

        self.layout(bounds, gap, min_size)?;

        fn update(node: &mut Node, split_id: SplitId, ratio: f64) -> bool {
            match node {
                Node::Leaf(_) => false,
                Node::Split {
                    id,
                    ratio: current_ratio,
                    first,
                    second,
                    ..
                } => {
                    if *id == split_id {
                        *current_ratio = ratio;
                        true
                    } else {
                        update(first, split_id, ratio) || update(second, split_id, ratio)
                    }
                }
            }
        }

        let mut candidate = self.clone();

        let Some(root) = candidate.root.as_mut() else {
            return Err(LayoutError::SplitNotFound(first.0));
        };

        if !update(root, first.0, first.1) {
            return Err(LayoutError::SplitNotFound(first.0));
        }

        let Some(root) = candidate.root.as_mut() else {
            return Err(LayoutError::SplitNotFound(second.0));
        };

        if !update(root, second.0, second.1) {
            return Err(LayoutError::SplitNotFound(second.0));
        }

        candidate.layout(bounds, gap, min_size)?;
        *self = candidate;

        Ok(())
    }

    /// Insert a window by explicitly splitting an existing leaf.
    ///
    /// `ratio` is the fraction assigned to the existing target window.
    /// The new window becomes the second child. Unlike automatic insertion,
    /// this operation never changes the requested axis.
    pub fn insert_with_ratio(
        &mut self,
        window_id: u64,
        target_window: u64,
        axis: SplitAxis,
        ratio: f64,
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<(), LayoutError> {
        if !is_valid_split_ratio(ratio) {
            return Err(LayoutError::InvalidRatio);
        }

        if self.contains(window_id) {
            return Err(LayoutError::DuplicateWindow(window_id));
        }

        if !bounds.is_valid() {
            return Err(LayoutError::InvalidBounds);
        }

        if !self.contains(target_window) {
            return Err(LayoutError::WindowNotFound(target_window));
        }

        self.layout(bounds, gap, min_size)?;

        let mut candidate = self.clone();
        let split_id = candidate.allocate_split_id();
        let replaced = Self::split_leaf_with_ratio(
            candidate.root.as_mut().expect("target exists in tree"),
            target_window,
            window_id,
            axis,
            ratio,
            split_id,
        );

        if !replaced {
            return Err(LayoutError::WindowNotFound(target_window));
        }

        candidate.layout(bounds, gap, min_size)?;
        *self = candidate;
        Ok(())
    }

    /// Change the orientation of the split immediately enclosing a window.
    ///
    /// The operation is transactional: if the new orientation cannot satisfy
    /// the current minimum sizes, the original tree remains unchanged.
    pub fn set_split_axis(
        &mut self,
        window_id: u64,
        axis: SplitAxis,
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<(), LayoutError> {
        if !self.contains(window_id) {
            return Err(LayoutError::WindowNotFound(window_id));
        }

        self.layout(bounds, gap, min_size)?;

        let mut candidate = self.clone();

        fn update_parent_axis(node: &mut Node, target_id: u64, axis: SplitAxis) -> bool {
            let Node::Split {
                axis: current_axis,
                first,
                second,
                ..
            } = node
            else {
                return false;
            };

            if matches!(first.as_ref(), Node::Leaf(id) if *id == target_id)
                || matches!(second.as_ref(), Node::Leaf(id) if *id == target_id)
            {
                *current_axis = axis;
                return true;
            }

            update_parent_axis(first, target_id, axis)
                || update_parent_axis(second, target_id, axis)
        }

        let updated = update_parent_axis(
            candidate.root.as_mut().expect("tree contains the window"),
            window_id,
            axis,
        );

        if !updated {
            return Err(LayoutError::NoSplitForWindow(window_id));
        }

        candidate.layout(bounds, gap, min_size)?;
        *self = candidate;
        Ok(())
    }

    /// Insert a window by splitting the focused leaf.
    ///
    /// If `focused_window` is absent or is not in this tree, the most
    /// recently inserted leaf is used instead.
    ///
    /// The operation is transactional: if the resulting tree cannot fit
    /// the requested minimum sizes, the original tree is preserved.
    /// Insert a window by splitting the focused leaf.
    ///
    /// If `focused_window` is absent or is not in this tree, the most
    /// recently inserted leaf is used instead.
    ///
    /// The preferred orientation follows the focused leaf's parent split.
    /// For the first split, it follows the workspace's wider dimension.
    /// If that orientation cannot fit, the other orientation is attempted.
    /// The operation is transactional.
    pub fn insert(
        &mut self,
        window_id: u64,
        focused_window: Option<u64>,
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<(), LayoutError> {
        fn parent_axis(node: &Node, target_id: u64) -> Option<SplitAxis> {
            match node {
                Node::Leaf(_) => None,
                Node::Split {
                    axis,
                    first,
                    second,
                    ..
                } => {
                    if matches!(first.as_ref(), Node::Leaf(id) if *id == target_id)
                        || matches!(second.as_ref(), Node::Leaf(id) if *id == target_id)
                    {
                        Some(*axis)
                    } else {
                        parent_axis(first, target_id).or_else(|| parent_axis(second, target_id))
                    }
                }
            }
        }

        if self.contains(window_id) {
            return Err(LayoutError::DuplicateWindow(window_id));
        }

        if !bounds.is_valid() {
            return Err(LayoutError::InvalidBounds);
        }

        let Some(root) = self.root.as_ref() else {
            self.root = Some(Node::Leaf(window_id));
            return Ok(());
        };

        let current = self.layout(bounds, gap, min_size)?;
        let target_id = focused_window
            .filter(|id| self.contains(*id))
            .or_else(|| self.last_leaf())
            .ok_or(LayoutError::InsufficientSpace)?;

        if !current
            .iter()
            .any(|placement| placement.window_id == target_id)
        {
            return Err(LayoutError::InsufficientSpace);
        }

        let preferred = parent_axis(root, target_id).unwrap_or_else(|| {
            if bounds.width >= bounds.height {
                SplitAxis::LeftRight
            } else {
                SplitAxis::TopBottom
            }
        });

        let alternate = match preferred {
            SplitAxis::LeftRight => SplitAxis::TopBottom,
            SplitAxis::TopBottom => SplitAxis::LeftRight,
        };

        let previous = root.clone();
        let previous_next_split_id = self.next_split_id;

        for axis in [preferred, alternate] {
            self.root = Some(previous.clone());

            let split_id = self.allocate_split_id();
            let replaced = Self::split_leaf(
                self.root.as_mut().expect("previous root is present"),
                target_id,
                window_id,
                axis,
                split_id,
            );

            if replaced && self.layout(bounds, gap, min_size).is_ok() {
                return Ok(());
            }
        }

        self.root = Some(previous);
        self.next_split_id = previous_next_split_id;
        Err(LayoutError::InsufficientSpace)
    }

    /// Insert a window while validating only the currently visible layout.
    ///
    /// Excluded windows remain in the persistent tree, preserving their
    /// previous positions while floating or minimized.
    pub fn insert_projected(
        &mut self,
        window_id: u64,
        focused_window: Option<u64>,
        excluded_windows: &[u64],
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<(), LayoutError> {
        fn parent_axis(node: &Node, target_id: u64) -> Option<SplitAxis> {
            match node {
                Node::Leaf(_) => None,
                Node::Split {
                    axis,
                    first,
                    second,
                    ..
                } => {
                    if matches!(first.as_ref(), Node::Leaf(id) if *id == target_id)
                        || matches!(second.as_ref(), Node::Leaf(id) if *id == target_id)
                    {
                        Some(*axis)
                    } else {
                        parent_axis(first, target_id).or_else(|| parent_axis(second, target_id))
                    }
                }
            }
        }

        if self.contains(window_id) {
            return Err(LayoutError::DuplicateWindow(window_id));
        }

        if !bounds.is_valid() {
            return Err(LayoutError::InvalidBounds);
        }

        if self.root.is_none() {
            self.root = Some(Node::Leaf(window_id));
            return Ok(());
        }

        let mut visible = self.clone();
        for excluded_id in excluded_windows {
            visible.remove(*excluded_id);
        }

        let target_id = focused_window
            .filter(|id| visible.contains(*id))
            .or_else(|| visible.last_leaf())
            .or_else(|| self.last_leaf())
            .ok_or(LayoutError::InsufficientSpace)?;

        visible.layout(bounds, gap, min_size)?;

        let root = self.root.as_ref().expect("tree root was checked above");

        let preferred = parent_axis(root, target_id).unwrap_or_else(|| {
            if bounds.width >= bounds.height {
                SplitAxis::LeftRight
            } else {
                SplitAxis::TopBottom
            }
        });

        let alternate = match preferred {
            SplitAxis::LeftRight => SplitAxis::TopBottom,
            SplitAxis::TopBottom => SplitAxis::LeftRight,
        };

        for axis in [preferred, alternate] {
            let mut candidate = self.clone();

            let split_id = candidate.allocate_split_id();
            let replaced = Self::split_leaf(
                candidate
                    .root
                    .as_mut()
                    .expect("tree root was checked above"),
                target_id,
                window_id,
                axis,
                split_id,
            );

            if !replaced {
                continue;
            }

            let mut projection = candidate.clone();
            for excluded_id in excluded_windows {
                projection.remove(*excluded_id);
            }

            if projection.layout(bounds, gap, min_size).is_ok() {
                *self = candidate;
                return Ok(());
            }
        }

        Err(LayoutError::InsufficientSpace)
    }

    /// Remove a leaf and collapse its parent split.
    /// Remove a leaf and collapse its parent split.
    pub fn remove(&mut self, window_id: u64) -> bool {
        fn remove_node(node: Node, id: u64) -> (Option<Node>, bool) {
            match node {
                Node::Leaf(candidate) => {
                    if candidate == id {
                        (None, true)
                    } else {
                        (Some(Node::Leaf(candidate)), false)
                    }
                }
                Node::Split {
                    id: split_id,
                    axis,
                    ratio,
                    first,
                    second,
                } => {
                    let (new_first, removed_first) = remove_node(*first, id);

                    if removed_first {
                        return match new_first {
                            Some(first) => (
                                Some(Node::Split {
                                    id: split_id,
                                    axis,
                                    ratio,
                                    first: Box::new(first),
                                    second,
                                }),
                                true,
                            ),
                            None => (Some(*second), true),
                        };
                    }

                    let first = new_first.expect("unremoved first child must remain");

                    let (new_second, removed_second) = remove_node(*second, id);

                    if removed_second {
                        return match new_second {
                            Some(second) => (
                                Some(Node::Split {
                                    id: split_id,
                                    axis,
                                    ratio,
                                    first: Box::new(first),
                                    second: Box::new(second),
                                }),
                                true,
                            ),
                            None => (Some(first), true),
                        };
                    }

                    (
                        Some(Node::Split {
                            id: split_id,
                            axis,
                            ratio,
                            first: Box::new(first),
                            second: Box::new(
                                new_second.expect("unremoved second child must remain"),
                            ),
                        }),
                        false,
                    )
                }
            }
        }

        let Some(root) = self.root.take() else {
            return false;
        };

        let (root, removed) = remove_node(root, window_id);
        self.root = root;
        removed
    }

    /// Calculate all leaf rectangles in stable tree order.
    pub fn layout(
        &self,
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<Vec<Placement>, LayoutError> {
        if !bounds.is_valid() {
            return Err(LayoutError::InvalidBounds);
        }

        if min_size.width < 1 || min_size.height < 1 {
            return Err(LayoutError::InvalidBounds);
        }

        let Some(root) = self.root.as_ref() else {
            return Ok(Vec::new());
        };

        let mut placements = Vec::with_capacity(self.len());
        Self::layout_node(root, bounds, gap.max(0), min_size, &mut placements)?;

        Ok(placements)
    }

    /// Return perpendicular split junctions suitable for corner resizing.
    pub fn corners(
        &self,
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<Vec<SplitCorner>, LayoutError> {
        self.layout(bounds, gap, min_size)?;

        let mut output = Vec::new();
        if let Some(root) = self.root.as_ref() {
            Self::collect_corners(root, bounds, gap.max(0), min_size, &mut output)?;
        }

        Ok(output)
    }

    fn collect_corners(
        node: &Node,
        rect: Rect,
        gap: i32,
        min_size: MinSize,
        output: &mut Vec<SplitCorner>,
    ) -> Result<(), LayoutError> {
        let Node::Split {
            id: outer_id,
            axis: outer_axis,
            ratio: outer_ratio,
            first,
            second,
        } = node
        else {
            return Ok(());
        };

        let (first_rect, second_rect, outer_divider, actual_gap) =
            Self::split_geometry(rect, *outer_axis, *outer_ratio, gap, min_size)?;

        for (child, child_rect) in [(first.as_ref(), first_rect), (second.as_ref(), second_rect)] {
            if let Node::Split {
                id: inner_id,
                axis: inner_axis,
                ratio: inner_ratio,
                ..
            } = child
            {
                if *outer_axis != *inner_axis {
                    let (_, _, inner_divider, _) = Self::split_geometry(
                        child_rect,
                        *inner_axis,
                        *inner_ratio,
                        actual_gap,
                        min_size,
                    )?;

                    // Handle center is the intersection of the split centerlines.
                    let center_x = match outer_axis {
                        SplitAxis::LeftRight => outer_divider.x + outer_divider.width / 2,
                        SplitAxis::TopBottom => inner_divider.x + inner_divider.width / 2,
                    };
                    let center_y = match outer_axis {
                        SplitAxis::LeftRight => inner_divider.y + inner_divider.height / 2,
                        SplitAxis::TopBottom => outer_divider.y + outer_divider.height / 2,
                    };

                    let handle_size = 24;
                    let rect = Rect::new(
                        center_x - handle_size / 2,
                        center_y - handle_size / 2,
                        handle_size,
                        handle_size,
                    );

                    output.push(SplitCorner {
                        outer_id: *outer_id,
                        inner_id: *inner_id,
                        outer_axis: *outer_axis,
                        inner_axis: *inner_axis,
                        inner_bounds: child_rect,
                        rect,
                    });
                }
            }
        }

        Self::collect_corners(first, first_rect, actual_gap, min_size, output)?;
        Self::collect_corners(second, second_rect, actual_gap, min_size, output)?;

        Ok(())
    }

    /// Return divider geometry in stable tree order.
    ///
    /// The returned rectangles use the same split sizing and minimum-size
    /// constraints as `layout()`.
    pub fn dividers(
        &self,
        bounds: Rect,
        gap: i32,
        min_size: MinSize,
    ) -> Result<Vec<SplitDivider>, LayoutError> {
        // Validate the complete visible tree before exposing divider geometry.
        self.layout(bounds, gap, min_size)?;

        let mut output = Vec::new();
        if let Some(root) = self.root.as_ref() {
            Self::collect_dividers(root, bounds, gap.max(0), min_size, &mut output)?;
        }

        Ok(output)
    }

    fn last_leaf(&self) -> Option<u64> {
        fn last(node: &Node) -> u64 {
            match node {
                Node::Leaf(id) => *id,
                Node::Split { second, .. } => last(second),
            }
        }

        self.root.as_ref().map(last)
    }

    fn split_leaf_with_ratio(
        node: &mut Node,
        target_id: u64,
        new_id: u64,
        axis: SplitAxis,
        ratio: f64,
        split_id: SplitId,
    ) -> bool {
        match node {
            Node::Leaf(id) if *id == target_id => {
                let old_id = *id;
                *node = Node::Split {
                    id: split_id,
                    axis,
                    ratio,
                    first: Box::new(Node::Leaf(old_id)),
                    second: Box::new(Node::Leaf(new_id)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                Self::split_leaf_with_ratio(first, target_id, new_id, axis, ratio, split_id)
                    || Self::split_leaf_with_ratio(second, target_id, new_id, axis, ratio, split_id)
            }
        }
    }

    fn split_leaf(
        node: &mut Node,
        target_id: u64,
        new_id: u64,
        axis: SplitAxis,
        split_id: SplitId,
    ) -> bool {
        match node {
            Node::Leaf(id) if *id == target_id => {
                let old_id = *id;
                *node = Node::Split {
                    id: split_id,
                    axis,
                    ratio: 0.5,
                    first: Box::new(Node::Leaf(old_id)),
                    second: Box::new(Node::Leaf(new_id)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                Self::split_leaf(first, target_id, new_id, axis, split_id)
                    || Self::split_leaf(second, target_id, new_id, axis, split_id)
            }
        }
    }

    /// Calculate both child rectangles and the divider using one shared rule.
    ///
    /// The effective gap is returned so nested splits preserve the same
    /// gap-clamping behavior used by the existing layout engine.
    fn split_geometry(
        rect: Rect,
        axis: SplitAxis,
        ratio: f64,
        gap: i32,
        min_size: MinSize,
    ) -> Result<(Rect, Rect, Rect, i32), LayoutError> {
        let actual_gap = gap.min(match axis {
            SplitAxis::LeftRight => rect.width.saturating_sub(2),
            SplitAxis::TopBottom => rect.height.saturating_sub(2),
        });

        let (first_rect, second_rect, divider_rect) = match axis {
            SplitAxis::LeftRight => {
                let available = rect.width.saturating_sub(actual_gap);
                let minimum = min_size.width;

                if available < minimum.saturating_mul(2) {
                    return Err(LayoutError::InsufficientSpace);
                }

                let first_width = ((available as f64) * ratio)
                    .round()
                    .clamp(minimum as f64, (available - minimum) as f64)
                    as i32;
                let second_width = available - first_width;

                (
                    Rect::new(rect.x, rect.y, first_width, rect.height),
                    Rect::new(
                        rect.x
                            .saturating_add(first_width)
                            .saturating_add(actual_gap),
                        rect.y,
                        second_width,
                        rect.height,
                    ),
                    Rect::new(
                        rect.x.saturating_add(first_width),
                        rect.y,
                        actual_gap,
                        rect.height,
                    ),
                )
            }
            SplitAxis::TopBottom => {
                let available = rect.height.saturating_sub(actual_gap);
                let minimum = min_size.height;

                if available < minimum.saturating_mul(2) {
                    return Err(LayoutError::InsufficientSpace);
                }

                let first_height = ((available as f64) * ratio)
                    .round()
                    .clamp(minimum as f64, (available - minimum) as f64)
                    as i32;
                let second_height = available - first_height;

                (
                    Rect::new(rect.x, rect.y, rect.width, first_height),
                    Rect::new(
                        rect.x,
                        rect.y
                            .saturating_add(first_height)
                            .saturating_add(actual_gap),
                        rect.width,
                        second_height,
                    ),
                    Rect::new(
                        rect.x,
                        rect.y.saturating_add(first_height),
                        rect.width,
                        actual_gap,
                    ),
                )
            }
        };

        Ok((first_rect, second_rect, divider_rect, actual_gap))
    }

    fn collect_dividers(
        node: &Node,
        rect: Rect,
        gap: i32,
        min_size: MinSize,
        output: &mut Vec<SplitDivider>,
    ) -> Result<(), LayoutError> {
        let Node::Split {
            id,
            axis,
            ratio,
            first,
            second,
        } = node
        else {
            return Ok(());
        };

        let (first_rect, second_rect, divider_rect, actual_gap) =
            Self::split_geometry(rect, *axis, *ratio, gap, min_size)?;

        output.push(SplitDivider {
            id: *id,
            axis: *axis,
            ratio: *ratio,
            parent_bounds: rect,
            rect: divider_rect,
        });

        Self::collect_dividers(first, first_rect, actual_gap, min_size, output)?;
        Self::collect_dividers(second, second_rect, actual_gap, min_size, output)?;

        Ok(())
    }

    fn layout_node(
        node: &Node,
        rect: Rect,
        gap: i32,
        min_size: MinSize,
        output: &mut Vec<Placement>,
    ) -> Result<(), LayoutError> {
        match node {
            Node::Leaf(window_id) => {
                if rect.width < min_size.width || rect.height < min_size.height {
                    return Err(LayoutError::InsufficientSpace);
                }

                output.push(Placement {
                    window_id: *window_id,
                    rect,
                });
            }
            Node::Split {
                axis,
                ratio,
                first,
                second,
                ..
            } => {
                let (first_rect, second_rect, _, actual_gap) =
                    Self::split_geometry(rect, *axis, *ratio, gap, min_size)?;

                Self::layout_node(first, first_rect, actual_gap, min_size, output)?;
                Self::layout_node(second, second_rect, actual_gap, min_size, output)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Rect = Rect::new(0, 0, 1920, 1080);
    const MIN_SIZE: MinSize = MinSize::new(320, 240);
    const GAP: i32 = 12;

    fn rects(tree: &SplitTree) -> Vec<Placement> {
        tree.layout(BOUNDS, GAP, MIN_SIZE).unwrap()
    }

    #[test]
    fn first_window_uses_entire_bounds() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();

        assert_eq!(
            rects(&tree),
            vec![Placement {
                window_id: 1,
                rect: BOUNDS,
            }]
        );
    }

    #[test]
    fn second_window_splits_wide_region_left_and_right() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();

        let result = rects(&tree);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].rect, Rect::new(0, 0, 954, 1080));
        assert_eq!(result[1].rect, Rect::new(966, 0, 954, 1080));
    }

    #[test]
    fn focused_window_receives_the_next_split() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(3, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();

        let result = rects(&tree);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].window_id, 1);
        assert_eq!(result[1].window_id, 3);
        assert_eq!(result[2].window_id, 2);

        assert_eq!(result[0].rect, Rect::new(0, 0, 471, 1080));
        assert_eq!(result[1].rect, Rect::new(483, 0, 471, 1080));
        assert_eq!(result[2].rect, Rect::new(966, 0, 954, 1080));
    }

    #[test]
    fn removing_a_window_collapses_its_parent_split() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(3, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();

        assert!(tree.remove(3));
        assert_eq!(tree.len(), 2);

        let result = rects(&tree);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].window_id, 1);
        assert_eq!(result[1].window_id, 2);
    }

    #[test]
    fn removing_last_window_leaves_empty_tree() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();

        assert!(tree.remove(1));
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);
    }

    #[test]
    fn duplicate_window_ids_are_rejected() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();

        assert_eq!(
            tree.insert(1, None, BOUNDS, GAP, MIN_SIZE),
            Err(LayoutError::DuplicateWindow(1))
        );
    }

    #[test]
    fn failed_insert_preserves_previous_tree() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        let before = tree.clone();

        let narrow_bounds = Rect::new(0, 0, 500, 300);

        assert_eq!(
            tree.insert(2, Some(1), narrow_bounds, GAP, MIN_SIZE),
            Err(LayoutError::InsufficientSpace)
        );
        assert_eq!(tree, before);
    }

    #[test]
    fn layout_reflows_to_new_output_bounds() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();

        let resized = Rect::new(100, 50, 1280, 720);
        let result = tree.layout(resized, GAP, MIN_SIZE).unwrap();

        assert_eq!(result[0].rect, Rect::new(100, 50, 634, 720));
        assert_eq!(result[1].rect, Rect::new(746, 50, 634, 720));
    }

    #[test]
    fn portrait_workspace_splits_top_and_bottom() {
        let mut tree = SplitTree::new();
        let portrait = Rect::new(0, 0, 1080, 1920);

        tree.insert(1, None, portrait, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), portrait, GAP, MIN_SIZE).unwrap();

        let result = tree.layout(portrait, GAP, MIN_SIZE).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].rect, Rect::new(0, 0, 1080, 954));
        assert_eq!(result[1].rect, Rect::new(0, 966, 1080, 954));
    }

    #[test]
    fn focused_split_falls_back_when_preferred_axis_cannot_fit() {
        let mut tree = SplitTree::new();
        let bounds = Rect::new(0, 0, 1000, 700);

        tree.insert(1, None, bounds, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), bounds, GAP, MIN_SIZE).unwrap();
        tree.insert(3, Some(1), bounds, GAP, MIN_SIZE).unwrap();

        let result = tree.layout(bounds, GAP, MIN_SIZE).unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].window_id, 1);
        assert_eq!(result[0].rect, Rect::new(0, 0, 494, 344));
        assert_eq!(result[1].window_id, 3);
        assert_eq!(result[1].rect, Rect::new(0, 356, 494, 344));
        assert_eq!(result[2].window_id, 2);
        assert_eq!(result[2].rect, Rect::new(506, 0, 494, 700));
    }
    #[test]
    fn insertion_ignores_suspended_windows_but_preserves_their_tree_nodes() {
        let bounds = Rect::new(0, 0, 1280, 720);
        let mut tree = SplitTree::new();

        tree.insert(1, None, bounds, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), bounds, GAP, MIN_SIZE).unwrap();
        tree.insert(3, Some(1), bounds, GAP, MIN_SIZE).unwrap();

        // Window 2 is suspended but must remain in the persistent tree.
        assert!(tree.contains(2));

        tree.insert_projected(4, Some(1), &[2], bounds, GAP, MIN_SIZE)
            .unwrap();

        assert!(tree.contains(2));
        assert!(tree.contains(4));

        // Remove the suspended window from a temporary visible projection.
        let mut visible = tree.clone();
        assert!(visible.remove(2));

        let placements = visible.layout(bounds, GAP, MIN_SIZE).unwrap();

        assert_eq!(
            placements
                .iter()
                .map(|placement| placement.window_id)
                .collect::<Vec<_>>(),
            vec![1, 4, 3]
        );

        assert_eq!(placements.len(), 3);

        assert!(placements.iter().all(|placement| {
            placement.rect.width >= MIN_SIZE.width && placement.rect.height >= MIN_SIZE.height
        }));
    }
    #[test]
    fn split_ratio_changes_first_child_proportion() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();

        tree.set_split_ratio(1, 0.65, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let result = rects(&tree);

        assert_eq!(result[0].rect, Rect::new(0, 0, 1240, 1080));
        assert_eq!(result[1].rect, Rect::new(1252, 0, 668, 1080));
    }

    #[test]
    fn invalid_split_ratio_is_rejected_without_mutation() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();

        let before = tree.clone();

        assert_eq!(
            tree.set_split_ratio(1, f64::NAN, BOUNDS, GAP, MIN_SIZE),
            Err(LayoutError::InvalidRatio)
        );

        assert_eq!(
            tree.set_split_ratio(1, 1.0, BOUNDS, GAP, MIN_SIZE),
            Err(LayoutError::InvalidRatio)
        );
        assert_eq!(
            tree.set_split_ratio(1, 0.0, BOUNDS, GAP, MIN_SIZE),
            Err(LayoutError::InvalidRatio)
        );

        assert_eq!(
            tree.set_split_ratio(1, -0.1, BOUNDS, GAP, MIN_SIZE),
            Err(LayoutError::InvalidRatio)
        );

        assert_eq!(
            tree.set_split_ratio(1, f64::INFINITY, BOUNDS, GAP, MIN_SIZE),
            Err(LayoutError::InvalidRatio)
        );

        assert_eq!(tree, before);
    }

    #[test]
    fn split_ratio_requires_an_existing_split() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();

        assert_eq!(
            tree.set_split_ratio(1, 0.6, BOUNDS, GAP, MIN_SIZE),
            Err(LayoutError::NoSplitForWindow(1))
        );

        assert_eq!(
            tree.set_split_ratio(99, 0.6, BOUNDS, GAP, MIN_SIZE),
            Err(LayoutError::WindowNotFound(99))
        );
    }
    #[test]
    fn dual_split_ratio_update_changes_both_dividers_atomically() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();
        tree.insert_with_ratio(3, 2, SplitAxis::TopBottom, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let before_layout = rects(&tree);
        let before_dividers = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();

        assert_eq!(before_dividers.len(), 2);

        let first_id = before_dividers[0].id;
        let second_id = before_dividers[1].id;

        tree.set_split_ratios_by_id((first_id, 0.60), (second_id, 0.65), BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let after_layout = rects(&tree);
        let after_dividers = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();

        assert_ne!(before_layout, after_layout);
        assert_eq!(after_dividers.len(), 2);

        assert_ne!(before_dividers[0].rect, after_dividers[0].rect);
        assert_ne!(before_dividers[1].rect, after_dividers[1].rect);
    }

    #[test]
    fn dual_split_ratio_update_missing_id_preserves_tree() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();
        tree.insert_with_ratio(3, 2, SplitAxis::TopBottom, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let dividers = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();
        let before = tree.clone();

        assert_eq!(
            tree.set_split_ratios_by_id(
                (dividers[0].id, 0.60),
                (SplitId(999999), 0.65),
                BOUNDS,
                GAP,
                MIN_SIZE,
            ),
            Err(LayoutError::SplitNotFound(SplitId(999999)))
        );

        assert_eq!(tree, before);
    }

    #[test]
    fn dual_split_ratio_update_rejects_invalid_ratios_without_mutation() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();
        tree.insert_with_ratio(3, 2, SplitAxis::TopBottom, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let dividers = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();
        let before = tree.clone();

        assert_eq!(
            tree.set_split_ratios_by_id(
                (dividers[0].id, 0.60),
                (dividers[1].id, f64::NAN),
                BOUNDS,
                GAP,
                MIN_SIZE,
            ),
            Err(LayoutError::InvalidRatio)
        );

        assert_eq!(
            tree.set_split_ratios_by_id(
                (dividers[0].id, 0.60),
                (dividers[0].id, 0.65),
                BOUNDS,
                GAP,
                MIN_SIZE,
            ),
            Err(LayoutError::InvalidRatio)
        );

        assert_eq!(tree, before);
    }

    #[test]
    fn explicit_insert_uses_requested_fractional_ratio() {
        let bounds = Rect::new(0, 0, 1920, 1080);
        let gap = 12;
        let min_size = MinSize::new(320, 240);
        let mut tree = SplitTree::new();

        tree.insert(1, None, bounds, gap, min_size).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.65, bounds, gap, min_size)
            .unwrap();

        let placements = tree.layout(bounds, gap, min_size).unwrap();
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].window_id, 1);
        assert_eq!(placements[1].window_id, 2);
        assert_eq!(placements[0].rect.width, 1240);
    }

    #[test]
    fn explicit_insert_supports_nested_splits() {
        let bounds = Rect::new(0, 0, 1920, 1080);
        let gap = 12;
        let min_size = MinSize::new(320, 240);
        let mut tree = SplitTree::new();

        tree.insert(1, None, bounds, gap, min_size).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.65, bounds, gap, min_size)
            .unwrap();
        tree.insert_with_ratio(3, 2, SplitAxis::TopBottom, 0.60, bounds, gap, min_size)
            .unwrap();

        assert_eq!(tree.len(), 3);
        assert_eq!(tree.layout(bounds, gap, min_size).unwrap().len(), 3);
    }

    #[test]
    fn changing_split_axis_updates_child_geometry() {
        let bounds = Rect::new(0, 0, 1920, 1080);
        let gap = 12;
        let min_size = MinSize::new(320, 240);
        let mut tree = SplitTree::new();

        tree.insert(1, None, bounds, gap, min_size).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.5, bounds, gap, min_size)
            .unwrap();

        tree.set_split_axis(2, SplitAxis::TopBottom, bounds, gap, min_size)
            .unwrap();

        let placements = tree.layout(bounds, gap, min_size).unwrap();
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].rect.width, bounds.width);
        assert!(placements[0].rect.height < bounds.height);
    }

    #[test]
    fn invalid_explicit_insert_preserves_tree() {
        let bounds = Rect::new(0, 0, 1920, 1080);
        let gap = 12;
        let min_size = MinSize::new(320, 240);
        let mut tree = SplitTree::new();

        tree.insert(1, None, bounds, gap, min_size).unwrap();
        let before = tree.clone();

        assert_eq!(
            tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 1.5, bounds, gap, min_size),
            Err(LayoutError::InvalidRatio)
        );
        assert_eq!(tree, before);
    }

    #[test]
    fn split_id_survives_ratio_and_axis_changes() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let original = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();
        assert_eq!(original.len(), 1);
        let original_id = original[0].id;

        tree.set_split_ratio(1, 0.65, BOUNDS, GAP, MIN_SIZE)
            .unwrap();
        let after_ratio = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();
        assert_eq!(after_ratio[0].id, original_id);
        assert_eq!(after_ratio[0].ratio, 0.65);

        tree.set_split_axis(2, SplitAxis::TopBottom, BOUNDS, GAP, MIN_SIZE)
            .unwrap();
        let after_axis = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();
        assert_eq!(after_axis[0].id, original_id);
        assert_eq!(after_axis[0].axis, SplitAxis::TopBottom);
    }

    #[test]
    fn set_split_ratio_by_id_updates_nested_split() {
        let bounds = BOUNDS;
        let mut tree = SplitTree::new();

        tree.insert(1, None, bounds, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.65, bounds, GAP, MIN_SIZE)
            .unwrap();
        tree.insert_with_ratio(3, 2, SplitAxis::TopBottom, 0.60, bounds, GAP, MIN_SIZE)
            .unwrap();

        let dividers = tree.dividers(bounds, GAP, MIN_SIZE).unwrap();
        let target = dividers[1].id;

        tree.set_split_ratio_by_id(target, 0.70, bounds, GAP, MIN_SIZE)
            .unwrap();

        let updated = tree.dividers(bounds, GAP, MIN_SIZE).unwrap();
        assert_eq!(updated[1].id, target);
        assert!((updated[1].ratio - 0.70).abs() < f64::EPSILON);
    }

    #[test]
    fn set_split_ratio_by_id_invalid_id_preserves_tree() {
        let mut tree = SplitTree::new();
        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert(2, Some(1), BOUNDS, GAP, MIN_SIZE).unwrap();

        let before = tree.clone();
        let result = tree.set_split_ratio_by_id(SplitId(u64::MAX), 0.60, BOUNDS, GAP, MIN_SIZE);

        assert_eq!(result, Err(LayoutError::SplitNotFound(SplitId(u64::MAX))));
        assert_eq!(tree, before);
    }

    #[test]
    fn nested_dividers_report_matching_parent_and_gap_geometry() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.65, BOUNDS, GAP, MIN_SIZE)
            .unwrap();
        tree.insert_with_ratio(3, 2, SplitAxis::TopBottom, 0.60, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let placements = tree.layout(BOUNDS, GAP, MIN_SIZE).unwrap();
        let dividers = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();

        assert_eq!(dividers.len(), 2);
        assert_ne!(dividers[0].id, dividers[1].id);

        let outer = dividers[0];
        let inner = dividers[1];

        assert_eq!(outer.axis, SplitAxis::LeftRight);
        assert_eq!(outer.parent_bounds, BOUNDS);
        assert_eq!(outer.rect.width, GAP);
        assert_eq!(outer.rect.height, BOUNDS.height);

        let second_region = Rect::new(1252, 0, 668, 1080);

        assert!(placements.iter().any(|placement| {
            placement.window_id == 2 && placement.rect.height < second_region.height
        }));
        assert_eq!(inner.parent_bounds, second_region);
        assert_eq!(inner.axis, SplitAxis::TopBottom);
        assert_eq!(inner.rect.height, GAP);
        assert_eq!(inner.rect.width, second_region.width);
    }

    #[test]
    fn perpendicular_nested_split_exposes_corner_geometry() {
        let mut tree = SplitTree::new();
        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.65, BOUNDS, GAP, MIN_SIZE)
            .unwrap();
        tree.insert_with_ratio(3, 2, SplitAxis::TopBottom, 0.60, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let dividers = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap();
        let corners = tree.corners(BOUNDS, GAP, MIN_SIZE).unwrap();

        assert_eq!(corners.len(), 1);
        assert_eq!(corners[0].outer_id, dividers[0].id);
        assert_eq!(corners[0].inner_id, dividers[1].id);
        assert_eq!(corners[0].outer_axis, SplitAxis::LeftRight);
        assert_eq!(corners[0].inner_axis, SplitAxis::TopBottom);
        assert_eq!(corners[0].rect.width, 24);
        assert_eq!(corners[0].rect.height, 24);
    }

    #[test]
    fn same_axis_nested_splits_do_not_expose_corner() {
        let mut tree = SplitTree::new();
        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.65, BOUNDS, GAP, MIN_SIZE)
            .unwrap();
        tree.insert_with_ratio(3, 2, SplitAxis::LeftRight, 0.60, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        assert!(tree.corners(BOUNDS, GAP, MIN_SIZE).unwrap().is_empty());
    }

    #[test]
    fn collapsed_split_id_is_not_reused() {
        let mut tree = SplitTree::new();

        tree.insert(1, None, BOUNDS, GAP, MIN_SIZE).unwrap();
        tree.insert_with_ratio(2, 1, SplitAxis::LeftRight, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let first_id = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap()[0].id;

        assert!(tree.remove(2));
        assert!(tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap().is_empty());

        tree.insert_with_ratio(3, 1, SplitAxis::TopBottom, 0.5, BOUNDS, GAP, MIN_SIZE)
            .unwrap();

        let replacement_id = tree.dividers(BOUNDS, GAP, MIN_SIZE).unwrap()[0].id;
        assert_ne!(replacement_id, first_id);
    }

    #[test]
    fn changing_axis_without_parent_split_is_rejected() {
        let bounds = Rect::new(0, 0, 1920, 1080);
        let gap = 12;
        let min_size = MinSize::new(320, 240);
        let mut tree = SplitTree::new();

        tree.insert(1, None, bounds, gap, min_size).unwrap();

        assert_eq!(
            tree.set_split_axis(1, SplitAxis::TopBottom, bounds, gap, min_size),
            Err(LayoutError::NoSplitForWindow(1))
        );
    }
}
