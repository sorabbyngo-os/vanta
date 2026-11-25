use super::{font::FontRenderer, primitives::Canvas, theme};

const PANEL_BG: u32 = 0xE51A2638;
const CARD_BG: u32 = 0x72566B88;
const CARD_HIGHLIGHT: u32 = 0x35FFFFFF;
const ACTIVE_BG: u32 = 0xAA426BA8;
const MAX_VISIBLE_WORKSPACES: u32 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Close,
    SwitchWorkspace(u32),
    CreateWorkspace,
}

pub fn draw(
    canvas: &mut Canvas<'_>,
    font: &FontRenderer,
    width: u32,
    height: u32,
    workspace_count: u32,
    active_workspace: u32,
) {
    if width < 100 || height < 100 {
        return;
    }

    let w = width as f32;
    let h = height as f32;

    canvas.rounded_rect_f(2.0, 2.0, w - 4.0, h - 4.0, 22.0, PANEL_BG);
    canvas.rounded_rect_f(3.0, 3.0, w - 6.0, 1.0, 0.5, 0x48FFFFFF);

    font.draw_text(canvas, 18.0, 32.0, "Controls", theme::COLOR_TEXT, 17.0);
    font.draw_text(
        canvas,
        18.0,
        53.0,
        "Workspace and system controls",
        theme::COLOR_TEXT_MUTED,
        10.5,
    );

    canvas.rounded_rect_f(w - 66.0, 13.0, 48.0, 27.0, 9.0, CARD_BG);
    font.draw_text(canvas, w - 55.0, 31.0, "Close", theme::COLOR_TEXT, 10.0);

    canvas.horizontal_line(16, 68, width.saturating_sub(32), 0x337D91B5);
    font.draw_text(
        canvas,
        18.0,
        96.0,
        "WORKSPACES",
        theme::COLOR_TEXT_MUTED,
        9.0,
    );

    let visible_count = workspace_count.min(MAX_VISIBLE_WORKSPACES);

    for index in 0..visible_count {
        let column = index % 5;
        let row = index / 5;
        let x = 16.0 + column as f32 * 65.0;
        let y = 112.0 + row as f32 * 48.0;
        let active = index == active_workspace;

        canvas.rounded_rect_f(
            x,
            y,
            57.0,
            40.0,
            11.0,
            if active { ACTIVE_BG } else { CARD_BG },
        );

        canvas.rounded_rect_f(x + 1.0, y + 1.0, 55.0, 1.0, 0.5, CARD_HIGHLIGHT);

        let label = format!("{}", index + 1);
        let label_width = font.text_width(&label, 14.0);
        font.draw_text(
            canvas,
            x + (57.0 - label_width) * 0.5,
            y + 25.0,
            &label,
            theme::COLOR_TEXT,
            14.0,
        );

        if active {
            canvas.rounded_rect_f(x + 23.0, y + 32.0, 11.0, 2.0, 1.0, 0xFFD9E8FF);
        }
    }

    if workspace_count > MAX_VISIBLE_WORKSPACES {
        font.draw_text(
            canvas,
            18.0,
            269.0,
            &format!("{} workspaces total", workspace_count),
            theme::COLOR_TEXT_MUTED,
            9.0,
        );
    }

    canvas.rounded_rect_f(16.0, 285.0, w - 32.0, 42.0, 12.0, CARD_BG);
    canvas.rounded_rect_f(17.0, 286.0, w - 34.0, 1.0, 0.5, CARD_HIGHLIGHT);
    font.draw_text(
        canvas,
        29.0,
        311.0,
        "+  Add workspace",
        theme::COLOR_TEXT,
        12.0,
    );

    canvas.horizontal_line(16, 340, width.saturating_sub(32), 0x337D91B5);
    font.draw_text(
        canvas,
        18.0,
        361.0,
        "SYSTEM CONTROLS",
        theme::COLOR_TEXT_MUTED,
        9.0,
    );

    let services = [
        ("Network", "Not connected"),
        ("Audio", "Not connected"),
        ("Bluetooth", "Not connected"),
        ("Display", "Not connected"),
    ];

    for (index, (title, status)) in services.iter().enumerate() {
        let column = index % 2;
        let row = index / 2;
        let x = 16.0 + column as f32 * 165.0;
        let y = 372.0 + row as f32 * 30.0;

        canvas.rounded_rect_f(x, y, 157.0, 25.0, 8.0, CARD_BG);
        font.draw_text(canvas, x + 9.0, y + 11.0, title, theme::COLOR_TEXT, 9.0);
        font.draw_text(
            canvas,
            x + 9.0,
            y + 21.0,
            status,
            theme::COLOR_TEXT_MUTED,
            7.0,
        );
    }

    if h >= 430.0 {
        font.draw_text(
            canvas,
            18.0,
            h - 7.0,
            "SYSTEM SERVICE INTEGRATION PENDING",
            theme::COLOR_TEXT_MUTED,
            7.5,
        );
    }
}

pub fn hit_test(
    width: u32,
    height: u32,
    x: f64,
    y: f64,
    workspace_count: u32,
) -> Option<ControlAction> {
    if x >= width.saturating_sub(66) as f64
        && x < width.saturating_sub(18) as f64
        && (13.0..40.0).contains(&y)
    {
        return Some(ControlAction::Close);
    }

    if (16.0..(width as f64 - 16.0)).contains(&x) && (112.0..256.0).contains(&y) {
        let column = ((x - 16.0) / 65.0) as u32;
        let row = ((y - 112.0) / 48.0) as u32;
        let local_x = (x - 16.0) - column as f64 * 65.0;
        let local_y = (y - 112.0) - row as f64 * 48.0;
        let index = row * 5 + column;

        if column < 5
            && row < 3
            && local_x < 57.0
            && local_y < 40.0
            && index < workspace_count.min(MAX_VISIBLE_WORKSPACES)
        {
            return Some(ControlAction::SwitchWorkspace(index));
        }
    }

    if x >= 16.0 && x < width.saturating_sub(16) as f64 && (285.0..327.0).contains(&y) {
        return Some(ControlAction::CreateWorkspace);
    }

    None
}
