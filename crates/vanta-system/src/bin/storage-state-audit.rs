use vanta_system::storage_state::StorageStateManager;
use vanta_system::SystemBus;

fn main() {
    println!("Vanta Storage State Audit");
    println!("========================");

    let bus = match SystemBus::connect() {
        Ok(bus) => bus,
        Err(error) => {
            eprintln!("System bus connection failed: {error}");
            return;
        }
    };

    let manager = match StorageStateManager::start(&bus) {
        Ok(manager) => manager,
        Err(error) => {
            eprintln!("Storage state initialization failed: {error}");
            return;
        }
    };

    let updates = manager.subscribe_updates();
    let initial = manager.view();

    println!("Initial revision: {}", initial.revision);
    println!("Drives: {}", initial.snapshot.drives.len());
    println!("Block devices: {}", initial.snapshot.block_devices.len());

    for drive in &initial.snapshot.drives {
        println!(
            "DRIVE: {} | {} | {} bytes",
            drive.vendor.as_deref().unwrap_or("Unknown vendor"),
            drive.model.as_deref().unwrap_or("Unknown model"),
            drive.size_bytes
        );
    }

    for device in &initial.snapshot.block_devices {
        println!(
            "BLOCK: {} | {} | {} bytes | filesystem: {} | mounted: {}",
            device.device,
            device.id.as_deref().unwrap_or("Unknown"),
            device.size_bytes,
            device.filesystem_type.as_deref().unwrap_or("Unknown"),
            device
                .filesystem
                .as_ref()
                .map(|fs| !fs.mount_points.is_empty())
                .unwrap_or(false)
        );
    }

    println!("\nWatching for live storage updates. Connect/disconnect a device.");
    println!("Press Ctrl+C to stop.\n");

    loop {
        match updates.recv() {
            Ok(revision) => {
                let view = manager.view();

                println!("\n--- Storage state updated ---");
                println!("Revision: {revision}");
                println!("Drives: {}", view.snapshot.drives.len());
                println!("Block devices: {}", view.snapshot.block_devices.len());

                if let Some(error) = view.last_error {
                    println!("Monitor/refresh error: {error}");
                }

                for device in &view.snapshot.block_devices {
                    println!(
                        "{} | {} | {} bytes",
                        device.device,
                        device
                            .filesystem_type
                            .as_deref()
                            .unwrap_or("Unknown filesystem"),
                        device.size_bytes
                    );
                }
            }
            Err(error) => {
                eprintln!("Storage state update channel closed: {error}");
                break;
            }
        }
    }
}
