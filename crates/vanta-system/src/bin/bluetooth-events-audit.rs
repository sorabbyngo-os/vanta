use vanta_system::SystemBus;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bus = SystemBus::connect().map_err(std::io::Error::other)?;

    println!("VANTA BLUETOOTH EVENT MONITOR");
    println!("=============================");

    let receiver = bus
        .subscribe_bluetooth_events()
        .map_err(std::io::Error::other)?;

    println!("Monitoring BlueZ adapter and device events...");
    println!("Press Ctrl+C to stop.\n");

    while let Ok(event) = receiver.recv() {
        println!("{event:#?}");
    }

    Ok(())
}
