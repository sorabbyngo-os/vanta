use super::{font::FontRenderer, primitives::Canvas, theme};
use vanta_ipc::MinimizedWindowInfo;

const ICON_SIZE: f32 = 44.0;
const ICON_GAP: f32 = 8.0;
const DOCK_PADDING: f32 = 10.0;
const ICON_COUNT: u32 = 5;

const APP_DOCK_X: f32 = 14.0;
const APP_DOCK_Y: f32 = 8.0;
const APP_DOCK_RADIUS: f32 = 21.0;

const CARD_HEIGHT: f32 = 48.0;
const CARD_MIN_WIDTH: f32 = 132.0;
const CARD_MAX_WIDTH: f32 = 220.0;
const CARD_GAP: f32 = 8.0;
const RIGHT_MARGIN: f32 = 14.0;

pub const SURFACE_WIDTH: u32 = (ICON_COUNT as f32 * ICON_SIZE
    + (ICON_COUNT - 1) as f32 * ICON_GAP
    + 2.0 * DOCK_PADDING) as u32;

fn app_dock_width() -> f32 {
    ICON_COUNT as f32 * ICON_SIZE + (ICON_COUNT - 1) as f32 * ICON_GAP + 2.0 * DOCK_PADDING
}

fn card_layout(
    width: u32,
    height: u32,
    windows: &[MinimizedWindowInfo],
) -> (usize, usize, f32, f32, f32) {
    let start_x = APP_DOCK_X + app_dock_width() + 12.0;
    let available = (width as f32 - start_x - RIGHT_MARGIN).max(0.0);

    let capacity = ((available + CARD_GAP) / (CARD_MIN_WIDTH + CARD_GAP))
        .floor()
        .max(0.0) as usize;

    let visible = windows.len().min(capacity);
    if visible == 0 {
        return (windows.len(), 0, start_x, 0.0, 0.0);
    }

    let card_width = ((available - CARD_GAP * (visible - 1) as f32) / visible as f32)
        .min(CARD_MAX_WIDTH)
        .max(CARD_MIN_WIDTH);

    let first_index = windows.len() - visible;
    let y = (height as f32 - CARD_HEIGHT) * 0.5;

    (first_index, visible, start_x, y, card_width)
}

/// Returns the minimized window ID under a pointer position.
pub fn minimized_window_at(
    width: u32,
    height: u32,
    windows: &[MinimizedWindowInfo],
    x: f64,
    y: f64,
) -> Option<u64> {
    let (first, visible, start_x, card_y, card_width) = card_layout(width, height, windows);

    if visible == 0 {
        return None;
    }

    for offset in 0..visible {
        let card_x = start_x + offset as f32 * (card_width + CARD_GAP);
        let inside = x >= card_x as f64
            && x < (card_x + card_width) as f64
            && y >= card_y as f64
            && y < (card_y + CARD_HEIGHT) as f64;

        if inside {
            return Some(windows[first + offset].id);
        }
    }

    None
}

/// Workspace indicators remain below the app dock, not below the minimized cards.
pub fn workspace_at(width: u32, height: u32, workspace_count: u32, x: f64, y: f64) -> Option<u32> {
    const DOT_SIZE: f32 = 4.0;
    const DOT_GAP: f32 = 5.0;
    const MAX_VISIBLE: u32 = 15;

    if y < height.saturating_sub(8) as f64 || y >= height as f64 {
        return None;
    }

    let count = workspace_count.min(MAX_VISIBLE);
    if count == 0 {
        return None;
    }

    let total_width = count as f32 * DOT_SIZE + (count - 1) as f32 * DOT_GAP;
    let start_x = APP_DOCK_X + (app_dock_width() - total_width) * 0.5;
    let relative_x = x - start_x as f64;

    if relative_x < 0.0 || relative_x >= total_width as f64 {
        return None;
    }

    let index = (relative_x / (DOT_SIZE + DOT_GAP) as f64) as u32;
    (index < count).then_some(index)
}

