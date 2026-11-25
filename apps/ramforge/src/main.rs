//! RAMForge – portable RAM test, benchmark and hardware monitor.
// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod benchsuite;
mod chart;
mod log;
mod paths;
mod report;

/// `ramforge.exe --version`: version and update key fingerprint.
fn print_version() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
    println!("RAMForge {}", env!("CARGO_PKG_VERSION"));
    println!("Update-Manifest:  {}", ramforge_update::MANIFEST_URL);
    println!("Fingerabdrücke der Update-Schlüssel (SHA-256):");
    for (name, fp) in ramforge_update::key::fingerprints() {
        println!("  {name:<8} {fp}");
    }
    if ramforge_update::key::is_placeholder() {
        println!("  (Entwicklungsversion: Test- bzw. Platzhalterschlüssel)");
    }
    println!("Lizenz: GPL-3.0-or-later");
}

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        print_version();
        return Ok(());
    }
    ramforge_update::selfreplace::cleanup_old();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(format!("RAMForge {}", env!("CARGO_PKG_VERSION")))
        .with_inner_size([1320.0, 860.0])
        .with_min_inner_size([960.0, 600.0])
        .with_icon(
            app::icon_rgba()
                .map(|(rgba, width, height)| eframe::egui::IconData {
                    rgba,
                    width,
                    height,
                })
                .unwrap_or_default(),
        );
    if app::boot_edition() {
        // Kiosk (cage): whole screen, no window frame. With a frame, winit
        // leaves a black strip at the right and bottom edges.
        viewport = viewport
            .with_decorations(false)
            .with_fullscreen(true)
            .with_min_inner_size([640.0, 480.0]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "RAMForge",
        options,
        Box::new(|cc| Ok(Box::new(app::RamForgeApp::new(cc)))),
    )
}
