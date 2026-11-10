use super::primitives::Canvas;
use ab_glyph::{point, Font, FontArc, ScaleFont};

const PRIMARY_FONT_PATH: &str = "/usr/share/fonts/adwaita-sans-fonts/AdwaitaSans-Regular.ttf";

/// Scalable system-font renderer for Vanta's software-composited surfaces.
pub struct FontRenderer {
    font: FontArc,
}

impl FontRenderer {
    /// Load the desktop's primary UI font once and retain it for subsequent frames.
    pub fn load() -> Result<Self, String> {
        let data = std::fs::read(PRIMARY_FONT_PATH)
            .map_err(|error| format!("failed to read {PRIMARY_FONT_PATH}: {error}"))?;
        let font = FontArc::try_from_vec(data)
            .map_err(|_| format!("invalid or unsupported font file: {PRIMARY_FONT_PATH}"))?;

        Ok(Self { font })
    }

    /// Draw text at the supplied baseline using antialiased glyph coverage.
    /// Coordinates and font size are in logical surface pixels.
    /// Color format: 0xAARRGGBB.
    pub fn draw_text(
        &self,
        canvas: &mut Canvas<'_>,
        x: f32,
        baseline: f32,
        text: &str,
        color: u32,
        size: f32,
    ) {
        let size = size.max(1.0);
        let scaled = self.font.as_scaled(size);
        let mut cursor_x = x;
        let mut previous_glyph = None;

        for character in text.chars() {
            let glyph_id = scaled.glyph_id(character);

            if let Some(previous) = previous_glyph {
                cursor_x += scaled.kern(previous, glyph_id);
            }

            let glyph = glyph_id.with_scale_and_position(size, point(cursor_x, baseline));
            if let Some(outlined) = self.font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                let origin_x = bounds.min.x.floor() as i32;
                let origin_y = bounds.min.y.floor() as i32;

                outlined.draw(|glyph_x, glyph_y, coverage| {
                    canvas.blend_coverage(
                        origin_x + glyph_x as i32,
                        origin_y + glyph_y as i32,
                        color,
                        coverage,
                    );
                });
            }

            cursor_x += scaled.h_advance(glyph_id);
            previous_glyph = Some(glyph_id);
        }
    }

    /// Measure a single line with the same advances and kerning used to draw it.
    pub fn text_width(&self, text: &str, size: f32) -> f32 {
        let scaled = self.font.as_scaled(size.max(1.0));
        let mut width = 0.0;
        let mut previous_glyph = None;

        for character in text.chars() {
            let glyph_id = scaled.glyph_id(character);

            if let Some(previous) = previous_glyph {
                width += scaled.kern(previous, glyph_id);
            }

            width += scaled.h_advance(glyph_id);
            previous_glyph = Some(glyph_id);
        }

        width
    }
}
