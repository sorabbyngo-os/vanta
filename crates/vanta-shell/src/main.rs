//! Vanta shell — DE chrome (panel, later launcher).
//! Phase S0: process exists; connects to Vanta's WAYLAND_DISPLAY when set.

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "(unset)".into());
    tracing::info!("vanta-shell starting");
    tracing::info!(wayland_display = %wayland_display, "Wayland display");
    println!("vanta-shell S0: ok (panel/layer-shell next)");
    println!("  WAYLAND_DISPLAY={wayland_display}");
}
