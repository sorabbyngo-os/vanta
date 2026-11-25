use std::time::{Duration, Instant};

use super::{font::FontRenderer, primitives::Canvas, theme::Theme};
use vanta_apps::Application;
use vanta_ipc::MinimizedWindowInfo;

const ICON_SIZE: f32 = 44.0;
const ICON_GAP: f32 = 6.0;
const DOCK_PADDING: f32 = 8.0;
const ICON_COUNT: u32 = 5;

const APP_DOCK_Y: f32 = 8.0;
const APP_DOCK_RADIUS: f32 = 18.0;

const CARD_HEIGHT: f32 = 40.0;
const CARD_MIN_WIDTH: f32 = 116.0;
const CARD_MAX_WIDTH: f32 = 200.0;
const CARD_GAP: f32 = 8.0;
const RIGHT_MARGIN: f32 = 14.0;

/// Cached static dock artwork. Rebuilt only when its theme colors change.
pub struct IconCache {
    text_color: u32,
    muted_color: u32,
    icons: [Vec<u8>; ICON_COUNT as usize],
}

impl IconCache {
    pub fn new(colors: &Theme) -> Self {
        let icons = std::array::from_fn(|index| {
            let mut pixels = vec![0u8; ICON_SIZE as usize * ICON_SIZE as usize * 4];

            {
                let mut canvas = Canvas::new(&mut pixels, ICON_SIZE as u32, ICON_SIZE as u32);

                draw_icon(&mut canvas, index as u32, 0.0, 0.0, colors);
            }

            pixels
        });

        Self {
            text_color: colors.text,
            muted_color: colors.text_muted,
            icons,
        }
    }

    fn refresh_if_needed(&mut self, colors: &Theme) {
        if self.text_color != colors.text || self.muted_color != colors.text_muted {
            *self = Self::new(colors);
        }
    }
}

/// Cached shadow and glass artwork for the application dock.
pub struct DockBackgroundCache {
    width: u32,
    height: u32,
    dock_width_bits: u32,
    dock_height_bits: u32,
    dock_color: u32,
    edge_color: u32,
    pixels: Vec<u8>,
}

impl DockBackgroundCache {
    pub fn new(dock_width: f32, dock_height: f32, colors: &Theme) -> Self {
        let origin_x = 0u32;
        let origin_y = APP_DOCK_Y.floor() as u32;

        let right = dock_width + 4.0;
        let bottom = APP_DOCK_Y + dock_height + 6.0;

        let width = (right.ceil() as u32).saturating_sub(origin_x);
        let height = (bottom.ceil() as u32).saturating_sub(origin_y);

        let mut pixels = vec![0u8; width as usize * height as usize * 4];

        {
            let mut canvas = Canvas::new(&mut pixels, width, height);
            canvas.clear(0);

            canvas.rounded_rect_f(
                0.0,
                APP_DOCK_Y + 3.0 - origin_y as f32,
                dock_width + 4.0,
                dock_height + 3.0,
                APP_DOCK_RADIUS + 2.0,
                0x18000000,
            );

            canvas.rounded_rect_f(
                2.0,
                APP_DOCK_Y - origin_y as f32,
                dock_width,
                dock_height,
                APP_DOCK_RADIUS,
                colors.dock,
            );

            canvas.rounded_rect_f(
                2.75,
                APP_DOCK_Y + 0.75 - origin_y as f32,
                dock_width - 1.5,
                dock_height - 1.5,
                APP_DOCK_RADIUS - 0.75,
                colors.glass_edge,
            );
        }

        Self {
            width,
            height,
            dock_width_bits: dock_width.to_bits(),
            dock_height_bits: dock_height.to_bits(),
            dock_color: colors.dock,
            edge_color: colors.glass_edge,
            pixels,
        }
    }

    fn refresh_if_needed(&mut self, dock_width: f32, dock_height: f32, colors: &Theme) {
        if self.dock_width_bits != dock_width.to_bits()
            || self.dock_height_bits != dock_height.to_bits()
            || self.dock_color != colors.dock
            || self.edge_color != colors.glass_edge
        {
            *self = Self::new(dock_width, dock_height, colors);
        }
    }
}

fn visible_application_count(applications: &[Application], width: u32) -> usize {
    let available_width = width as f32 * 0.70;
    let total_slots = ((available_width - 2.0 * DOCK_PADDING + ICON_GAP) / (ICON_SIZE + ICON_GAP))
        .floor()
        .max(1.0) as usize;

    applications
        .iter()
        .filter(|app| is_dock_application(app))
        .count()
        .min(total_slots.saturating_sub(1))
}

