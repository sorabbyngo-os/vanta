use vanta_system::SystemBus;

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    format!("{value:.2} {}", UNITS[unit])
}

fn main() {
    let bus = SystemBus::connect().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1);
    });

    let storage = bus.storage_snapshot().unwrap_or_else(|error| {
        eprintln!("Storage discovery failed: {error}");
        std::process::exit(1);
    });

    println!("\nVANTA STORAGE AUDIT");
    println!("================================");

    println!("\nPhysical drives: {}", storage.drives.len());

    for drive in &storage.drives {
        println!(
            "\nDrive: {}",
            drive.model.as_deref().unwrap_or("Unknown model")
        );
        println!("  Vendor: {}", drive.vendor.as_deref().unwrap_or("Unknown"));
        println!("  Size: {}", human_size(drive.size_bytes));
        println!(
            "  Bus: {}",
            drive.connection_bus.as_deref().unwrap_or("Unknown")
        );
        println!("  Media available: {:?}", drive.media_available);
        println!("  Removable: {:?}", drive.removable);
    }

    println!("\nBlock devices: {}", storage.block_devices.len());

    for block in &storage.block_devices {
        println!("\n{}", block.device);
        println!("  Size: {}", human_size(block.size_bytes));
        println!("  Read-only: {}", block.read_only);
        println!(
            "  Filesystem: {}",
            block.filesystem_type.as_deref().unwrap_or("None")
        );
        println!(
            "  Label: {}",
            block.filesystem_label.as_deref().unwrap_or("None")
        );
        println!(
            "  Filesystem UUID: {}",
            block.filesystem_uuid.as_deref().unwrap_or("None")
        );

        if let Some(partition) = &block.partition {
            println!("  Partition number: {}", partition.number);
            println!(
                "  Partition name: {}",
                partition.name.as_deref().unwrap_or("None")
            );
            println!("  Partition size: {}", human_size(partition.size_bytes));
            println!("  Partition offset: {}", human_size(partition.offset_bytes));
        }

        if let Some(filesystem) = &block.filesystem {
            println!(
                "  Mount points: {}",
                if filesystem.mount_points.is_empty() {
                    "None".to_string()
                } else {
                    filesystem.mount_points.join(", ")
                }
            );
        }

        if let Some(encryption) = &block.encryption {
            println!("  Encrypted mapping: {:?}", encryption.cleartext_device);
        }

        if let Some(table) = &block.partition_table {
            println!(
                "  Partition table: {}",
                table.table_type.as_deref().unwrap_or("Unknown")
            );
            println!("  Partitions: {}", table.partitions.len());
        }
    }

    println!("\n================================");
}