pub fn draw(
    canvas: &mut Canvas<'_>,
    font: &FontRenderer,
    width: u32,
    height: u32,
    workspace_count: u32,
    active_workspace: u32,
    minimized_windows: &[MinimizedWindowInfo],
) {
    if width < 320 || height < theme::DOCK_HEIGHT + 8 {
        return;
    }

    let dock_width = app_dock_width().min(width as f32 * 0.70);
    let dock_height = theme::DOCK_HEIGHT as f32;
    let dock_y = APP_DOCK_Y;

    // App dock: left side of the shared bottom strip.
    if dock_width >= app_dock_width() {
        canvas.rounded_rect_f(
            APP_DOCK_X - 2.0,
            dock_y + 3.0,
            dock_width + 4.0,
            dock_height + 3.0,
            APP_DOCK_RADIUS + 2.0,
            0x18000000,
        );

        canvas.rounded_rect_f(
            APP_DOCK_X,
            dock_y,
            dock_width,
            dock_height,
            APP_DOCK_RADIUS,
            0xE51A2030,
        );

        canvas.rounded_rect_f(
            APP_DOCK_X + 0.75,
            dock_y + 0.75,
            dock_width - 1.5,
            dock_height - 1.5,
            APP_DOCK_RADIUS - 0.75,
            0x557D91B5,
        );

        for index in 0..ICON_COUNT {
            let x = APP_DOCK_X + DOCK_PADDING + index as f32 * (ICON_SIZE + ICON_GAP);
            let y = dock_y + (dock_height - ICON_SIZE) * 0.5;

            draw_icon(canvas, index, x, y);

            if index == 0 {
                canvas.rounded_rect_f(
                    x + ICON_SIZE * 0.5 - 3.0,
                    dock_y + dock_height - 5.0,
                    6.0,
                    2.5,
                    1.25,
                    theme::COLOR_ACCENT,
                );
            }
        }

        draw_workspaces(canvas, height, workspace_count, active_workspace);
    }

    // Minimized windows: right side, vertically aligned with the app dock.
    let (first, visible, start_x, card_y, card_width) =
        card_layout(width, height, minimized_windows);

    for offset in 0..visible {
        let window = &minimized_windows[first + offset];
        let x = start_x + offset as f32 * (card_width + CARD_GAP);

        canvas.rounded_rect_f(x, card_y + 2.0, card_width, CARD_HEIGHT, 15.0, 0x18000000);

        canvas.rounded_rect_f(x, card_y, card_width, CARD_HEIGHT, 15.0, 0xE51A2030);

        canvas.rounded_rect_f(
            x + 0.75,
            card_y + 0.75,
            card_width - 1.5,
            CARD_HEIGHT - 1.5,
            14.25,
            0x557D91B5,
        );

        // Small application marker.
        canvas.rounded_rect_f(x + 10.0, card_y + 17.0, 5.0, 14.0, 2.5, theme::COLOR_ACCENT);

        let title = window
            .title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or(window.app_id.as_deref())
            .unwrap_or("Minimized window");

        let available_text_width = card_width - 36.0;
        let title = fit_title(font, title, available_text_width, 13.0);

        font.draw_text(
            canvas,
            x + 23.0,
            card_y + 29.0,
            &title,
            theme::COLOR_TEXT,
            13.0,
        );
    }
}

fn fit_title(font: &FontRenderer, title: &str, max_width: f32, size: f32) -> String {
    if font.text_width(title, size) <= max_width {
        return title.to_owned();
    }

    let mut result = String::new();

    for character in title.chars() {
        let mut candidate = result.clone();
        candidate.push(character);
        candidate.push('…');

        if font.text_width(&candidate, size) > max_width {
            break;
        }

        result.push(character);
    }

    result.push('…');
    result
}

