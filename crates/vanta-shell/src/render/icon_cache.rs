use png::{ColorType, Decoder, Transformations};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, BufReader},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
};

use smithay_client_toolkit::reexports::calloop::channel;

const ICON_SIZE: u32 = 38;

#[derive(Clone)]
pub struct IconPixels {
    pub pixels: Arc<[u8]>,
    pub size: u32,
}

pub struct IconReady;

static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<IconPixels>>>>> = OnceLock::new();

static PENDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

static JOB_SENDER: OnceLock<mpsc::Sender<String>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Option<Arc<IconPixels>>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pending() -> &'static Mutex<HashSet<String>> {
    PENDING.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn initialize(event_sender: channel::Sender<IconReady>) -> io::Result<()> {
    let (job_sender, job_receiver) = mpsc::channel::<String>();

    JOB_SENDER.set(job_sender).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "icon worker already initialized",
        )
    })?;

    thread::Builder::new()
        .name("vanta-icon-loader".into())
        .spawn(move || {
            while let Ok(icon) = job_receiver.recv() {
                let loaded = resolve_icon(&icon).and_then(load_icon);

                if let Ok(mut guard) = cache().lock() {
                    guard.insert(icon.clone(), loaded);
                }

                if let Ok(mut guard) = pending().lock() {
                    guard.remove(&icon);
                }

                let _ = event_sender.send(IconReady);
            }
        })?;

    Ok(())
}

pub fn get_icon(icon: Option<&str>) -> Option<Arc<IconPixels>> {
    let icon = icon?.trim();

    if icon.is_empty() {
        return None;
    }

    if let Ok(guard) = cache().lock() {
        if let Some(cached) = guard.get(icon) {
            return cached.clone();
        }
    }

    let sender = JOB_SENDER.get()?;

    let should_queue = {
        let mut guard = pending().lock().ok()?;
        guard.insert(icon.to_owned())
    };

    if !should_queue {
        return None;
    }

    if sender.send(icon.to_owned()).is_err() {
        if let Ok(mut guard) = pending().lock() {
            guard.remove(icon);
        }
    }

    None
}

fn resolve_icon(icon: &str) -> Option<PathBuf> {
    let direct = Path::new(icon);

    if direct.is_absolute() && direct.is_file() {
        return Some(direct.to_path_buf());
    }

    let filename = direct.file_name()?.to_str()?;
    let stem = filename
        .strip_suffix(".png")
        .or_else(|| filename.strip_suffix(".svg"))
        .unwrap_or(filename);

    let mut roots = Vec::new();

    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(&home).join(".local/share/icons"));
        roots.push(PathBuf::from(&home).join(".icons"));
        roots.push(PathBuf::from(&home).join(".local/share/pixmaps"));
    }

    roots.push(PathBuf::from("/usr/local/share/icons"));
    roots.push(PathBuf::from("/usr/share/icons"));
    roots.push(PathBuf::from("/usr/share/pixmaps"));

    let themes = [
        "Adwaita",
        "AdwaitaLegacy",
        "hicolor",
        "breeze",
        "breeze-dark",
        "oxygen",
        "default",
    ];

    let sizes = [
        "scalable", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "24x24", "22x22",
        "16x16",
    ];

    let extensions = ["svg", "png"];

    for root in roots {
        if root.ends_with("pixmaps") {
            for ext in extensions {
                let candidate = root.join(format!("{stem}.{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }

            let candidate = root.join(filename);
            if candidate.is_file() {
                return Some(candidate);
            }

            continue;
        }

        for theme in themes {
            for size in sizes {
                for ext in extensions {
                    let candidate = root
                        .join(theme)
                        .join(size)
                        .join("apps")
                        .join(format!("{stem}.{ext}"));

                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    None
}

fn load_icon(path: PathBuf) -> Option<Arc<IconPixels>> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => decode_png(path),
        "svg" => decode_svg(path),
        _ => None,
    }
}

fn decode_png(path: PathBuf) -> Option<Arc<IconPixels>> {
    let file = File::open(path).ok()?;
    let mut decoder = Decoder::new(BufReader::new(file));

    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);

    let mut reader = decoder.read_info().ok()?;
    let buffer_size = reader.output_buffer_size()?;
    let mut buffer = vec![0; buffer_size];

    let info = reader.next_frame(&mut buffer).ok()?;

    if info.width == 0 || info.height == 0 || info.width > 2048 || info.height > 2048 {
        return None;
    }

    let source = &buffer[..info.buffer_size()];

    let channels = match info.color_type {
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Indexed => return None,
    };

    let expected = info.width as usize * info.height as usize * channels;

    if source.len() < expected {
        return None;
    }

    let mut bgra = Vec::with_capacity(info.width as usize * info.height as usize * 4);

    for pixel in source.chunks_exact(channels) {
        let (r, g, b, a) = match info.color_type {
            ColorType::Rgb => (pixel[0], pixel[1], pixel[2], 255),
            ColorType::Rgba => (pixel[0], pixel[1], pixel[2], pixel[3]),
            ColorType::Grayscale => (pixel[0], pixel[0], pixel[0], 255),
            ColorType::GrayscaleAlpha => (pixel[0], pixel[0], pixel[0], pixel[1]),
            ColorType::Indexed => unreachable!(),
        };

        let alpha = a as u16;

        bgra.extend_from_slice(&[
            ((b as u16 * alpha) / 255) as u8,
            ((g as u16 * alpha) / 255) as u8,
            ((r as u16 * alpha) / 255) as u8,
            a,
        ]);
    }

    Some(Arc::new(IconPixels {
        pixels: scale_bgra(&bgra, info.width, info.height).into(),
        size: ICON_SIZE,
    }))
}

fn decode_svg(path: PathBuf) -> Option<Arc<IconPixels>> {
    let data = std::fs::read(path).ok()?;
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(&data, &options).ok()?;

    let svg_size = tree.size();
    let width = svg_size.width();
    let height = svg_size.height();

    if width <= 0.0 || height <= 0.0 || width > 4096.0 || height > 4096.0 {
        return None;
    }

    let scale = (ICON_SIZE as f32 / width).min(ICON_SIZE as f32 / height);
    let offset_x = (ICON_SIZE as f32 - width * scale) / 2.0;
    let offset_y = (ICON_SIZE as f32 - height * scale) / 2.0;

    let transform =
        resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(offset_x, offset_y);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(ICON_SIZE, ICON_SIZE)?;

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // tiny-skia produces premultiplied RGBA. Canvas expects premultiplied BGRA.
    let rgba = pixmap.data();
    let mut bgra = Vec::with_capacity(rgba.len());

    for pixel in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }

    Some(Arc::new(IconPixels {
        pixels: bgra.into(),
        size: ICON_SIZE,
    }))
}

fn scale_bgra(source: &[u8], source_width: u32, source_height: u32) -> Vec<u8> {
    let mut scaled = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];

    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let sx = x * source_width / ICON_SIZE;
            let sy = y * source_height / ICON_SIZE;

            let src = ((sy * source_width + sx) * 4) as usize;
            let dst = ((y * ICON_SIZE + x) * 4) as usize;

            scaled[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
        }
    }

    scaled
}
