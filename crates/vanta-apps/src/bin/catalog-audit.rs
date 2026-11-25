use vanta_apps::ApplicationCatalog;

fn main() {
    let catalog = match ApplicationCatalog::discover() {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("Catalog discovery failed: {error}");
            return;
        }
    };

    println!("Vanta application catalog");
    println!("=========================");
    println!("Discovered applications: {}", catalog.applications.len());
    println!();

    for app in &catalog.applications {
        println!("{} ({})", app.name, app.id);
        println!("  Icon: {}", app.icon.as_deref().unwrap_or("(none)"));
        println!(
            "  Categories: {}",
            if app.categories.is_empty() {
                "(none)".to_owned()
            } else {
                app.categories.join(", ")
            }
        );
        println!(
            "  Launch method: {}",
            if app.dbus_activatable {
                "D-Bus capable"
            } else {
                "Exec command"
            }
        );
        println!("  Desktop file: {}", app.desktop_file.display());
        println!();
    }
}
