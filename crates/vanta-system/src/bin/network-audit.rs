use vanta_system::{
    network::{NetworkDeviceType, NetworkSnapshot},
    SystemBus,
};

fn print_report(snapshot: &NetworkSnapshot) {
    println!("\nVANTA NETWORK AUDIT");
    println!("==============================================");

    println!("Manager state: {:?}", snapshot.manager_state);
    println!("Connectivity: {:?}", snapshot.connectivity);
    println!("Networking enabled: {}", snapshot.networking_enabled);
    println!("Wi-Fi enabled: {}", snapshot.wifi_enabled);
    println!(
        "Wi-Fi hardware enabled: {:?}",
        snapshot.wifi_hardware_enabled
    );

    println!("\nDevices: {}", snapshot.devices.len());

    for device in &snapshot.devices {
        println!("\n  Interface: {}", device.interface);
        println!("  Type: {:?}", device.device_type);
        println!("  Raw type: {}", device.raw_device_type);
        println!("  State: {:?}", device.state);
        println!("  Physical link: {:?}", device.link_connected);
        println!("  Raw state: {}", device.raw_state);
        println!("  Managed: {}", device.managed);
        println!(
            "  IP interface: {}",
            device.ip_interface.as_deref().unwrap_or("Unavailable")
        );
        println!(
            "  Hardware address: {}",
            device.hardware_address.as_deref().unwrap_or("Unavailable")
        );
        println!(
            "  Driver: {}",
            device.driver.as_deref().unwrap_or("Unavailable")
        );
        println!("  MTU: {:?}", device.mtu);
        println!(
            "  Active connection: {}",
            device.active_connection.as_deref().unwrap_or("None")
        );

        if let Some(wifi) = &device.wifi {
            println!("  Wi-Fi:");
            println!(
                "    SSID: {}",
                wifi.ssid.as_deref().unwrap_or("Not connected")
            );
            println!("    Signal: {:?}", wifi.signal_strength);
            println!("    Frequency: {:?}", wifi.frequency_mhz);
            println!("    Bitrate: {:?}", wifi.bitrate_kbps);
        }
    }

    let wifi_count = snapshot
        .devices
        .iter()
        .filter(|device| device.device_type == NetworkDeviceType::Wifi)
        .count();

    let ethernet_count = snapshot
        .devices
        .iter()
        .filter(|device| device.device_type == NetworkDeviceType::Ethernet)
        .count();

    println!("\nSummary:");
    println!("  Wi-Fi devices: {wifi_count}");
    println!("  Ethernet devices: {ethernet_count}");

    println!("==============================================");
}

fn main() {
    let bus = SystemBus::connect().unwrap_or_else(|error| {
        eprintln!("System D-Bus connection failed: {error}");
        std::process::exit(1);
    });

    let snapshot = bus.network_snapshot().unwrap_or_else(|error| {
        eprintln!("Network discovery failed: {error}");
        std::process::exit(1);
    });

    print_report(&snapshot);
}