fn app_dock_width(applications: &[Application], width: u32) -> f32 {
    let slots = visible_application_count(applications, width) + 1;
    slots as f32 * (ICON_SIZE + ICON_GAP) - ICON_GAP + 2.0 * DOCK_PADDING
}

pub fn is_dock_application(application: &Application) -> bool {
    !application.terminal
        && !application
            .categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case("ConsoleOnly"))
        && (application.exec.is_some() || application.dbus_activatable)
}

fn app_dock_x(width: u32, applications: &[Application]) -> f32 {
    ((width as f32 - app_dock_width(applications, width)) * 0.5).max(0.0)
}

fn icon_slot_x(index: usize, dock_x: f32) -> f32 {
    dock_x + DOCK_PADDING + index as f32 * (ICON_SIZE + ICON_GAP)
}

pub fn dock_application_at(
    x: f64,
    y: f64,
    applications: &[Application],
    width: u32,
) -> Option<usize> {
    let icon_y = APP_DOCK_Y + (super::theme::DOCK_HEIGHT as f32 - ICON_SIZE) * 0.5;
    let dock_x = app_dock_x(width, applications);

    (0..visible_application_count(applications, width)).find(|&index| {
        let icon_x = icon_slot_x(index, dock_x);
        x >= icon_x as f64
            && x < (icon_x + ICON_SIZE) as f64
            && y >= icon_y as f64
            && y < (icon_y + ICON_SIZE) as f64
    })
}

pub fn application_grid_icon_at(x: f64, y: f64, applications: &[Application], width: u32) -> bool {
    let dock_x = app_dock_x(width, applications);
    let icon_x = icon_slot_x(visible_application_count(applications, width), dock_x);
    let icon_y = APP_DOCK_Y + (super::theme::DOCK_HEIGHT as f32 - ICON_SIZE) * 0.5;

    x >= icon_x as f64
        && x < (icon_x + ICON_SIZE) as f64
        && y >= icon_y as f64
        && y < (icon_y + ICON_SIZE) as f64
}

