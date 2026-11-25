use std::{env, fs, fs::File, io::BufReader, path::PathBuf};

fn main() {
    let source = PathBuf::from("../../assets/wallpapers/vanta-default.png");
    println!("cargo:rerun-if-changed={}", source.display());

    let decoder = png::Decoder::new(BufReader::new(
        File::open(&source).expect("open Vanta wallpaper"),
    ));
    let mut reader = decoder.read_info().expect("read Vanta wallpaper metadata");
    let mut decoded = vec![
        0;
        reader
            .output_buffer_size()
            .expect("valid wallpaper dimensions")
    ];
    let info = reader
        .next_frame(&mut decoded)
        .expect("decode Vanta wallpaper");
    let pixels = &decoded[..info.buffer_size()];

    let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);

    match info.color_type {
        png::ColorType::Rgb => {
            for pixel in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Rgba => rgba.extend_from_slice(pixels),
        other => panic!("unsupported Vanta wallpaper color type: {other:?}"),
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    fs::write(out_dir.join("wallpaper.rgba"), rgba).expect("write converted wallpaper");

    let metadata = format!(
        "pub const WALLPAPER_WIDTH: i32 = {};\npub const WALLPAPER_HEIGHT: i32 = {};\n",
        info.width, info.height
    );
    fs::write(out_dir.join("wallpaper_meta.rs"), metadata).expect("write wallpaper metadata");

    println!(
        "cargo:warning=Preconverted Vanta wallpaper: {}x{} RGBA",
        info.width, info.height
    );
}