fn draw_workspaces(
    canvas: &mut Canvas<'_>,
    height: u32,
    workspace_count: u32,
    active_workspace: u32,
) {
    const DOT_SIZE: f32 = 4.0;
    const DOT_GAP: f32 = 5.0;
    const MAX_VISIBLE: u32 = 15;

    let count = workspace_count.clamp(1, MAX_VISIBLE);
    let total_width = count as f32 * DOT_SIZE + (count - 1) as f32 * DOT_GAP;
    let start_x = APP_DOCK_X + (app_dock_width() - total_width) * 0.5;
    let y = height as f32 - 6.0;

    for index in 0..count {
        let x = start_x + index as f32 * (DOT_SIZE + DOT_GAP);
        let color = if index == active_workspace {
            theme::COLOR_ACCENT
        } else {
            theme::COLOR_TEXT_MUTED
        };

        canvas.rounded_rect_f(x, y, DOT_SIZE, DOT_SIZE, 2.0, color);
    }
}

fn draw_icon(canvas: &mut Canvas<'_>, index: u32, x: f32, y: f32) {
    let white = theme::COLOR_TEXT;
    let dark = 0xFF263247;
    let muted = theme::COLOR_TEXT_MUTED;

    let backgrounds = [0xFF354B70, 0xFF315D78, 0xFF53617A, 0xFF65567F, 0xFF3E6870];

    canvas.rounded_rect_f(
        x,
        y,
        ICON_SIZE,
        ICON_SIZE,
        14.0,
        backgrounds[index as usize],
    );

    match index {
        // Documents
        0 => {
            canvas.rounded_rect_f(x + 14.0, y + 9.0, 20.0, 29.0, 4.0, white);
            canvas.rounded_rect_f(x + 18.0, y + 17.0, 12.0, 1.5, 0.75, dark);
            canvas.rounded_rect_f(x + 18.0, y + 22.0, 9.0, 1.5, 0.75, dark);
            canvas.rounded_rect_f(x + 18.0, y + 27.0, 12.0, 1.5, 0.75, dark);
        }

        // Browser / globe
        1 => {
            canvas.rounded_rect_f(x + 9.0, y + 9.0, 30.0, 30.0, 15.0, white);
            canvas.rounded_rect_f(x + 11.0, y + 21.0, 26.0, 2.0, 1.0, backgrounds[1]);
            canvas.rounded_rect_f(x + 22.0, y + 11.0, 2.0, 26.0, 1.0, backgrounds[1]);
            canvas.rounded_rect_f(x + 15.0, y + 15.0, 18.0, 18.0, 9.0, backgrounds[1]);
            canvas.rounded_rect_f(x + 22.0, y + 16.0, 2.0, 16.0, 1.0, white);
        }

        // Folder
        2 => {
            canvas.rounded_rect_f(x + 10.0, y + 15.0, 28.0, 22.0, 5.0, white);
            canvas.rounded_rect_f(x + 12.0, y + 11.0, 13.0, 8.0, 3.0, muted);
            canvas.rounded_rect_f(x + 13.0, y + 22.0, 22.0, 1.5, 0.75, backgrounds[2]);
        }

        // Settings
        3 => {
            canvas.rounded_rect_f(x + 13.0, y + 13.0, 22.0, 22.0, 7.0, white);
            canvas.rounded_rect_f(x + 19.0, y + 19.0, 10.0, 10.0, 5.0, backgrounds[3]);
            canvas.rounded_rect_f(x + 22.0, y + 8.0, 4.0, 8.0, 2.0, white);
            canvas.rounded_rect_f(x + 22.0, y + 32.0, 4.0, 8.0, 2.0, white);
            canvas.rounded_rect_f(x + 8.0, y + 22.0, 8.0, 4.0, 2.0, white);
            canvas.rounded_rect_f(x + 32.0, y + 22.0, 8.0, 4.0, 2.0, white);
        }

        // Application grid
        _ => {
            for row in 0..3 {
                for column in 0..3 {
                    canvas.rounded_rect_f(
                        x + 11.0 + column as f32 * 9.0,
                        y + 11.0 + row as f32 * 9.0,
                        6.0,
                        6.0,
                        2.0,
                        white,
                    );
                }
            }
        }
    }
}
