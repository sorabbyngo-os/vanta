use vanta_system::SystemBus;

fn main() {
    println!("Vanta Storage Event Monitor");
    println!("Listening for UDisks2 events...");
    println!("Press Ctrl+C to stop.\n");

    let bus = match SystemBus::connect() {
        Ok(bus) => bus,
        Err(error) => {
            eprintln!("D-Bus connection failed: {error}");
            return;
        }
    };

    let receiver = match bus.subscribe_storage_events() {
        Ok(receiver) => receiver,
        Err(error) => {
            eprintln!("Could not start storage monitor: {error}");
            return;
        }
    };

    loop {
        match receiver.recv() {
            Ok(event) => println!("{event:#?}"),
            Err(error) => {
                eprintln!("Storage event channel closed: {error}");
                break;
            }
        }
    }
}