fn card_layout(
    width: u32,
    height: u32,
    windows: &[MinimizedWindowInfo],
    applications: &[Application],
) -> (usize, usize, f32, f32, f32) {
    let start_x = app_dock_x(width, applications) + app_dock_width(applications, width) + 12.0;
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
    applications: &[Application],
    x: f64,
    y: f64,
) -> Option<u64> {
    let (first, visible, start_x, card_y, card_width) =
        card_layout(width, height, windows, applications);

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
pub fn workspace_at(
    width: u32,
    height: u32,
    workspace_count: u32,
    applications: &[Application],
    x: f64,
    y: f64,
) -> Option<u32> {
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
    let dock_x = app_dock_x(width, applications);
    let start_x = dock_x + (app_dock_width(applications, width) - total_width) * 0.5;
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
    applications: &[Application],
    colors: &Theme,
    icon_cache: &mut Option<IconCache>,
    background_cache: &mut Option<DockBackgroundCache>,
    measure: bool,
) -> [Duration; 5] {
    // Timings: cache construction, glass blit, icons, workspaces, minimized cards.
    let mut timings = [Duration::ZERO; 5];

    if width < 320 || height < super::theme::DOCK_HEIGHT + 8 {
        return timings;
    }

    let dock_width = app_dock_width(applications, width);
    let dock_x = app_dock_x(width, applications);
    let dock_height = super::theme::DOCK_HEIGHT as f32;
    let dock_y = APP_DOCK_Y;

    // App dock glass background. Separate cache work from recurring compositing.
    if dock_width <= width as f32 * 0.70 {
        let cache_started = measure.then(Instant::now);

        let cache = background_cache
            .get_or_insert_with(|| DockBackgroundCache::new(dock_width, dock_height, colors));
        cache.refresh_if_needed(dock_width, dock_height, colors);

        if let Some(started) = cache_started {
            timings[0] = started.elapsed();
        }

        let blit_started = measure.then(Instant::now);

        canvas.blit_premultiplied(
            &cache.pixels,
            cache.width,
            cache.height,
            (dock_x - 2.0).floor().max(0.0) as u32,
            APP_DOCK_Y.floor() as u32,
        );

        if let Some(started) = blit_started {
            timings[1] = started.elapsed();
        }
    }

    let mut stage_started = measure.then(Instant::now);

    // Render only real installed applications in the application slots.
    if dock_width <= width as f32 * 0.70 {
        let cache = icon_cache.get_or_insert_with(|| IconCache::new(colors));
        cache.refresh_if_needed(colors);

        let visible_count = visible_application_count(applications, width);
        let y = dock_y + (dock_height - ICON_SIZE) * 0.5;

        for (index, application) in applications
            .iter()
            .filter(|application| is_dock_application(application))
            .take(visible_count)
            .enumerate()
        {
            let x = icon_slot_x(index, dock_x);

            if let Some(icon) = super::icon_cache::get_icon(application.icon.as_deref()) {
                let icon_x = x + (ICON_SIZE - icon.size as f32) * 0.5;
                let icon_y = y + (ICON_SIZE - icon.size as f32) * 0.5;

                canvas.blit_premultiplied(
                    icon.pixels.as_ref(),
                    icon.size,
                    icon.size,
                    icon_x.max(0.0) as u32,
                    icon_y.max(0.0) as u32,
                );
            }

            if index == 0 {
                canvas.rounded_rect_f(
                    x + ICON_SIZE * 0.5 - 3.0,
                    dock_y + dock_height - 5.0,
                    6.0,
                    2.5,
                    1.25,
                    colors.accent,
                );
            }
        }

        // Dedicated application-grid launcher, after the executable apps.
        let grid_x = icon_slot_x(visible_count, dock_x);
        canvas.blit_premultiplied(
            &cache.icons[(ICON_COUNT - 1) as usize],
            ICON_SIZE as u32,
            ICON_SIZE as u32,
            grid_x as u32,
            y as u32,
        );
    }

    if let Some(started) = stage_started {
        timings[3] = started.elapsed();
        stage_started = Some(Instant::now());
    }

    if dock_width <= width as f32 * 0.70 {
        draw_workspaces(
            canvas,
            height,
            workspace_count,
            active_workspace,
            applications,
            width,
            colors,
        );
    }

    if let Some(started) = stage_started {
        timings[2] = started.elapsed();
        stage_started = Some(Instant::now());
    }

    // Minimized windows: right side, vertically aligned with the app dock.
    let (first, visible, start_x, card_y, card_width) =
        card_layout(width, height, minimized_windows, applications);

    for offset in 0..visible {
        let window = &minimized_windows[first + offset];
        let x = start_x + offset as f32 * (card_width + CARD_GAP);

        canvas.rounded_rect_f(x, card_y + 2.0, card_width, CARD_HEIGHT, 13.0, 0x18000000);

        canvas.rounded_rect_f(x, card_y, card_width, CARD_HEIGHT, 13.0, colors.dock);

        canvas.rounded_rect_f(
            x + 0.75,
            card_y + 0.75,
            card_width - 1.5,
            CARD_HEIGHT - 1.5,
            12.25,
            colors.glass_edge,
        );

        canvas.rounded_rect_f(x + 10.0, card_y + 14.0, 5.0, 12.0, 2.5, colors.accent);

        let title = window
            .title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or(window.app_id.as_deref())
            .unwrap_or("Minimized window");

        let available_text_width = card_width - 36.0;
        let title = fit_title(font, title, available_text_width, 13.0);

        font.draw_text(canvas, x + 23.0, card_y + 25.0, &title, colors.text, 13.0);
    }

    if let Some(started) = stage_started {
        timings[4] = started.elapsed();
    }

    timings
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
    applications: &[Application],
    width: u32,
    colors: &Theme,
) {
    const DOT_SIZE: f32 = 4.0;
    const DOT_GAP: f32 = 5.0;
    const MAX_VISIBLE: u32 = 15;

    let count = workspace_count.clamp(1, MAX_VISIBLE);
    let total_width = count as f32 * DOT_SIZE + (count - 1) as f32 * DOT_GAP;
    let dock_x = app_dock_x(width, applications);
    let start_x = dock_x + (app_dock_width(applications, width) - total_width) * 0.5;
    let y = height as f32 - 6.0;

    for index in 0..count {
        let x = start_x + index as f32 * (DOT_SIZE + DOT_GAP);
        let color = if index == active_workspace {
            colors.accent
        } else {
            colors.text_muted
        };

        canvas.rounded_rect_f(x, y, DOT_SIZE, DOT_SIZE, 2.0, color);
    }
}

fn draw_icon(canvas: &mut Canvas<'_>, index: u32, x: f32, y: f32, colors: &Theme) {
    let white = colors.text;
    let dark = 0xFF263247;
    let muted = colors.text_muted;

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
