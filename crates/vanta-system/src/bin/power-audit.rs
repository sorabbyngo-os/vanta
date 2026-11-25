use vanta_system::SystemBus;

fn main() {
    let bus = SystemBus::connect().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1);
    });

    let power = bus.power_snapshot().unwrap_or_else(|error| {
        eprintln!("Power discovery failed: {error}");
        std::process::exit(1);
    });

    println!("\nVANTA POWER AUDIT");
    println!("==============================");

    println!("On battery: {}", power.on_battery);
    println!("Lid present: {}", power.lid_present);
    println!("Lid closed: {}", power.lid_closed);

    for (index, battery) in power.batteries.iter().enumerate() {
        println!("\nBattery {}", index + 1);
        println!("  Present: {}", battery.present);
        println!("  State: {:?}", battery.state);
        println!("  Charge: {:.1}%", battery.percentage);
        println!("  Energy: {:.2} Wh", battery.energy_wh);
        println!("  Full capacity: {:.2} Wh", battery.energy_full_wh);
        println!("  Power rate: {:.2} W", battery.energy_rate_w);
        println!(
            "  Time to empty: {}",
            battery
                .time_to_empty_seconds
                .map(|seconds| format!("{seconds} seconds"))
                .unwrap_or_else(|| "Unavailable".to_string())
        );
        println!(
            "  Time to full: {}",
            battery
                .time_to_full_seconds
                .map(|seconds| format!("{seconds} seconds"))
                .unwrap_or_else(|| "Unavailable".to_string())
        );
    }

    println!("\nPower capabilities:");
    println!("  Suspend: {:?}", power.capabilities.suspend);
    println!("  Hibernate: {:?}", power.capabilities.hibernate);
    println!("  Power off: {:?}", power.capabilities.power_off);
    println!("  Reboot: {:?}", power.capabilities.reboot);

    if let Some(profiles) = power.profiles {
        println!("\nPower profiles:");
        println!("  Active: {}", profiles.active_profile);

        for profile in profiles.profiles {
            println!("  Available: {} ({:?})", profile.name, profile.driver);
        }

        println!(
            "  Performance degraded: {:?}",
            profiles.performance_degraded
        );
    } else {
        println!("\nPower Profiles service unavailable or unreadable");
    }

    println!("==============================");
}
