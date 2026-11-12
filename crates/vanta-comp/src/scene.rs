//! Renderer-independent desktop scene ordering.

/// Compose render elements in the front-to-back order expected by
/// Smithay's output damage tracker.
///
/// The renderer-specific backend creates the elements. This function only
/// defines their shared desktop stacking order.
pub(crate) fn compose_front_to_back<E>(
    mut overlays: Vec<E>,
    mut desktop: Vec<E>,
    wallpaper: Option<E>,
) -> Vec<E> {
    let capacity = overlays.len() + desktop.len() + usize::from(wallpaper.is_some());
    let mut elements = Vec::with_capacity(capacity);

    elements.append(&mut overlays);
    elements.append(&mut desktop);

    if let Some(wallpaper) = wallpaper {
        elements.push(wallpaper);
    }

    elements
}

#[cfg(test)]
mod tests {
    use super::compose_front_to_back;

    #[test]
    fn preserves_overlay_desktop_wallpaper_order() {
        let result = compose_front_to_back(vec![1, 2], vec![3, 4, 5], Some(6));

        assert_eq!(result, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn supports_missing_wallpaper() {
        let result = compose_front_to_back(vec![1], vec![2, 3], None);

        assert_eq!(result, vec![1, 2, 3]);
    }
}
