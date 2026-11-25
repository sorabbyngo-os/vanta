use super::{primitives::Canvas, theme};

const ICON_SIZE: f32 = 48.0;
const ICON_GAP: f32 = 12.0;
const DOCK_PADDING: f32 = 14.0;
const ICON_COUNT: u32 = 5;

pub const SURFACE_WIDTH: u32 = (ICON_COUNT as f32 * ICON_SIZE
    + (ICON_COUNT - 1) as f32 * ICON_GAP
    + 2.0 * DOCK_PADDING) as u32;

pub fn draw(
    canvas: &mut Canvas<'_>,
    width: u32,
    height: u32,
    workspace_count: u32,
    active_workspace: u32,
) {
    if width < 180 || height < theme::DOCK_HEIGHT + 8 {
        return;
    }

    let content_width = ICON_COUNT as f32 * ICON_SIZE + (ICON_COUNT - 1) as f32 * ICON_GAP;
    let dock_width = content_width + DOCK_PADDING * 2.0;

    if dock_width > width as f32 {
        return;
    }

    let dock_x = (width as f32 - dock_width) * 0.5;
    let dock_y = (height as f32 - theme::DOCK_HEIGHT as f32) * 0.5;

    // Layered translucent shadow. Kept subtle so the dock does not look detached.
    canvas.rounded_rect_f(
        dock_x - 2.0,
        dock_y + 3.0,
        dock_width + 4.0,
        theme::DOCK_HEIGHT as f32 + 3.0,
        25.0,
        0x18000000,
    );
    canvas.rounded_rect_f(
        dock_x - 1.0,
        dock_y + 2.0,
        dock_width + 2.0,
        theme::DOCK_HEIGHT as f32 + 2.0,
        24.0,
        0x22000000,
    );

    // Main glass-like surface and restrained outline.
    canvas.rounded_rect_f(
        dock_x,
        dock_y,
        dock_width,
        theme::DOCK_HEIGHT as f32,
        23.0,
        0xE51A2030,
    );
    canvas.rounded_rect_f(
        dock_x + 0.75,
        dock_y + 0.75,
        dock_width - 1.5,
        theme::DOCK_HEIGHT as f32 - 1.5,
        22.25,
        0x557D91B5,
    );
    canvas.fill_rect_f(
        dock_x + 18.0,
        dock_y + 1.5,
        dock_width - 36.0,
        0.65,
        0x28FFFFFF,
    );

    for index in 0..ICON_COUNT {
        let x = dock_x + DOCK_PADDING + index as f32 * (ICON_SIZE + ICON_GAP);
        let y = dock_y + (theme::DOCK_HEIGHT as f32 - ICON_SIZE) * 0.5;

        draw_icon(canvas, index, x, y);

        // Running indicator, retained for the first pinned application.
        if index == 0 {
            canvas.rounded_rect_f(
                x + ICON_SIZE * 0.5 - 3.0,
                dock_y + theme::DOCK_HEIGHT as f32 - 5.0,
                6.0,
                2.5,
                1.25,
                theme::COLOR_ACCENT,
            );
        }
    }

    draw_workspaces(
        canvas,
        width,
        dock_y,
        dock_width,
        workspace_count,
        active_workspace,
    );
}

fn draw_workspaces(
    canvas: &mut Canvas<'_>,
    width: u32,
    dock_y: f32,
    dock_width: f32,
    workspace_count: u32,
    active_workspace: u32,
) {
    const DOT_SIZE: f32 = 4.0;
    const DOT_GAP: f32 = 5.0;
    const MAX_VISIBLE: u32 = 15;

    let count = workspace_count.clamp(1, MAX_VISIBLE);
    let total_width = count as f32 * DOT_SIZE + (count - 1) as f32 * DOT_GAP;
    let start_x = (width as f32 - total_width) * 0.5;
    let y = dock_y + theme::DOCK_HEIGHT as f32 + 2.0;

    // Keep the indicator row within the existing dock surface.
    if y + DOT_SIZE > 92.0 {
        return;
    }

    for index in 0..count {
        let x = start_x + index as f32 * (DOT_SIZE + DOT_GAP);
        let color = if index == active_workspace {
            theme::COLOR_ACCENT
        } else {
            theme::COLOR_TEXT_MUTED
        };

        canvas.rounded_rect_f(x, y, DOT_SIZE, DOT_SIZE, 2.0, color);
    }

    let _ = dock_width;
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
