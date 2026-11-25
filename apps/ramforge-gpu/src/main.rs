//! RAMForge GPU module (optional download, ~5 MB).
//!
//! `RAMForge-GPU.exe --json` runs the GPU benchmark and prints the result as
//! JSON on stdout; progress goes to stderr as "PROGRESS <0..1> <text>".
//! RAMForge.exe starts it from the benchmark tab if it lies next to it.
use std::sync::atomic::AtomicBool;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("RAMForge-GPU {} (Arbeitslast v{})", env!("CARGO_PKG_VERSION"), ramforge_gpu::WORKLOAD_VERSION);
        return;
    }
    let cancel = AtomicBool::new(false);
    let r = ramforge_gpu::run(&cancel, &|f, text| eprintln!("PROGRESS {f:.2} {text}"));
    if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string(&r).unwrap_or_default());
    } else {
        match &r.skipped {
            Some(why) => println!("{why}"),
            None => println!(
                "{} ({}, {})\n  VRAM {:.0} GB/s · FP32 {:.2} TFLOPS · zur GPU {:.1} GB/s · zum PC {:.1} GB/s · Fehler {}",
                r.adapter, r.backend, r.driver, r.vram_gbs, r.fp32_tflops, r.upload_gbs, r.download_gbs, r.errors
            ),
        }
    }
}
