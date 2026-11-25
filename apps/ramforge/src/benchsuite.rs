//! Benchmark suite: CPU, RAM, combined CPU+RAM load. Results carry the
//! program version and every setting, and are kept in a history file so runs
//! can be compared (before/after BIOS, XMP/EXPO, cooling changes).

use ramforge_core::bench::{self, BenchResult};
use ramforge_core::cpubench::{self, CpuScore};
use ramforge_core::stress::{self, StressResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Selection {
    pub cpu: bool,
    pub ram: bool,
    pub stress: bool,
    #[serde(default)]
    pub gpu: bool,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            cpu: true,
            ram: true,
            stress: false,
            gpu: false,
        }
    }
}

/// Everything that changes a score. Two runs are comparable only if these are equal.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BenchSettings {
    pub threads: usize,
    pub ram_mib_per_thread: usize,
    pub cpu_secs: f64,
    pub stress_secs: u64,
    pub workload_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchRun {
    pub time: chrono::DateTime<chrono::Local>,
    pub app_version: String,
    pub cpu_name: String,
    pub settings: BenchSettings,
    pub selection: Selection,
    #[serde(default)]
    pub cpu: Vec<CpuScore>,
    #[serde(default)]
    pub ram: Option<BenchResult>,
    #[serde(default)]
    pub stress: Option<StressResult>,
    #[serde(default)]
    pub gpu: Option<ramforge_gpu::GpuResult>,
    #[serde(default)]
    pub cancelled: bool,
    /// Free text, e.g. "XMP an" (editable in the history).
    #[serde(default)]
    pub note: String,
}

impl BenchRun {
    pub fn errors(&self) -> u64 {
        self.cpu.iter().map(|c| c.errors).sum::<u64>()
            + self.stress.as_ref().map(|s| s.errors).unwrap_or(0)
            + self.gpu.as_ref().map(|g| g.errors).unwrap_or(0)
    }
    /// Same workload and settings: scores may be compared.
    pub fn comparable(&self, other: &BenchRun) -> bool {
        self.settings == other.settings && self.app_version == other.app_version
    }
    pub fn cpu_points(&self) -> (u32, u32) {
        let ok: Vec<&CpuScore> = self.cpu.iter().filter(|c| c.skipped.is_none()).collect();
        if ok.is_empty() {
            return (0, 0);
        }
        let n = ok.len() as f64;
        // Geometric mean, so one strong test does not dominate.
        let g = |f: &dyn Fn(&CpuScore) -> u32| {
            (ok.iter().map(|c| (f(c).max(1) as f64).ln()).sum::<f64>() / n)
                .exp()
                .round() as u32
        };
        (g(&|c| c.single_points), g(&|c| c.multi_points))
    }
    pub fn settings_text(&self) -> String {
        let s = &self.settings;
        format!(
            "RAMForge {} · Arbeitslast v{} (GPU v{}) · {} Threads · RAM {} MiB/Thread · CPU {:.0} s/Test · Last {} s",
            self.app_version, s.workload_version, ramforge_gpu::WORKLOAD_VERSION, s.threads, s.ram_mib_per_thread, s.cpu_secs, s.stress_secs
        )
    }
}

/// Progress: (fraction, step name, detail). A new step name becomes a phase marker.
pub type Progress<'a> = dyn Fn(f32, &str, &str) + 'a;

