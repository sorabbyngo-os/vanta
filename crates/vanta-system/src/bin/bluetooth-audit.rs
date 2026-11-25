use vanta_system::SystemBus;

fn main() {
    let bus = SystemBus::connect().unwrap_or_else(|error| {
        eprintln!("System D-Bus connection failed: {error}");
        std::process::exit(1);
    });

    let snapshot = bus.bluetooth_snapshot().unwrap_or_else(|error| {
        eprintln!("Bluetooth discovery failed: {error}");
        std::process::exit(1);
    });

    println!("\nVANTA BLUETOOTH AUDIT");
    println!("==============================================");

    println!("Adapters: {}", snapshot.adapters.len());

    for adapter in &snapshot.adapters {
        println!("\n  Adapter: {}", adapter.object_path);
        println!("  Address: {}", adapter.address);
        println!(
            "  Name: {}",
            adapter.name.as_deref().unwrap_or("Unavailable")
        );
        println!(
            "  Alias: {}",
            adapter.alias.as_deref().unwrap_or("Unavailable")
        );
        println!("  Address type: {:?}", adapter.address_type);
        println!("  Powered: {}", adapter.powered);
        println!("  Power state: {:?}", adapter.power_state);
        println!("  Discoverable: {:?}", adapter.discoverable);
        println!(
            "  Discoverable timeout: {:?}",
            adapter.discoverable_timeout_seconds
        );
        println!("  Pairable: {:?}", adapter.pairable);
        println!("  Pairable timeout: {:?}", adapter.pairable_timeout_seconds);
        println!("  Discovering: {:?}", adapter.discovering);
        println!("  Advertised UUIDs: {}", adapter.uuids.len());
    }

    println!("\nKnown devices: {}", snapshot.devices.len());

    for device in &snapshot.devices {
        println!("\n  Device: {}", device.object_path);
        println!("  Address: {}", device.address);
        println!(
            "  Name: {}",
            device.name.as_deref().unwrap_or("Unavailable")
        );
        println!(
            "  Alias: {}",
            device.alias.as_deref().unwrap_or("Unavailable")
        );
        println!("  Icon: {:?}", device.icon);
        println!("  Adapter: {}", device.adapter_path);
        println!("  Address type: {:?}", device.address_type);
        println!("  Paired: {}", device.paired);
        println!("  Bonded: {:?}", device.bonded);
        println!("  Trusted: {}", device.trusted);
        println!("  Connected: {}", device.connected);
        println!("  Blocked: {}", device.blocked);
        println!("  RSSI: {:?} dBm", device.rssi_dbm);
        println!("  TX power: {:?} dBm", device.tx_power_dbm);
        println!("  Services resolved: {:?}", device.services_resolved);
        println!("  Services: {}", device.uuids.len());
    }

    println!("\n==============================================");
}
