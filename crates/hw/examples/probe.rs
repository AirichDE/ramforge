//! Prints system info and two rounds of sensor readings.
//! `cargo run -p ramforge-hw --example probe`

use std::time::Duration;

fn main() {
    let info = ramforge_hw::collect();
    println!("OS:    {}", info.os);
    println!(
        "CPU:   {} ({} Threads) {:?}",
        info.cpu.brand, info.cpu.logical_processors, info.cpu.features
    );
    println!(
        "RAM:   {:.1} GiB, frei {:.1} GiB",
        info.memory.total_bytes as f64 / (1u64 << 30) as f64,
        info.memory.available_bytes as f64 / (1u64 << 30) as f64
    );
    match &info.smbios {
        Some(s) => {
            println!(
                "Board: {} {} | BIOS {} {}",
                s.board_manufacturer, s.board_product, s.bios_vendor, s.bios_version
            );
            for m in s.memory.iter().filter(|m| m.is_installed()) {
                println!(
                    "  {:<12} {:>3} GiB {} {} {} MT/s (läuft mit {}) {} {} mV",
                    m.locator,
                    m.size_bytes >> 30,
                    m.memory_type,
                    m.form_factor,
                    m.speed_mts,
                    m.configured_speed_mts,
                    m.manufacturer,
                    m.configured_voltage_mv
                );
                println!("               Teil {}", m.part_number);
            }
        }
        None => println!("SMBIOS: {:?}", info.smbios_error),
    }
    let hub = ramforge_hw::sensors::SensorHub::start_default(Duration::from_millis(500));
    std::thread::sleep(Duration::from_millis(1600));
    for (n, s) in hub.source_status() {
        println!("Quelle {n}: {s}");
    }
    for s in hub
        .snapshot_light()
        .iter()
        .filter(|s| !s.reading.label.starts_with("Thread"))
    {
        let r = &s.reading;
        println!(
            "  [{:?}] {} / {} = {:.1} {}",
            r.group, r.device, r.label, r.value, r.unit
        );
    }
    hub.shutdown();
}
