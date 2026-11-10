pub struct Canvas<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
}

impl<'a> Canvas<'a> {
    pub fn new(pixels: &'a mut [u8], width: u32, height: u32) -> Self {
        Self {
            pixels,
            width,
            height,
        }
    }

    pub fn clear(&mut self, color: u32) {
        let alpha = ((color >> 24) & 0xFF) as u32;
        let red = ((color >> 16) & 0xFF) as u32;
        let green = ((color >> 8) & 0xFF) as u32;
        let blue = (color & 0xFF) as u32;

        let premultiply = |channel: u32| ((channel * alpha + 127) / 255) as u8;

        let pixel = [
            premultiply(blue),
            premultiply(green),
            premultiply(red),
            alpha as u8,
        ];

        for destination in self.pixels.chunks_exact_mut(4) {
            destination.copy_from_slice(&pixel);
        }
    }

    pub fn fill_rect(&mut self, x: u32, y: u32, width: u32, height: u32, color: u32) {
        let right = x.saturating_add(width).min(self.width);
        let bottom = y.saturating_add(height).min(self.height);

        for py in y.min(self.height)..bottom {
            for px in x.min(self.width)..right {
                self.blend_pixel(px, py, color);
            }
        }
    }

    pub fn rounded_rect(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        radius: u32,
        color: u32,
    ) {
        if width == 0 || height == 0 || x >= self.width || y >= self.height {
            return;
        }

        let visible_width = width.min(self.width - x);
        let visible_height = height.min(self.height - y);

        if visible_width == 0 || visible_height == 0 {
            return;
        }

        let radius = radius.min(width / 2).min(height / 2);

        if radius == 0 {
            self.fill_rect(x, y, visible_width, visible_height, color);
            return;
        }

        let Some(mut pixmap) = tiny_skia::Pixmap::new(visible_width, visible_height) else {
            return;
        };

        let left = 0.0_f32;
        let top = 0.0_f32;
        let right = width as f32;
        let bottom = height as f32;
        let radius = radius as f32;
        let tangent = radius * 0.552_284_8;

        let mut builder = tiny_skia::PathBuilder::new();

        builder.move_to(left + radius, top);
        builder.line_to(right - radius, top);
        builder.cubic_to(
            right - radius + tangent,
            top,
            right,
            top + radius - tangent,
            right,
            top + radius,
        );
        builder.line_to(right, bottom - radius);
        builder.cubic_to(
            right,
            bottom - radius + tangent,
            right - radius + tangent,
            bottom,
            right - radius,
            bottom,
        );
        builder.line_to(left + radius, bottom);
        builder.cubic_to(
            left + radius - tangent,
            bottom,
            left,
            bottom - radius + tangent,
            left,
            bottom - radius,
        );
        builder.line_to(left, top + radius);
        builder.cubic_to(
            left,
            top + radius - tangent,
            left + radius - tangent,
            top,
            left + radius,
            top,
        );
        builder.close();

        let Some(path) = builder.finish() else {
            return;
        };

        let alpha = ((color >> 24) & 0xFF) as u8;
        let red = ((color >> 16) & 0xFF) as u8;
        let green = ((color >> 8) & 0xFF) as u8;
        let blue = (color & 0xFF) as u8;

        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(red, green, blue, alpha);
        paint.anti_alias = true;

        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );

