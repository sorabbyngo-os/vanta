//! Pure adaptive split-tree layout engine.
//!
//! This module deliberately has no Smithay or compositor dependencies.
//! It owns persistent window topology and calculates logical rectangles.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitAxis {
    LeftRight,
    TopBottom,
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Leaf(u64),
    Split {
        axis: SplitAxis,
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SplitTree {
    root: Option<Node>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    DuplicateWindow(u64),
    InvalidBounds,
    InsufficientSpace,
}

impl SplitTree {
    pub fn new() -> Self {
        Self::default()
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

        for axis in [preferred, alternate] {
            self.root = Some(previous.clone());

            let replaced = Self::split_leaf(
                self.root.as_mut().expect("previous root is present"),
                target_id,
                window_id,
                axis,
            );

            if replaced && self.layout(bounds, gap, min_size).is_ok() {
                return Ok(());
            }
        }

        self.root = Some(previous);
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

            let replaced = Self::split_leaf(
                candidate
                    .root
                    .as_mut()
                    .expect("tree root was checked above"),
                target_id,
                window_id,
                axis,
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

    fn last_leaf(&self) -> Option<u64> {
        fn last(node: &Node) -> u64 {
            match node {
                Node::Leaf(id) => *id,
                Node::Split { second, .. } => last(second),
            }
        }

        self.root.as_ref().map(last)
    }

    fn split_leaf(node: &mut Node, target_id: u64, new_id: u64, axis: SplitAxis) -> bool {
        match node {
            Node::Leaf(id) if *id == target_id => {
                let old_id = *id;
                *node = Node::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Node::Leaf(old_id)),
                    second: Box::new(Node::Leaf(new_id)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                Self::split_leaf(first, target_id, new_id, axis)
                    || Self::split_leaf(second, target_id, new_id, axis)
            }
        }
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
            } => {
                let gap = gap.min(match axis {
                    SplitAxis::LeftRight => rect.width.saturating_sub(2),
                    SplitAxis::TopBottom => rect.height.saturating_sub(2),
                });

                let (first_rect, second_rect) = match axis {
                    SplitAxis::LeftRight => {
                        let available = rect.width.saturating_sub(gap);
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
                                rect.x.saturating_add(first_width).saturating_add(gap),
                                rect.y,
                                second_width,
                                rect.height,
                            ),
                        )
                    }
                    SplitAxis::TopBottom => {
                        let available = rect.height.saturating_sub(gap);
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
                                rect.y.saturating_add(first_height).saturating_add(gap),
                                rect.width,
                                second_height,
                            ),
                        )
                    }
                };

                Self::layout_node(first, first_rect, gap, min_size, output)?;
                Self::layout_node(second, second_rect, gap, min_size, output)?;
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
}