pub fn run(
    sel: &Selection,
    settings: &BenchSettings,
    cpu_name: &str,
    cancel: &AtomicBool,
    progress: &Progress,
) -> BenchRun {
    let mut run = BenchRun {
        time: chrono::Local::now(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        cpu_name: cpu_name.into(),
        settings: settings.clone(),
        selection: sel.clone(),
        cpu: Vec::new(),
        ram: None,
        stress: None,
        gpu: None,
        cancelled: false,
        note: String::new(),
    };
    let parts = [sel.cpu, sel.ram, sel.stress, sel.gpu]
        .iter()
        .filter(|b| **b)
        .count()
        .max(1) as f32;
    let mut done = 0f32;
    let sub = |done: f32, f: f32| (done + f.clamp(0.0, 1.0)) / parts;
    if sel.cpu && !cancel.load(Ordering::Relaxed) {
        for (i, k) in cpubench::CpuKind::ALL.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            progress(
                sub(done, i as f32 / 3.0),
                &format!("CPU · {}", k.name()),
                "Single- und Multi-Thread",
            );
            run.cpu.push(cpubench::run_one(
                *k,
                settings.threads,
                settings.cpu_secs,
                cancel,
            ));
        }
        done += 1.0;
    }
    if sel.ram && !cancel.load(Ordering::Relaxed) {
        let bytes = settings.ram_mib_per_thread << 20;
        run.ram = bench::run(settings.threads, bytes, cancel, &|f, l| {
            progress(sub(done, f), &format!("RAM · {l}"), "")
        });
        done += 1.0;
    }
    if sel.stress && !cancel.load(Ordering::Relaxed) {
        let bytes = settings.ram_mib_per_thread << 20;
        let d = done;
        run.stress = Some(stress::run(
            settings.threads,
            bytes,
            settings.stress_secs,
            cancel,
            &|f, l| progress(sub(d, f), "CPU+RAM-Last", l),
        ));
        done += 1.0;
    }
    if sel.gpu && !cancel.load(Ordering::Relaxed) {
        let d = done;
        run.gpu = Some(match ramforge_gpu::find_module() {
            Some(m) => ramforge_gpu::run_module(&m, &|f, l| progress(sub(d, f), l, "")),
            None => ramforge_gpu::GpuResult {
                skipped: Some(
                    "GPU-Modul nicht installiert (RAMForge-GPU.exe neben RAMForge.exe)".into(),
                ),
                ..Default::default()
            },
        });
    }
    run.cancelled = cancel.load(Ordering::Relaxed);
    progress(1.0, "Fertig", "");
    run
}

const HISTORY_MAX: usize = 50;

pub fn load_history(path: &Path) -> Vec<BenchRun> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save_history(path: &Path, h: &[BenchRun]) {
    let start = h.len().saturating_sub(HISTORY_MAX);
    if let Ok(s) = serde_json::to_string_pretty(&h[start..]) {
        let _ = std::fs::write(path, s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> BenchSettings {
        BenchSettings {
            threads: 2,
            ram_mib_per_thread: 16,
            cpu_secs: 0.2,
            stress_secs: 2,
            workload_version: cpubench::WORKLOAD_VERSION,
        }
    }

    #[test]
    fn suite_runs_selected_parts_and_history_round_trips() {
        let cancel = AtomicBool::new(false);
        let steps = std::sync::Mutex::new(Vec::<String>::new());
        let sel = Selection {
            cpu: true,
            ram: false,
            stress: true,
            gpu: false,
        };
        let r = run(&sel, &settings(), "Test-CPU", &cancel, &|_, step, _| {
            let mut s = steps.lock().unwrap();
            if s.last().map(|l| l != step).unwrap_or(true) {
                s.push(step.to_string());
            }
        });
        assert_eq!(r.cpu.len(), 3);
        assert!(r.ram.is_none());
        assert!(r.stress.is_some());
        assert_eq!(r.errors(), 0);
        let steps = steps.into_inner().unwrap();
        assert!(
            steps.iter().any(|s| s.starts_with("CPU · Ganzzahl"))
                && steps.iter().any(|s| s == "CPU+RAM-Last"),
            "{steps:?}"
        );
        let (st, mt) = r.cpu_points();
        assert!(st > 0 && mt >= st);

        let path = std::env::temp_dir().join(format!(
            "ramforge-bench-history-{}.json",
            std::process::id()
        ));
        save_history(&path, &[r.clone(), r.clone()]);
        let h = load_history(&path);
        assert_eq!(h.len(), 2);
        assert!(h[0].comparable(&h[1]));
        let mut other = h[1].clone();
        other.settings.threads = 16;
        assert!(
            !h[0].comparable(&other),
            "andere Einstellungen dürfen nicht vergleichbar sein"
        );
        std::fs::remove_file(path).ok();
    }
}