        for (pixel_index, source_pixel) in pixmap.data().chunks_exact(4).enumerate() {
            let source_red = source_pixel[0] as u32;
            let source_green = source_pixel[1] as u32;
            let source_blue = source_pixel[2] as u32;
            let source_alpha = source_pixel[3] as u32;

            if source_alpha == 0 {
                continue;
            }

            let local_x = pixel_index as u32 % visible_width;
            let local_y = pixel_index as u32 / visible_width;
            let destination_index = (((y + local_y) * self.width + (x + local_x)) * 4) as usize;

            if destination_index + 3 >= self.pixels.len() {
                continue;
            }

            let inverse = 255 - source_alpha;

            let destination_blue = self.pixels[destination_index] as u32;
            let destination_green = self.pixels[destination_index + 1] as u32;
            let destination_red = self.pixels[destination_index + 2] as u32;
            let destination_alpha = self.pixels[destination_index + 3] as u32;

            self.pixels[destination_index] =
                (source_blue + (destination_blue * inverse + 127) / 255).min(255) as u8;
            self.pixels[destination_index + 1] =
                (source_green + (destination_green * inverse + 127) / 255).min(255) as u8;
            self.pixels[destination_index + 2] =
                (source_red + (destination_red * inverse + 127) / 255).min(255) as u8;
            self.pixels[destination_index + 3] =
                (source_alpha + (destination_alpha * inverse + 127) / 255).min(255) as u8;
        }
    }

    /// Blend an antialiased coverage sample into the premultiplied BGRA canvas.
    pub fn blend_coverage(&mut self, x: i32, y: i32, color: u32, coverage: f32) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }

        let coverage = coverage.clamp(0.0, 1.0);
        if coverage <= 0.0 {
            return;
        }

        let alpha = ((color >> 24) & 0xFF) as f32;
        let effective_alpha = (alpha * coverage).round() as u32;

        if effective_alpha == 0 {
            return;
        }

        let covered_color = (color & 0x00FF_FFFF) | (effective_alpha << 24);
        self.blend_pixel(x as u32, y as u32, covered_color);
    }

    /// Draw an axis-aligned rectangle with fractional coordinates.
    /// Coverage is calculated per pixel, preserving antialiased edges.
    pub fn fill_rect_f(&mut self, x: f32, y: f32, width: f32, height: f32, color: u32) {
        if !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            return;
        }

        let right = x + width;
        let bottom = y + height;

        if right <= 0.0 || bottom <= 0.0 || x >= self.width as f32 || y >= self.height as f32 {
            return;
        }

        let start_x = x.floor().max(0.0) as u32;
        let start_y = y.floor().max(0.0) as u32;
        let end_x = right.ceil().min(self.width as f32) as u32;
        let end_y = bottom.ceil().min(self.height as f32) as u32;

        for py in start_y..end_y {
            let coverage_y = (bottom.min(py as f32 + 1.0) - y.max(py as f32)).clamp(0.0, 1.0);

            for px in start_x..end_x {
                let coverage_x = (right.min(px as f32 + 1.0) - x.max(px as f32)).clamp(0.0, 1.0);

                self.blend_coverage(px as i32, py as i32, color, coverage_x * coverage_y);
            }
        }
    }

    /// Draw a rounded rectangle using fractional geometry and 4x4 coverage
    /// sampling. This avoids temporary pixmap allocations for individual shapes.
    pub fn rounded_rect_f(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        color: u32,
    ) {
        if !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || !radius.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            return;
        }

        let right = x + width;
        let bottom = y + height;

        if right <= 0.0 || bottom <= 0.0 || x >= self.width as f32 || y >= self.height as f32 {
            return;
        }

        let radius = radius.max(0.0).min(width * 0.5).min(height * 0.5);

        if radius <= f32::EPSILON {
            self.fill_rect_f(x, y, width, height, color);
            return;
        }

        let start_x = x.floor().max(0.0) as u32;
        let start_y = y.floor().max(0.0) as u32;
        let end_x = right.ceil().min(self.width as f32) as u32;
        let end_y = bottom.ceil().min(self.height as f32) as u32;
        const SAMPLE_OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];

        let inner_left = x + radius;
        let inner_right = right - radius;
        let inner_top = y + radius;
        let inner_bottom = bottom - radius;
        let radius_squared = radius * radius;

        for py in start_y..end_y {
            for px in start_x..end_x {
                let mut covered = 0_u32;

                for offset_y in SAMPLE_OFFSETS {
                    let sample_y = py as f32 + offset_y;
                    if sample_y < y || sample_y > bottom {
                        continue;
                    }

                    let dy = if sample_y < inner_top {
                        inner_top - sample_y
                    } else if sample_y > inner_bottom {
                        sample_y - inner_bottom
                    } else {
                        0.0
                    };

                    for offset_x in SAMPLE_OFFSETS {
                        let sample_x = px as f32 + offset_x;

                        if sample_x < x || sample_x > right {
                            continue;
                        }

                        let dx = if sample_x < inner_left {
                            inner_left - sample_x
                        } else if sample_x > inner_right {
                            sample_x - inner_right
                        } else {
                            0.0
                        };

                        if dx * dx + dy * dy <= radius_squared {
                            covered += 1;
                        }
                    }
                }

                if covered != 0 {
                    self.blend_coverage(px as i32, py as i32, color, covered as f32 / 16.0);
                }
            }
        }
    }

    pub fn horizontal_line(&mut self, x: u32, y: u32, width: u32, color: u32) {
        self.fill_rect(x, y, width, 1, color);
    }

    fn blend_pixel(&mut self, x: u32, y: u32, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }

        let index = ((y * self.width + x) * 4) as usize;
        if index + 3 >= self.pixels.len() {
            return;
        }

        let alpha = ((color >> 24) & 0xFF) as u32;
        let inverse = 255 - alpha;

        let red = ((color >> 16) & 0xFF) as u32;
        let green = ((color >> 8) & 0xFF) as u32;
        let blue = (color & 0xFF) as u32;

        let source_red = (red * alpha + 127) / 255;
        let source_green = (green * alpha + 127) / 255;
        let source_blue = (blue * alpha + 127) / 255;

        let destination_blue = self.pixels[index] as u32;
        let destination_green = self.pixels[index + 1] as u32;
        let destination_red = self.pixels[index + 2] as u32;
        let destination_alpha = self.pixels[index + 3] as u32;

        self.pixels[index] =
            (source_blue + (destination_blue * inverse + 127) / 255).min(255) as u8;
        self.pixels[index + 1] =
            (source_green + (destination_green * inverse + 127) / 255).min(255) as u8;
        self.pixels[index + 2] =
            (source_red + (destination_red * inverse + 127) / 255).min(255) as u8;
        self.pixels[index + 3] = (alpha + (destination_alpha * inverse + 127) / 255).min(255) as u8;
    }
}
