use super::{font::FontRenderer, primitives::Canvas, theme};

pub fn draw(
    canvas: &mut Canvas<'_>,
    font: &FontRenderer,
    width: u32,
    height: u32,
    workspace_count: u32,
    active_workspace: u32,
    controls_hovered: bool,
) {
    if width == 0 || height == 0 {
        return;
    }

    let w = width as f32;
    let h = height as f32;
    let center_y = h * 0.5;

    // Full-width glass surface with a subtle upper reflection.

    let control_y = ((h - 30.0) * 0.5).max(0.0);
    let control_h = 30.0_f32.min(h);
    let baseline = control_y + (control_h - 13.0) * 0.5 + 10.0;

    // Activities and workspace indicators share one glass pill.
    let count = workspace_count.clamp(1, 15);
    let active = active_workspace.min(count - 1);
    let dot_spacing = 10.0;
    let dots_width = count as f32 * dot_spacing - 4.0;
    let activities_width = 104.0 + dots_width + 18.0;

    canvas.rounded_rect_f(
        12.0,
        control_y,
        activities_width,
        control_h,
        10.0,
        theme::COLOR_GLASS_SURFACE,
    );

    font.draw_text(
        canvas,
        25.0,
        baseline,
        "Activities",
        theme::COLOR_TEXT,
        13.0,
    );

    let dots_start = 25.0 + font.text_width("Activities", 13.0) + 15.0;

    for index in 0..count {
        let is_active = index == active;
        let dot_size = if is_active { 7.0 } else { 5.0 };
        let dot_x = dots_start + index as f32 * dot_spacing;
        let dot_y = center_y - dot_size * 0.5;

        canvas.rounded_rect_f(
            dot_x,
            dot_y,
            dot_size,
            dot_size,
            dot_size * 0.5,
            if is_active {
                theme::COLOR_ACCENT
            } else {
                theme::COLOR_WORKSPACE_DOT
            },
        );
    }

    // Apps and Places navigation.
    let nav_x = 12.0 + activities_width + 22.0;

    font.draw_text(
        canvas,
        nav_x,
        baseline,
        "Apps",
        theme::COLOR_TEXT_MUTED,
        12.0,
    );

    let places_x = nav_x + font.text_width("Apps", 12.0) + 24.0;

    font.draw_text(
        canvas,
        places_x,
        baseline,
        "Places",
        theme::COLOR_TEXT_MUTED,
        12.0,
    );

    // Shared geometry keeps the workspace pill clear of Quick Settings.
    let (quick_x, quick_y, quick_width, quick_height) = quick_settings_bounds(width, height);

    let label = format!("WORKSPACE  {} / {}", active + 1, count);
    let label_width = font.text_width(&label, 10.5);
    let pill_width = label_width + 24.0;
    let pill_x = (quick_x - 10.0 - pill_width).max(nav_x + 150.0);

    if pill_x + pill_width <= w - 8.0 {
        canvas.rounded_rect_f(
            pill_x,
            control_y,
            pill_width,
            control_h,
            10.0,
            theme::COLOR_GLASS_SURFACE,
        );

        font.draw_text(
            canvas,
            pill_x + 12.0,
            baseline,
            &label,
            theme::COLOR_TEXT_MUTED,
            10.5,
        );
    }

    // Dedicated Quick Settings entry.
    canvas.rounded_rect_f(
        quick_x,
        quick_y,
        quick_width,
        quick_height,
        10.0,
        if controls_hovered {
            theme::COLOR_PANEL_ACCENT
        } else {
            theme::COLOR_GLASS_SURFACE
        },
    );
    font.draw_text(
        canvas,
        quick_x + 17.0,
        quick_y + 19.0,
        "Controls",
        theme::COLOR_TEXT,
        12.0,
    );
}

/// Panel-local geometry shared by rendering and pointer hit-testing.
pub fn quick_settings_bounds(width: u32, height: u32) -> (f32, f32, f32, f32) {
    let x = (width as f32 - 112.0).max(8.0);
    let y = ((height as f32 - 30.0) * 0.5).max(0.0);
    (
        x,
        y,
        100.0_f32.min((width as f32 - x).max(0.0)),
        30.0_f32.min(height as f32),
    )
}

pub fn quick_settings_hit_test(width: u32, height: u32, x: f64, y: f64) -> bool {
    let (left, top, button_width, button_height) = quick_settings_bounds(width, height);
    x >= left as f64
        && x < (left + button_width) as f64
        && y >= top as f64
        && y < (top + button_height) as f64
}
