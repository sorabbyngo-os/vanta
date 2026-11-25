use vanta_system::{ServiceInfo, SystemBus, SystemServices};

fn print_service(service: &ServiceInfo) {
    let status = match &service.status {
        vanta_system::ServiceStatus::Available => "AVAILABLE",
        vanta_system::ServiceStatus::Unavailable => "UNAVAILABLE",
        vanta_system::ServiceStatus::Unknown(_) => "UNKNOWN",
    };

    println!("{:<20} {:<12} {}", service.name, status, service.bus_name);

    if let vanta_system::ServiceStatus::Unknown(error) = &service.status {
        println!("  Error: {error}");
    }
}

fn print_report(services: &SystemServices) {
    println!("\nVANTA SYSTEM SERVICE DISCOVERY");
    println!("==============================================");

    print_service(&services.upower);
    print_service(&services.logind);
    print_service(&services.power_profiles);
    print_service(&services.network_manager);
    print_service(&services.bluez);
    print_service(&services.udisks2);
    print_service(&services.tuned);

    println!("==============================================");
}

fn main() {
    let bus = match SystemBus::connect() {
        Ok(bus) => bus,
        Err(error) => {
            eprintln!("Cannot initialize system integration: {error}");
            std::process::exit(1);
        }
    };

    let services = bus.discover_services();
    print_report(&services);
}
