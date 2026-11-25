//! Main window.

use crate::log::{AppLog, Level};
use crate::paths::{self, Paths, Settings, DISCLAIMER_VERSION};
use crate::report;
use eframe::egui::{self, Color32, Key, RichText};
use ramforge_core::bench::BenchResult;
use ramforge_core::runner::{self, Outcome, Preset, RunStatus, RunSummary, TestConfig};
use ramforge_core::{MemError, TestKind};
use ramforge_hw::sensors::{
    EventKind, SensorGroup, SensorHub, SensorKind, SensorStat, SessionData,
};
use ramforge_hw::spd::{self, SpdInfo};
use ramforge_hw::SystemInfo;
use ramforge_update::online::{self, NewsItem};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DISCLAIMER_MD: &str = include_str!("../../../DISCLAIMER.md");
const ICON_PNG: &[u8] = include_bytes!("../../../assets/logo/out/ramforge-icon-256.png");
pub const URL_SITE: &str = "https://ramforge.org";
pub const URL_DISCLAIMER: &str = "https://ramforge.org/disclaimer";
pub const URL_PRIVACY: &str = "https://ramforge.org/privacy";
pub const URL_SOURCE: &str = "https://github.com/airichde/ramforge";
const SPONSOR_NOTE: &str = "Hosting freundlicherweise bereitgestellt von Nerdscave Consulting, nicht am Projekt beteiligt.";
const URL_SPONSOR: &str = "https://nerdscave-consulting.com/";

pub fn icon_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let (rgba, w, h) = icon_rgba()?;
    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    Some(ctx.load_texture("ramforge-logo", img, egui::TextureOptions::LINEAR))
}
const ACCENT: Color32 = Color32::from_rgb(0xF2, 0x6B, 0x2A);
const OK: Color32 = Color32::from_rgb(0x2E, 0xB8, 0x6A);
const BAD: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
const WARN: Color32 = Color32::from_rgb(0xE8, 0x9A, 0x2C);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    System,
    Test,
    Bench,
    Sensors,
    Spd,
    Log,
    About,
}

impl Tab {
    const ALL: [Tab; 7] = [
        Tab::System,
        Tab::Test,
        Tab::Bench,
        Tab::Sensors,
        Tab::Spd,
        Tab::Log,
        Tab::About,
    ];
    /// Tabs shown in this build: the boot variant "Nur RAM-Test" has no benchmarks.
    fn visible() -> Vec<Tab> {
        Tab::ALL
            .into_iter()
            .filter(|t| *t != Tab::Bench || !ram_only())
            .collect()
    }
    fn title(self) -> &'static str {
        match self {
            Tab::System => "System",
            Tab::Test => "RAM-Test",
            Tab::Bench => "Benchmark",
            Tab::Sensors => "Sensoren",
            Tab::Spd => "SPD",
            Tab::Log => "Protokoll",
            Tab::About => "Über",
        }
    }
}

struct BenchJob {
    cancel: Arc<AtomicBool>,
    /// (fraction, step, detail)
    progress: Arc<Mutex<(f32, String, String)>>,
    result: Arc<Mutex<Option<crate::benchsuite::BenchRun>>>,
}

pub struct RamForgeApp {
    tab: Tab,
    paths: Paths,
    settings: Settings,
    log: AppLog,
    sys: SystemInfo,
    hub: SensorHub,
    async_msgs: Arc<Mutex<Vec<(Level, String)>>>,

    // RAM test
    preset: Option<Preset>,
    custom: Vec<(TestKind, bool)>,
    threads: usize,
    passes: u32,
    bitfade_secs: u64,
    lock_memory: bool,
    stop_on_error: bool,
    inject_fault: bool,
    run: Option<Arc<RunStatus>>,
    finalized: bool,
    errors: Vec<MemError>,
    bits: [u64; 64],
    last_summary: Option<RunSummary>,

    // Benchmark
    bench_threads: usize,
    bench_mib: usize,
    bench: Option<BenchJob>,
    bench_result: Option<BenchResult>,
    bench_sel: crate::benchsuite::Selection,
    bench_stress_secs: u64,
    bench_run: Option<crate::benchsuite::BenchRun>,
    bench_history: Vec<crate::benchsuite::BenchRun>,
    gpu_download: Arc<Mutex<Option<String>>>,

    // Sensors
    sensor_filter: String,
    show_threads: bool,
    plot_ids: Vec<String>,

    // SPD
    spd_files: Vec<(String, Result<SpdInfo, String>)>,

    disclaimer_check: bool,
    show_serials: bool,
    last_report: Option<std::path::PathBuf>,
    updater: ramforge_update::ui::UpdateUi,
    update_started: bool,
    news: Arc<Mutex<Option<Vec<NewsItem>>>>,
    news_seen_before: i64,
    online_started: bool,
    stats_choice: bool,
    logo: Option<egui::TextureHandle>,
    /// `--screenshot <file>`: render, save a PNG, exit (docs and CI).
    auto_shot: Option<(
        std::path::PathBuf,
        std::time::Instant,
        bool,
        Arc<AtomicBool>,
    )>,
    /// `--start-test`: quick self test on 10 % of free RAM (end-to-end check).
    /// `--start-test selftest|quick|standard|extreme` or `--start-bench`.
    auto_test: Option<String>,
    /// Recorded sensor sessions of this program run (newest last, max 6).
    sessions: Vec<SessionData>,
    chart_test: crate::chart::ChartState,
    chart_bench: crate::chart::ChartState,
    chart_log: crate::chart::ChartState,
    log_session: usize,
    /// Last (pass, test index) marked as phase, last benchmark step marked.
    /// How many entries of the runner's phase log were marked already.
    phases_seen: usize,
    bench_phase: String,
}

impl RamForgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let paths = Paths::resolve();
        let settings = Settings::load(&paths.settings_file());
        let mut log = AppLog::new(&paths.logs);
        log.info(format!("RAMForge {} gestartet", env!("CARGO_PKG_VERSION")));
        log.info(format!(
            "Datenordner: {}{}",
            paths.root.display(),
            if paths.portable {
                " (portabel)"
            } else {
                " (Benutzerprofil, EXE-Ordner nicht beschreibbar)"
            }
        ));
        if std::env::args().any(|a| a == "--updated") {
            log.info(format!(
                "Update auf Version {} installiert",
                env!("CARGO_PKG_VERSION")
            ));
        }
        let sys = ramforge_hw::collect();
        if let Some(e) = &sys.smbios_error {
            log.warn(format!("SMBIOS: {e}"));
        }
        log.info(format!(
            "System: {} | {} | {}",
            sys.os,
            sys.cpu.brand,
            report::gib(sys.memory.total_bytes)
        ));
        let hub = SensorHub::start_default(Duration::from_millis(settings.sensor_interval_ms));
        let threads = sys.cpu.logical_processors.max(1);
        let start_tab = match arg_value("--tab").as_deref() {
            Some("system") => Tab::System,
            Some("bench") if !ram_only() => Tab::Bench,
            Some("sensors") => Tab::Sensors,
            Some("spd") => Tab::Spd,
            Some("log") => Tab::Log,
            Some("about") => Tab::About,
            _ => Tab::Test,
        };
        let app = Self {
            tab: start_tab,
            custom: TestKind::ALL
                .iter()
                .map(|&k| (k, k != TestKind::BitFade && k != TestKind::Hammer))
                .collect(),
            preset: Some(Preset::Standard),
            threads,
            passes: Preset::Standard.passes(),
            bitfade_secs: 300,
            lock_memory: true,
            stop_on_error: false,
            inject_fault: false,
            run: None,
            finalized: true,
            errors: Vec::new(),
            bits: [0; 64],
            last_summary: None,
            bench_threads: threads,
            bench_mib: 256,
            bench: None,
            bench_result: None,
            bench_sel: Default::default(),
            bench_stress_secs: 60,
            bench_run: None,
            bench_history: crate::benchsuite::load_history(&paths.root.join("bench-history.json")),
            gpu_download: Arc::new(Mutex::new(None)),
            sensor_filter: String::new(),
            show_threads: false,
            plot_ids: Vec::new(),
            spd_files: Vec::new(),
            disclaimer_check: false,
            show_serials: false,
            last_report: None,
            updater: ramforge_update::ui::UpdateUi::new(
                "RAMForge",
                ramforge_update::AssetKind::WindowsPortable,
                env!("CARGO_PKG_VERSION"),
            ),
            update_started: false,
            news: Arc::new(Mutex::new(None)),
            news_seen_before: 0,
            online_started: false,
            stats_choice: false,
            logo: load_logo(&cc.egui_ctx),
            sessions: Vec::new(),
            chart_test: Default::default(),
            chart_bench: Default::default(),
            chart_log: Default::default(),
            log_session: 0,
            phases_seen: 0,
            bench_phase: String::new(),
            auto_test: if std::env::args().any(|a| a == "--start-bench") && !ram_only() {
                Some("bench".into())
            } else if std::env::args().any(|a| a == "--start-test") {
                Some(
                    arg_value("--start-test")
                        .filter(|v| !v.starts_with("--"))
                        .unwrap_or_else(|| "selftest".into()),
                )
            } else {
                None
            },
            auto_shot: arg_value("--screenshot").map(|p| {
                (
                    std::path::PathBuf::from(p),
                    std::time::Instant::now(),
                    false,
                    Arc::new(AtomicBool::new(false)),
                )
            }),
            async_msgs: Arc::new(Mutex::new(Vec::new())),
            paths,
            settings,
            log,
            sys,
            hub,
        };
        ramforge_update::ui::install_symbol_font(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx, &app.settings.theme);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            cc.egui_ctx.style_mut_of(theme, |s| {
                s.visuals.selection.bg_fill = ACCENT;
                s.visuals.hyperlink_color = ACCENT;
                s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            });
        }
        app
    }

    fn running(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|r| r.running.load(Ordering::Acquire))
    }
    fn busy(&self) -> bool {
        self.running() || self.bench.is_some()
    }
    fn accepted(&self) -> bool {
        self.settings.disclaimer_accepted >= DISCLAIMER_VERSION
    }
    fn save_settings(&self) {
        self.settings.save(&self.paths.settings_file());
    }

    // ---------------------------------------------------------------- actions

    fn start_test(&mut self) {
        if self.busy() || !self.accepted() {
            return;
        }
        let tests: Vec<TestKind> = match self.preset {
            Some(p) => p.tests(),
            None => self
                .custom
                .iter()
                .filter(|(_, on)| *on)
                .map(|(k, _)| *k)
                .collect(),
        };
        if tests.is_empty() {
            self.log.warn("Kein Test ausgewählt");
            return;
        }
        let avail = ramforge_core::mem::mem_status().available_bytes;
        let total =
            ((avail as f64 * self.settings.mem_percent as f64 / 100.0) as u64).max(64 << 20);
        let cfg = TestConfig {
            tests,
            passes: self.passes,
            threads: self.threads.max(1),
            total_bytes: total,
            lock_memory: self.lock_memory,
            bitfade_secs: self.bitfade_secs,
            stop_on_error: self.stop_on_error,
            inject_fault: self.inject_fault,
        };
        let name = self.preset.map(|p| p.name()).unwrap_or("Eigene Auswahl");
        self.log.info(format!(
            "RAM-Test gestartet: {name}, {} mit {} Threads, {} Durchläufe{}",
            report::gib(total),
            cfg.threads,
            if cfg.passes == 0 {
                "∞".to_string()
            } else {
                cfg.passes.to_string()
            },
            if cfg.inject_fault {
                ", Selbsttest aktiv"
            } else {
                ""
            }
        ));
        self.begin_session("test", &format!("RAM-Test gestartet: {name}"));
        self.errors.clear();
        self.bits = [0; 64];
        self.finalized = false;
        self.run = Some(runner::start(cfg));
    }

    /// Starts the sensor recording for a RAM test or benchmark (if enabled).
    fn begin_session(&mut self, kind: &str, start_text: &str) {
        self.phases_seen = 0;
        self.bench_phase.clear();
        if !self.settings.record_sensors {
            return;
        }
        self.hub
            .set_interval(Duration::from_millis(self.settings.record_interval_ms));
        let csv = self
            .paths
            .logs
            .join(format!("sensors-{}-{kind}.csv", paths::stamp()));
        match self.hub.start_session(kind, Some(&csv)) {
            Ok(()) => self.log.info(format!("Sensor-Aufzeichnung: {}", csv.display())),
            Err(e) => self.log.warn(format!("Sensor-CSV nicht möglich ({e}); der Verlauf wird trotzdem im Programm aufgezeichnet")),
        }
        self.hub.mark_kind(EventKind::Info, start_text);
    }

    /// Ends the recording with the result as last marker and keeps the series.
    fn end_session(&mut self, result_text: &str) {
        if !self.hub.session_active() {
            return;
        }
        self.hub.mark_kind(EventKind::Info, result_text);
        if let Some(d) = self.hub.stop_session() {
            self.log.info(format!(
                "Sensor-Aufzeichnung beendet: {} Sensoren, {} Ereignisse{}",
                d.series.len(),
                d.events.len(),
                d.csv
                    .as_ref()
                    .map(|p| format!(", CSV {}", p.display()))
                    .unwrap_or_default()
            ));
            self.sessions.push(d);
            if self.sessions.len() > 6 {
                self.sessions.remove(0);
            }
            self.log_session = self.sessions.len() - 1;
        }
        self.hub
            .set_interval(Duration::from_millis(self.settings.sensor_interval_ms));
    }

    fn last_session(&self, kind: &str) -> Option<&SessionData> {
        self.sessions.iter().rev().find(|s| s.label == kind)
    }

    /// Checkbox + interval row shown in the RAM test and benchmark tabs.
    fn record_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .checkbox(
                    &mut self.settings.record_sensors,
                    "Sensoren während des Tests aufzeichnen",
                )
                .changed()
            {
                self.save_settings();
            }
            ui.add_enabled_ui(self.settings.record_sensors, |ui| {
                ui.label("Intervall");
                for (ms, label) in [
                    (500u64, "0,5 s"),
                    (1000, "1 s"),
                    (2000, "2 s"),
                    (5000, "5 s"),
                ] {
                    if ui
                        .selectable_label(self.settings.record_interval_ms == ms, label)
                        .clicked()
                    {
                        self.settings.record_interval_ms = ms;
                        self.save_settings();
                    }
                }
            });
        });
        let snap = self.hub.snapshot_light();
        let temps = snap
            .iter()
            .any(|s| s.reading.kind == SensorKind::Temperature);
        let volts = snap.iter().any(|s| s.reading.kind == SensorKind::Voltage);
        if self.settings.record_sensors && !(temps && volts) {
            ui.label(
                RichText::new(
                    "Für Temperaturen und Spannungen HWiNFO mit 'Shared Memory Support' starten. \
                     Auslastung, Takt und RAM-Belegung werden auch ohne HWiNFO aufgezeichnet.",
                )
                .small()
                .color(WARN),
            );
        }
    }

    fn stop_test(&mut self) {
        if let Some(r) = &self.run {
            if r.running.load(Ordering::Acquire) {
                r.stop();
                self.log.info("Test wird abgebrochen…");
            }
        }
        if let Some(b) = &self.bench {
            b.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn start_bench(&mut self) {
        if self.busy() || !self.accepted() {
            return;
        }
        let sel = self.bench_sel.clone();
        if !(sel.cpu || sel.ram || sel.stress || sel.gpu) {
            self.log.warn("Kein Benchmark ausgewählt");
            return;
        }
        let threads = self.bench_threads.max(1);
        let avail = ramforge_core::mem::mem_status().available_bytes as usize;
        let per_thread_mib = (self.bench_mib).min(avail / 2 / threads >> 20).max(16);
        let settings = crate::benchsuite::BenchSettings {
            threads,
            ram_mib_per_thread: per_thread_mib,
            cpu_secs: 3.0,
            stress_secs: self.bench_stress_secs,
            workload_version: ramforge_core::cpubench::WORKLOAD_VERSION,
        };
        let parts: Vec<&str> = [
            (sel.cpu, "CPU"),
            (sel.ram, "RAM"),
            (sel.stress, "CPU+RAM-Last"),
            (sel.gpu, "GPU"),
        ]
        .iter()
        .filter(|p| p.0)
        .map(|p| p.1)
        .collect();
        let text = format!(
            "Benchmark gestartet: {} · {threads} Threads × {per_thread_mib} MiB",
            parts.join(", ")
        );
        self.log.info(text.clone());
        self.begin_session("bench", &text);
        let job = BenchJob {
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new((0.0, "Start".into(), String::new()))),
            result: Arc::new(Mutex::new(None)),
        };
        let (c, p, r) = (job.cancel.clone(), job.progress.clone(), job.result.clone());
        let cpu_name = self.sys.cpu.brand.clone();
        std::thread::spawn(move || {
            let res = crate::benchsuite::run(&sel, &settings, &cpu_name, &c, &|f, step, detail| {
                *p.lock().unwrap() = (f, step.to_string(), detail.to_string())
            });
            *r.lock().unwrap() = Some(res);
        });
        self.bench = Some(job);
    }

    fn screenshot(&mut self, ctx: &egui::Context) {
        let path = self
            .paths
            .screenshots
            .join(format!("ramforge-{}.png", paths::stamp()));
        let msgs = self.async_msgs.clone();
        ctx.request_screenshot(move |img| {
            let bytes: Vec<u8> = img
                .pixels
                .iter()
                .flat_map(|c| c.to_srgba_unmultiplied())
                .collect();
            let res = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes)
                .ok_or_else(|| "ungültige Bildgröße".to_string())
                .and_then(|i| i.save(&path).map_err(|e| e.to_string()));
            msgs.lock().unwrap().push(match res {
                Ok(()) => (
                    Level::Info,
                    format!("Screenshot gespeichert: {}", path.display()),
                ),
                Err(e) => (Level::Error, format!("Screenshot fehlgeschlagen: {e}")),
            });
        });
        ctx.request_repaint();
    }

    fn save_report(&mut self) {
        let sensors = self.hub.snapshot_light();
        let sessions: Vec<&SessionData> = ["test", "bench"]
            .iter()
            .filter_map(|k| self.last_session(k))
            .collect();
        let r = report::build(report::ReportInput {
            system: &self.sys,
            test: self.last_summary.as_ref(),
            errors: &self.errors,
            bit_histogram: &self.bits,
            benchmark: self.bench_result.as_ref(),
            bench_run: self.bench_run.as_ref(),
            sensors: &sensors,
            events: self.hub.events(),
            sessions,
            anonymize: self.settings.anonymize_reports,
        });
        match report::save_all(&r, &self.paths.reports) {
            Ok(p) => {
                self.log.info(format!(
                    "Bericht gespeichert: {} (+ .json, .txt)",
                    p.display()
                ));
                self.last_report = Some(p);
            }
            Err(e) => self
                .log
                .error(format!("Bericht konnte nicht gespeichert werden: {e}")),
        }
    }

    /// Called every frame: collects results from background threads.
    fn poll(&mut self) {
        for (lvl, m) in std::mem::take(&mut *self.async_msgs.lock().unwrap()) {
            self.log.add(lvl, m);
        }
        if let Some(run) = self.run.clone() {
            // Phases first, so an error is listed after the phase it happened in.
            let new_phases: Vec<(u32, usize)> = run
                .phase_log
                .lock()
                .unwrap()
                .get(self.phases_seen..)
                .map(|v| v.to_vec())
                .unwrap_or_default();
            for (pass, ti) in new_phases {
                self.phases_seen += 1;
                if let Some(t) = run.config.tests.get(ti) {
                    self.hub.mark_kind(
                        EventKind::Phase,
                        format!("Durchlauf {} · {}", pass + 1, t.name()),
                    );
                }
            }
            for e in run.sink.errors_since(self.errors.len()) {
                let n = self.errors.len() + 1;
                let bits: Vec<String> = (0..64)
                    .filter(|b| e.xor() & (1u64 << b) != 0)
                    .map(|b| b.to_string())
                    .collect();
                let text = format!(
                    "RAM-Fehler #{n}: {} Durchlauf {} Thread {} Adresse 0x{:X} soll {:016X} ist {:016X} Bit {}{}",
                    e.test, e.pass + 1, e.thread, e.address, e.expected, e.actual, bits.join(","),
                    if e.simulated { " (Selbsttest, simuliert)" } else { "" }
                );
                // Chart/CSV markers for the first 1000 errors; beyond that a
                // defective module would flood the CSV, the table keeps all.
                if n <= 1000 {
                    self.hub.mark_kind(EventKind::Error, text.clone());
                } else if n == 1001 {
                    self.hub.mark_kind(
                        EventKind::Info,
                        "Mehr als 1000 Fehler: weitere nur in Fehlertabelle und Bericht",
                    );
                }
                if n <= 100 {
                    self.log.error(text);
                } else if n == 101 {
                    self.log
                        .warn("Weitere Fehler stehen nur noch in der Fehlertabelle und im Bericht");
                }
                self.errors.push(e);
            }
            self.bits = run.sink.bit_histogram();
            if !run.running.load(Ordering::Acquire) && !self.finalized {
                self.finalized = true;
                if let Some(s) = run.summary.lock().unwrap().clone() {
                    let text = format!(
                        "RAM-Test beendet: {} – {} (Dauer {}, {} Durchläufe, {} Fehler)",
                        report::outcome_text(s.outcome),
                        s.message,
                        report::duration(s.duration_secs),
                        s.passes_completed,
                        s.errors
                    );
                    self.log.add(
                        if s.outcome == Outcome::Failed {
                            Level::Error
                        } else {
                            Level::Info
                        },
                        text.clone(),
                    );
                    if !s.memory_locked && s.outcome != Outcome::Aborted {
                        self.log.warn("Nicht der gesamte Testspeicher konnte gesperrt werden; Windows konnte Teile auslagern.");
                    }
                    self.last_summary = Some(s);
                    self.end_session(&text);
                } else {
                    self.end_session("RAM-Test beendet");
                }
                self.save_report();
            }
        }
        let step = self
            .bench
            .as_ref()
            .map(|b| b.progress.lock().unwrap().1.clone());
        // (progress.2 is detail text such as the current GB/s and is not a phase.)
        if let Some(step) = step {
            if step != self.bench_phase && step != "Start" && step != "Fertig" {
                self.hub
                    .mark_kind(EventKind::Phase, format!("Benchmark · {step}"));
                self.bench_phase = step;
            }
        }
        let done = self
            .bench
            .as_ref()
            .and_then(|b| b.result.lock().unwrap().take());
        if let Some(run) = done {
            self.bench = None;
            let mut parts = Vec::new();
            for c in &run.cpu {
                match &c.skipped {
                    Some(why) => parts.push(format!("{}: {why}", c.kind.name())),
                    None => parts.push(format!(
                        "{} [{}]: 1T {} Pkt ({:.1} {u}), {}T {} Pkt ({:.1} {u}), Skalierung {:.1}×{}",
                        c.kind.name(), c.variant, c.single_points, c.single_raw, c.threads, c.multi_points, c.multi_raw, c.scaling,
                        if c.errors > 0 { format!(", {} RECHENFEHLER", c.errors) } else { String::new() },
                        u = c.kind.unit()
                    )),
                }
            }
            if let Some(r) = &run.ram {
                parts.push(format!(
                    "RAM: Lesen {:.2} GB/s, Schreiben {:.2} GB/s, Kopieren {:.2} GB/s, Latenz {:.1} ns",
                    r.read_gbs, r.write_gbs, r.copy_gbs, r.dram_latency_ns().unwrap_or(0.0)
                ));
            }
            if let Some(s) = &run.stress {
                parts.push(format!(
                    "CPU+RAM-Last: Ø {:.1} GB/s (min {:.1}, max {:.1}, Stabilität {:.0} %), {} Fehler",
                    s.avg_gbs, s.min_gbs, s.max_gbs, s.stability() * 100.0, s.errors
                ));
                for e in s.first_errors.iter().take(20) {
                    self.hub.mark_kind(
                        EventKind::Error,
                        format!(
                            "Fehler unter Last: Thread {} Adresse 0x{:X} soll {:016X} ist {:016X}",
                            e.thread, e.address, e.expected, e.actual
                        ),
                    );
                }
            }
            if let Some(g) = &run.gpu {
                parts.push(match &g.skipped {
                    Some(why) => format!("GPU: {why}"),
                    None => format!(
                        "GPU {} ({}): VRAM {:.0} GB/s, FP32 {:.2} TFLOPS, zur GPU {:.1} GB/s, zum PC {:.1} GB/s, {} Fehler",
                        g.adapter, g.backend, g.vram_gbs, g.fp32_tflops, g.upload_gbs, g.download_gbs, g.errors
                    ),
                });
            }
            for p in &parts {
                if p.contains("RECHENFEHLER") || p.contains("Fehler") && !p.contains(" 0 Fehler") {
                    self.log.error(p.clone());
                } else {
                    self.log.info(p.clone());
                }
            }
            let state = if run.cancelled {
                "abgebrochen"
            } else {
                "beendet"
            };
            self.end_session(&format!(
                "Benchmark {state}{}",
                if run.errors() > 0 {
                    format!(" – {} Fehler", run.errors())
                } else {
                    String::new()
                }
            ));
            if let Some(r) = &run.ram {
                self.bench_result = Some(r.clone());
            }
            if !run.cancelled {
                self.bench_history.push(run.clone());
                crate::benchsuite::save_history(
                    &self.paths.root.join("bench-history.json"),
                    &self.bench_history,
                );
            }
            self.bench_run = Some(run);
            self.save_report();
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if !self.accepted() {
            return;
        }
        let (f5, esc, f12, save, tab) = ctx.input(|i| {
            let tab = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
            ]
            .iter()
            .position(|k| i.modifiers.command && i.key_pressed(*k));
            (
                i.key_pressed(Key::F5),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::F12),
                i.modifiers.command && i.key_pressed(Key::S),
                tab,
            )
        });
        if f5 {
            if self.tab == Tab::Bench {
                self.start_bench()
            } else {
                self.tab = Tab::Test;
                self.start_test()
            }
        }
        if esc {
            self.stop_test();
        }
        if f12 {
            self.screenshot(ctx);
        }
        if save {
            self.save_report();
        }
        if let Some(t) = tab.and_then(|t| Tab::visible().get(t).copied()) {
            self.tab = t;
        }
    }
}

impl RamForgeApp {
    fn auto_screenshot(&mut self, ctx: &egui::Context) {
        if self.accepted() {
            if let Some(mode) = self.auto_test.take() {
                if mode == "bench" {
                    self.bench_mib = 128;
                    self.bench_sel = crate::benchsuite::Selection {
                        cpu: true,
                        ram: true,
                        stress: true,
                        gpu: true,
                    };
                    self.bench_stress_secs = 10;
                    self.start_bench();
                } else {
                    let preset = match mode.as_str() {
                        "standard" => Preset::Standard,
                        "extreme" => Preset::Extreme,
                        _ => Preset::Quick,
                    };
                    self.preset = Some(preset);
                    self.passes = if mode == "selftest" {
                        1
                    } else {
                        preset.passes()
                    };
                    self.inject_fault = mode == "selftest";
                    // Automated runs use 10 % of the free RAM and never change the saved setting.
                    let keep = self.settings.mem_percent;
                    self.settings.mem_percent = 10.0;
                    self.start_test();
                    self.settings.mem_percent = keep;
                }
            }
        }
        let busy = self.busy();
        let Some((path, start, requested, done)) = &mut self.auto_shot else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(100));
        if done.load(Ordering::Acquire) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if !*requested && !busy && start.elapsed() > Duration::from_secs(5) {
            *requested = true;
            let (path, done) = (path.clone(), done.clone());
            ctx.request_screenshot(move |img| {
                let bytes: Vec<u8> = img
                    .pixels
                    .iter()
                    .flat_map(|c| c.to_srgba_unmultiplied())
                    .collect();
                if let Some(i) =
                    image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes)
                {
                    let _ = i.save(&path);
                }
                done.store(true, Ordering::Release);
            });
        }
    }

    /// News (if updates/news are enabled) and the opt-in statistics ping.
    fn start_online(&mut self) {
        let ver = env!("CARGO_PKG_VERSION");
        if self.settings.check_updates && self.news.lock().unwrap().is_none() {
            self.news_seen_before = self.settings.last_news_id;
            let (news, msgs) = (self.news.clone(), self.async_msgs.clone());
            std::thread::spawn(move || match online::fetch_news("de", ver) {
                Ok(items) => {
                    msgs.lock().unwrap().push((
                        Level::Info,
                        format!("Neuigkeiten geladen: {} Einträge", items.len()),
                    ));
                    *news.lock().unwrap() = Some(items);
                }
                Err(e) => msgs
                    .lock()
                    .unwrap()
                    .push((Level::Info, format!("Neuigkeiten nicht abrufbar: {e}"))),
            });
        }
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        // Automated runs (--screenshot, --start-test, --start-bench) never send statistics.
        if self.settings.stats_consent && self.settings.last_ping != today && !automation_mode() {
            if !online::is_uuid_v4(&self.settings.install_id) {
                self.settings.install_id = online::new_install_id();
            }
            // Recorded before sending: at most one attempt per day, even when offline.
            self.settings.last_ping = today;
            self.save_settings();
            let (id, os, msgs) = (
                self.settings.install_id.clone(),
                self.sys.os.clone(),
                self.async_msgs.clone(),
            );
            std::thread::spawn(move || {
                let m = match online::ping(&id, ver, "windows", &os) {
                    Ok(()) => "Anonyme Statistik gesendet".to_string(),
                    Err(e) => format!("Anonyme Statistik nicht gesendet: {e}"),
                };
                msgs.lock().unwrap().push((Level::Info, m));
            });
        }
    }

    fn set_stats_consent(&mut self, on: bool) {
        if on {
            self.settings.stats_consent = true;
            self.settings.install_id = online::new_install_id();
            self.settings.last_ping.clear();
            self.log.info("Anonyme Statistik eingeschaltet");
            self.online_started = false; // sends the first ping on the next frame
        } else {
            let id = std::mem::take(&mut self.settings.install_id);
            self.settings.stats_consent = false;
            self.settings.last_ping.clear();
            self.log
                .info("Anonyme Statistik ausgeschaltet, Löschanfrage wird gesendet");
            let msgs = self.async_msgs.clone();
            std::thread::spawn(move || {
                let m = match online::forget(&id) {
                    Ok(()) => (Level::Info, "Server hat die Statistikdaten dieser Installation gelöscht".to_string()),
                    Err(e) => (
                        Level::Warn,
                        format!("Löschanfrage fehlgeschlagen ({e}). Die lokale ID ist gelöscht; ohne Widerruf löscht der Server nach 13 Monaten."),
                    ),
                };
                msgs.lock().unwrap().push(m);
            });
        }
        self.save_settings();
    }

    fn unread_news(&self) -> usize {
        self.news
            .lock()
            .unwrap()
            .as_ref()
            .map(|v| {
                v.iter()
                    .filter(|n| n.id > self.settings.last_news_id)
                    .count()
            })
            .unwrap_or(0)
    }

    fn mark_news_read(&mut self) {
        let max = self
            .news
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|v| v.iter().map(|n| n.id).max());
        if let Some(max) = max {
            if max > self.settings.last_news_id {
                self.settings.last_news_id = max;
                self.save_settings();
            }
        }
    }

    /// Update check: starts once after the disclaimer was accepted, never blocks.
    fn update_tick(&mut self, ctx: &egui::Context) {
        if !self.update_started && self.settings.check_updates {
            self.update_started = true;
            self.updater
                .check_in_background(&self.settings.skipped_update);
        }
        if !self.online_started {
            self.online_started = true;
            self.start_online();
        }
        for (warn, m) in self.updater.take_log() {
            self.log
                .add(if warn { Level::Warn } else { Level::Info }, m);
        }
        match self.updater.show(ctx, self.busy()) {
            ramforge_update::ui::UiEvent::Skip(v) => {
                self.log.info(format!("Update {v} wird übersprungen"));
                self.settings.skipped_update = v;
                self.save_settings();
            }
            ramforge_update::ui::UiEvent::Restart => {
                self.log
                    .info("Neue Version gestartet, RAMForge wird beendet");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ramforge_update::ui::UiEvent::None => {}
        }
    }
}

fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Started by the boot stick's session script (RAMFORGE_BOOT=1).
pub fn boot_edition() -> bool {
    std::env::var("RAMFORGE_BOOT")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Boot variant "Nur RAM-Test" (RAMFORGE_VARIANT=ram): RAM test, sensors and
/// SPD only, no benchmarks.
pub fn ram_only() -> bool {
    boot_edition()
        && std::env::var("RAMFORGE_VARIANT")
            .map(|v| v == "ram")
            .unwrap_or(false)
}

/// Power off or reboot the live system (boot edition only).
fn power(action: &str) {
    if boot_edition() {
        let _ = std::process::Command::new("systemctl").arg(action).spawn();
    }
}

/// True for unattended runs (docs, CI, self tests): no user input, no statistics.
fn automation_mode() -> bool {
    std::env::args().any(|a| a == "--screenshot" || a == "--start-test" || a == "--start-bench")
}

fn apply_theme(ctx: &egui::Context, theme: &str) {
    ctx.set_theme(match theme {
        "light" => egui::ThemePreference::Light,
        "system" => egui::ThemePreference::System,
        _ => egui::ThemePreference::Dark,
    });
}

impl eframe::App for RamForgeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if automation_mode() {
            // An automated run must not react to clicks or keys that happen to
            // hit its window (e.g. the user working while a test window opens).
            ctx.input_mut(|i| i.events.clear());
        }
        self.poll();
        self.shortcuts(&ctx);
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::right("sidebar")
            .resizable(true)
            .default_size(280.0)
            .min_size(220.0)
            .show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::System => scroll(ui, |ui| self.tab_system(ui)),
            Tab::Test => scroll(ui, |ui| self.tab_test(ui)),
            Tab::Bench => scroll(ui, |ui| self.tab_bench(ui)),
            Tab::Sensors => self.tab_sensors(ui),
            Tab::Spd => scroll(ui, |ui| self.tab_spd(ui)),
            Tab::Log => self.tab_log(ui),
            Tab::About => scroll(ui, |ui| self.tab_about(ui)),
        });
        if !self.accepted() {
            self.disclaimer_modal(&ctx);
        } else {
            self.update_tick(&ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(if self.busy() { 200 } else { 500 }));
        self.auto_screenshot(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(r) = &self.run {
            r.stop();
        }
        self.hub.shutdown();
        self.save_settings();
    }
}

fn scroll(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, f);
}

fn heading(ui: &mut egui::Ui, t: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(t).size(18.0).strong());
    ui.add_space(2.0);
}

pub(crate) fn fmt_val(v: f64, unit: &str) -> String {
    if !v.is_finite() {
        return "–".into();
    }
    match unit {
        "V" => format!("{v:.3} V"),
        "%" | "°C" | "W" | "A" => format!("{v:.1} {unit}"),
        "MHz" | "RPM" => format!("{v:.0} {unit}"),
        "GiB" | "GB" => format!("{v:.2} {unit}"),
        "" => format!("{v:.2}"),
        u => format!("{v:.2} {u}"),
    }
}

fn temp_color(s: &SensorStat) -> Option<Color32> {
    if s.reading.kind != SensorKind::Temperature {
        return None;
    }
    let v = s.reading.value;
    let hot = if s.reading.group == SensorGroup::Memory {
        (55.0, 70.0)
    } else {
        (75.0, 90.0)
    };
    if v >= hot.1 {
        Some(BAD)
    } else if v >= hot.0 {
        Some(WARN)
    } else {
        None
    }
}

/// Picks the values that matter most during a RAM test for the side panel.
fn key_sensors(all: &[SensorStat], pinned: &[String]) -> Vec<SensorStat> {
    let mut out: Vec<SensorStat> = Vec::new();
    let mut take = |pred: &dyn Fn(&SensorStat) -> bool, max: usize| {
        for s in all.iter().filter(|s| pred(s)).take(max) {
            if !out.iter().any(|o| o.reading.id == s.reading.id) {
                out.push(s.clone());
            }
        }
    };
    let lab = |s: &SensorStat, w: &[&str]| {
        let l = s.reading.label.to_lowercase();
        w.iter().any(|x| l.contains(x))
    };
    take(&|s| pinned.contains(&s.reading.id), 32);
    take(
        &|s| {
            s.reading.group == SensorGroup::Cpu
                && s.reading.kind == SensorKind::Temperature
                && lab(s, &["tctl", "tdie", "package", "die", "cpu"])
        },
        2,
    );
    take(
        &|s| s.reading.group == SensorGroup::Cpu && s.reading.kind == SensorKind::Temperature,
        1,
    );
    take(
        &|s| {
            s.reading.group == SensorGroup::Cpu
                && s.reading.kind == SensorKind::Load
                && lab(s, &["gesamt", "total"])
        },
        1,
    );
    take(
        &|s| {
            s.reading.group == SensorGroup::Cpu
                && s.reading.kind == SensorKind::Clock
                && !lab(s, &["thread", "core #", "bus"])
        },
        1,
    );
    take(
        &|s| {
            s.reading.group == SensorGroup::Cpu
                && s.reading.kind == SensorKind::Power
                && lab(s, &["package", "ppt", "cpu"])
        },
        1,
    );
    take(
        &|s| s.reading.group == SensorGroup::Memory && s.reading.kind == SensorKind::Temperature,
        4,
    );
    take(
        &|s| {
            s.reading.kind == SensorKind::Voltage
                && lab(s, &["dram", "vdimm", "vdd", "dimm", "vddq"])
        },
        2,
    );
    take(
        &|s| s.reading.group == SensorGroup::Memory && s.reading.kind == SensorKind::Load,
        1,
    );
    take(
        &|s| s.reading.group == SensorGroup::Memory && lab(s, &["belegt", "used"]),
        1,
    );
    take(
        &|s| s.reading.group == SensorGroup::Mainboard && s.reading.kind == SensorKind::Temperature,
        3,
    );
    take(
        &|s| s.reading.group == SensorGroup::Storage && s.reading.kind == SensorKind::Temperature,
        1,
    );
    take(
        &|s| s.reading.group == SensorGroup::System && lab(s, &["error", "fehler"]),
        2,
    );
    out
}

fn md_view(ui: &mut egui::Ui, md: &str) {
    for line in md.lines() {
        let l = line.trim_end().replace("**", "");
        if l.is_empty() || l == "---" {
            ui.add_space(4.0);
        } else if let Some(h) = l.strip_prefix("### ") {
            ui.label(RichText::new(h).strong());
        } else if let Some(h) = l.strip_prefix("## ") {
            ui.add_space(6.0);
            ui.label(RichText::new(h).size(17.0).strong().color(ACCENT));
        } else if let Some(h) = l.strip_prefix("# ") {
            ui.label(RichText::new(h).size(20.0).strong());
        } else if let Some(q) = l.strip_prefix("> ") {
            ui.label(RichText::new(q).italics().weak());
        } else if let Some(b) = l.strip_prefix("- ") {
            ui.label(format!("  •  {b}"));
        } else {
            ui.label(l.trim());
        }
    }
}

// -------------------------------------------------------------------- panels

impl RamForgeApp {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(t) = &self.logo {
                ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(26.0, 26.0)));
            }
            ui.label(RichText::new("RAM").size(22.0).strong().color(ACCENT));
            ui.add_space(-8.0);
            ui.label(RichText::new("Forge").size(22.0).strong());
            ui.add_space(12.0);
            for (i, t) in Tab::visible().iter().enumerate() {
                let r = ui.selectable_label(self.tab == *t, RichText::new(t.title()).size(15.0));
                if r.on_hover_text(format!("Strg+{}", i + 1)).clicked() {
                    self.tab = *t;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Boot edition: there is no desktop, so power off / reboot live here.
                if boot_edition() {
                    if ui
                        .add_enabled(!self.busy(), egui::Button::new("⏻ Aus"))
                        .on_hover_text("Rechner ausschalten")
                        .clicked()
                    {
                        power("poweroff");
                    }
                    if ui
                        .add_enabled(!self.busy(), egui::Button::new("⟳ Neustart"))
                        .on_hover_text("Rechner neu starten")
                        .clicked()
                    {
                        power("reboot");
                    }
                    ui.separator();
                }
                let (icon, next, tip) = match self.settings.theme.as_str() {
                    "light" => ("🌙", "dark", "Dunkles Design"),
                    _ => ("☀", "light", "Helles Design"),
                };
                if ui.button(icon).on_hover_text(tip).clicked() {
                    self.settings.theme = next.into();
                    apply_theme(ui.ctx(), next);
                    self.save_settings();
                }
                let unread = self.unread_news();
                if unread > 0
                    && ui
                        .button(RichText::new(format!("📰 {unread} neu")).color(ACCENT))
                        .on_hover_text("Neuigkeiten anzeigen")
                        .clicked()
                {
                    self.tab = Tab::About;
                }
                if ui.button("📷").on_hover_text("Screenshot (F12)").clicked() {
                    let ctx = ui.ctx().clone();
                    self.screenshot(&ctx);
                }
                if ui
                    .button("💾 Bericht")
                    .on_hover_text("Bericht als HTML, JSON und TXT speichern (Strg+S)")
                    .clicked()
                {
                    self.save_report();
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        if std::env::var("RAMFORGE_DATA_VOLATILE")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            ui.label(
                RichText::new(
                    "⚠ Berichte und Logs liegen nur im Arbeitsspeicher und gehen beim Ausschalten verloren. \
                     Mit RAMForge-USB.exe eingerichtete Sticks speichern sie in RAMForge\\RAMForge-data.",
                )
                .color(WARN)
                .strong(),
            );
        }
        ui.horizontal(|ui| {
            for (name, status) in self.hub.source_status() {
                ui.label(RichText::new(format!("{name}: {status}")).small().weak());
                ui.separator();
            }
            if let Some(p) = self.hub.recording_path() {
                ui.label(
                    RichText::new(format!(
                        "● Aufzeichnung {}",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    ))
                    .small()
                    .color(BAD),
                );
                ui.separator();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new("F5 Start · Esc Stopp · F12 Screenshot · Strg+S Bericht")
                        .small()
                        .weak(),
                );
            });
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        // Test status
        if let Some(run) = &self.run {
            let running = run.running.load(Ordering::Acquire);
            let errors = run.sink.count();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if running {
                        "RAM-Test läuft"
                    } else {
                        "RAM-Test beendet"
                    })
                    .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let c = if errors > 0 { BAD } else { OK };
                    ui.label(RichText::new(format!("{errors} Fehler")).strong().color(c));
                });
            });
            if running {
                ui.add(
                    egui::ProgressBar::new(run.pass_fraction() as f32).text(format!(
                        "Durchlauf {} · {:.0} %",
                        run.pass.load(Ordering::Relaxed) + 1,
                        run.pass_fraction() * 100.0
                    )),
                );
                ui.label(RichText::new(run.current_test().map(|t| t.name()).unwrap_or("")).small());
                ui.label(
                    RichText::new(format!(
                        "Laufzeit {}",
                        report::duration(run.elapsed().as_secs_f64())
                    ))
                    .small()
                    .weak(),
                );
            }
            ui.separator();
        }
        ui.label(RichText::new("Sensoren live").strong());
        let snap = self.hub.snapshot_light();
        let keys = key_sensors(&snap, &self.settings.pinned_sensors);
        if keys.is_empty() {
            ui.label(RichText::new("Noch keine Werte…").weak());
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut last_group = None;
            egui::Grid::new("side_sensors").num_columns(2).striped(true).show(ui, |ui| {
                for s in &keys {
                    if last_group != Some(s.reading.group) {
                        last_group = Some(s.reading.group);
                        ui.label(RichText::new(s.reading.group.name()).small().color(ACCENT));
                        ui.end_row();
                    }
                    let tip = format!(
                        "{} · {}\nmin {}  max {}  Ø {}",
                        s.reading.device, s.reading.source,
                        fmt_val(s.min, &s.reading.unit), fmt_val(s.max, &s.reading.unit), fmt_val(s.avg(), &s.reading.unit)
                    );
                    ui.label(&s.reading.label).on_hover_text(tip);
                    let mut t = RichText::new(fmt_val(s.reading.value, &s.reading.unit)).monospace();
                    if let Some(c) = temp_color(s) {
                        t = t.color(c).strong();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(t));
                    ui.end_row();
                }
            });
            if !snap.iter().any(|s| s.reading.kind == SensorKind::Temperature) {
                ui.add_space(8.0);
                let hint = if cfg!(windows) {
                    "Keine Temperaturen verfügbar. Unter Windows: HWiNFO64 starten und „Shared Memory Support“ aktivieren, dann erscheinen CPU-, RAM- und Board-Sensoren hier automatisch."
                } else {
                    "Keine Temperaturen verfügbar. Linux hat für dieses System keine Sensor-Treiber (hwmon) geladen. In virtuellen Maschinen ist das normal."
                };
                ui.label(RichText::new(hint).small().weak());
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Weitere Werte im Tab Sensoren anheften (📌).").small().weak());
        });
    }

    fn disclaimer_modal(&mut self, ctx: &egui::Context) {
        egui::Modal::new(egui::Id::unique("disclaimer")).show(ctx, |ui| {
            ui.set_max_width(760.0);
            ui.label(RichText::new("Bitte vor der ersten Nutzung lesen / Please read before first use").size(18.0).strong());
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| md_view(ui, DISCLAIMER_MD));
            ui.separator();
            ui.checkbox(
                &mut self.disclaimer_check,
                "Ich habe den Haftungsausschluss gelesen und akzeptiere ihn. / I have read and accept the disclaimer.",
            );
            ui.add_space(6.0);
            ui.checkbox(&mut self.stats_choice, "Freiwillig: anonyme Nutzungsstatistik senden / Optional: send anonymous usage statistics");
            ui.label(
                RichText::new(
                    "Höchstens einmal am Tag: eine zufällige Installations-ID (nicht aus der Hardware abgeleitet), Programmversion, Edition und grobe Windows-Version (z. B. „Windows 11“). \
                     Der Server bestimmt daraus nur das Land und verwirft die IP-Adresse. Keine Hardwaredaten, Seriennummern, Ergebnisse oder Logs. Jederzeit im Tab „Über“ widerrufbar.",
                )
                .small()
                .weak(),
            );
            ui.horizontal(|ui| {
                ui.hyperlink_to(RichText::new("Datenschutz").small(), URL_PRIVACY);
                ui.hyperlink_to(RichText::new("Haftungsausschluss online").small(), URL_DISCLAIMER);
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(self.disclaimer_check, egui::Button::new("Akzeptieren / Accept")).clicked() {
                    self.settings.disclaimer_accepted = DISCLAIMER_VERSION;
                    if self.stats_choice {
                        self.set_stats_consent(true);
                    }
                    self.save_settings();
                    self.log.info(format!("Haftungsausschluss Version {DISCLAIMER_VERSION} akzeptiert"));
                }
                if ui.button("Beenden / Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }

    // ---------------------------------------------------------------- tabs

    fn tab_system(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            heading(ui, "System");
            if ui.button("⟳ Neu einlesen").clicked() {
                self.sys = ramforge_hw::collect();
                self.log.info("Systeminformationen neu eingelesen");
            }
        });
        let s = &self.sys;
        egui::Grid::new("sys")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(RichText::new(k).weak());
                    ui.label(v);
                    ui.end_row();
                };
                row("Betriebssystem", s.os.clone());
                row("Prozessor", s.cpu.brand.clone());
                row(
                    "CPU-Details",
                    format!(
                        "{} · Familie {:X}h Modell {:X}h Stepping {} · {} Threads{}",
                        s.cpu.vendor,
                        s.cpu.family,
                        s.cpu.model,
                        s.cpu.stepping,
                        s.cpu.logical_processors,
                        if s.cpu.hypervisor {
                            " · Hypervisor aktiv (VM oder Hyper-V/VBS)"
                        } else {
                            ""
                        }
                    ),
                );
                row("Befehlssätze", s.cpu.features.join(", "));
                row(
                    "Arbeitsspeicher",
                    format!(
                        "{} gesamt, {} frei",
                        report::gib(s.memory.total_bytes),
                        report::gib(s.memory.available_bytes)
                    ),
                );
                if let Some(b) = &s.smbios {
                    row(
                        "Mainboard",
                        format!(
                            "{} {} {}",
                            b.board_manufacturer, b.board_product, b.board_version
                        ),
                    );
                    row(
                        "System",
                        format!("{} {}", b.system_manufacturer, b.system_product),
                    );
                    row(
                        "BIOS / UEFI",
                        format!("{} {} ({})", b.bios_vendor, b.bios_version, b.bios_date),
                    );
                    for c in &b.processors {
                        row(
                            "CPU-Sockel",
                            format!(
                                "{}: {} Kerne / {} Threads, Basistakt {} MHz, Referenztakt {} MHz",
                                c.socket,
                                c.cores,
                                c.threads,
                                c.current_speed_mhz,
                                c.external_clock_mhz
                            ),
                        );
                    }
                    let used = b.memory.iter().filter(|m| m.is_installed()).count();
                    row(
                        "Speicherplätze",
                        format!(
                            "{used} von {} belegt · max. {} laut Firmware · SMBIOS {}",
                            b.memory_slots_total.max(b.memory.len() as u32),
                            report::gib(b.memory_max_capacity_bytes),
                            b.version
                        ),
                    );
                }
            });
        if let Some(b) = &s.smbios {
            heading(ui, "Speichermodule");
            ui.checkbox(&mut self.show_serials, "Seriennummern anzeigen");
            egui::Grid::new("dimms")
                .num_columns(10)
                .striped(true)
                .show(ui, |ui| {
                    for h in [
                        "Steckplatz",
                        "Größe",
                        "Typ",
                        "Nenntakt",
                        "Ist-Takt",
                        "Spannung",
                        "Ranks",
                        "ECC",
                        "Hersteller",
                        "Teilenummer / Serie",
                    ] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for m in &b.memory {
                        ui.label(format!("{} {}", m.bank, m.locator).trim());
                        if !m.is_installed() {
                            ui.label(RichText::new("leer").weak());
                            for _ in 0..8 {
                                ui.label("");
                            }
                            ui.end_row();
                            continue;
                        }
                        ui.label(report::gib(m.size_bytes));
                        ui.label(format!("{} {}", m.memory_type, m.form_factor));
                        ui.label(format!("{} MT/s", m.speed_mts));
                        let c =
                            if m.configured_speed_mts > 0 && m.configured_speed_mts < m.speed_mts {
                                WARN
                            } else {
                                ui.visuals().text_color()
                            };
                        ui.label(
                            RichText::new(format!("{} MT/s", m.configured_speed_mts)).color(c),
                        );
                        ui.label(if m.configured_voltage_mv > 0 {
                            format!("{:.3} V", m.configured_voltage_mv as f64 / 1000.0)
                        } else {
                            "–".into()
                        });
                        ui.label(if m.ranks > 0 {
                            m.ranks.to_string()
                        } else {
                            "–".into()
                        });
                        ui.label(if m.has_ecc() { "ja" } else { "nein" });
                        ui.label(&m.manufacturer);
                        ui.label(if self.show_serials {
                            format!("{}  /  {}", m.part_number, m.serial)
                        } else {
                            m.part_number.clone()
                        });
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Nenn- und Ist-Takt sowie Spannung stammen aus der Firmware (SMBIOS). Ein Ist-Takt unter dem Nenntakt heißt meist: XMP/EXPO ist im BIOS nicht aktiv. \
                     Timings, SPD-Inhalt und XMP/EXPO-Profile: siehe Tab SPD. Unter Windows liefert HWiNFO (Shared Memory) die aktuellen Timings im Tab Sensoren.",
                )
                .small()
                .weak(),
            );
        }
    }

    fn tab_test(&mut self, ui: &mut egui::Ui) {
        heading(ui, "RAM-Test");
        let running = self.running();
        let avail = ramforge_core::mem::mem_status().available_bytes;
        ui.add_enabled_ui(!self.busy(), |ui| {
            egui::Grid::new("testcfg").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                ui.label("Profil");
                ui.horizontal(|ui| {
                    for p in Preset::ALL {
                        if ui.selectable_label(self.preset == Some(p), p.name()).clicked() {
                            self.preset = Some(p);
                            self.passes = p.passes();
                        }
                    }
                    if ui.selectable_label(self.preset.is_none(), "Eigene Auswahl").clicked() {
                        self.preset = None;
                    }
                });
                ui.end_row();

                ui.vertical(|ui| ui.label("Tests"));
                ui.vertical(|ui| match self.preset {
                    Some(p) => {
                        let names: Vec<&str> = p.tests().iter().map(|t| t.name()).collect();
                        ui.label(RichText::new(names.join(" → ")).small());
                    }
                    None => {
                        egui::Grid::new("custom").num_columns(3).show(ui, |ui| {
                            for (i, (k, on)) in self.custom.iter_mut().enumerate() {
                                ui.checkbox(on, k.name()).on_hover_text(k.description());
                                if i % 3 == 2 {
                                    ui.end_row();
                                }
                            }
                        });
                    }
                });
                ui.end_row();

                ui.label("Speicher");
                ui.horizontal(|ui| {
                    if ui.add(egui::Slider::new(&mut self.settings.mem_percent, 10.0..=95.0).suffix(" %")).changed() {
                        self.save_settings();
                    }
                    let bytes = (avail as f64 * self.settings.mem_percent as f64 / 100.0) as u64;
                    ui.label(format!("≈ {} von {} frei", report::gib(bytes), report::gib(avail)));
                });
                ui.end_row();

                ui.label("Threads");
                ui.add(egui::Slider::new(&mut self.threads, 1..=self.sys.cpu.logical_processors.max(1)));
                ui.end_row();

                ui.label("Durchläufe");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.passes).range(0..=10_000));
                    ui.label(RichText::new(if self.passes == 0 { "endlos, bis Stopp" } else { "0 = endlos" }).weak());
                });
                ui.end_row();

                ui.label("Bit-Fade-Wartezeit");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.bitfade_secs).range(1..=7200).suffix(" s"));
                    ui.label(RichText::new("je Muster, nur für Bit Fade").weak());
                });
                ui.end_row();

                ui.vertical(|ui| ui.label("Sensoren"));
                ui.vertical(|ui| self.record_controls(ui));
                ui.end_row();

                ui.vertical(|ui| ui.label("Optionen"));
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.lock_memory, "Testspeicher im RAM sperren (empfohlen, verhindert Auslagern)");
                    ui.checkbox(&mut self.stop_on_error, "Beim ersten Fehler anhalten");
                    ui.checkbox(&mut self.inject_fault, "Selbsttest: einen künstlichen Bitfehler einspeisen")
                        .on_hover_text("Prüft, ob Fehlererkennung, Protokoll und Bericht funktionieren. Der Fehler ist als „Selbsttest“ markiert.");
                });
                ui.end_row();
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let start =
                egui::Button::new(RichText::new("▶  Test starten (F5)").size(16.0).strong())
                    .fill(ACCENT);
            if ui
                .add_enabled(!self.busy() && self.accepted(), start)
                .clicked()
            {
                self.start_test();
            }
            if ui
                .add_enabled(
                    running,
                    egui::Button::new(RichText::new("⏹  Stopp (Esc)").size(16.0)),
                )
                .clicked()
            {
                self.stop_test();
            }
        });
        let hint = if cfg!(windows) {
            "Unter Windows bleiben einige hundert MB für das System reserviert. Für 100 % Abdeckung gibt es die Boot-Variante."
        } else {
            "Ausgenommen ist nur der Speicher, den Linux und die Oberfläche selbst belegen (einige hundert MB)."
        };
        ui.label(RichText::new(hint).small().weak());

        ui.add_space(10.0);
        if let Some(run) = self.run.clone() {
            ui.separator();
            ui.label(RichText::new(run.phase.lock().unwrap().clone()).strong());
            if running {
                ui.add(
                    egui::ProgressBar::new(run.pass_fraction() as f32)
                        .show_percentage()
                        .text(format!(
                            "Durchlauf {} von {}",
                            run.pass.load(Ordering::Relaxed) + 1,
                            if run.config.passes == 0 {
                                "∞".to_string()
                            } else {
                                run.config.passes.to_string()
                            }
                        )),
                );
                ui.add(
                    egui::ProgressBar::new(run.test_fraction() as f32)
                        .show_percentage()
                        .text(format!(
                            "Test {} von {}: {}",
                            run.test_index.load(Ordering::Relaxed) + 1,
                            run.config.tests.len(),
                            run.current_test().map(|t| t.name()).unwrap_or("")
                        )),
                );
            }
            egui::Grid::new("runinfo").num_columns(2).show(ui, |ui| {
                ui.label(RichText::new("Laufzeit").weak());
                ui.label(report::duration(run.elapsed().as_secs_f64()));
                ui.end_row();
                ui.label(RichText::new("Getestet").weak());
                ui.label(format!(
                    "{} mit {} Threads{}",
                    report::gib(run.allocated_bytes.load(Ordering::Relaxed)),
                    run.config.threads,
                    if run.all_locked.load(Ordering::Relaxed) {
                        ", gesperrt"
                    } else {
                        ", nicht vollständig gesperrt"
                    }
                ));
                ui.end_row();
                ui.label(RichText::new("Fehler").weak());
                let n = run.sink.count();
                ui.label(
                    RichText::new(n.to_string())
                        .size(18.0)
                        .strong()
                        .color(if n > 0 { BAD } else { OK }),
                );
                ui.end_row();
            });
            if let (Some(s), false) = (&self.last_summary, running) {
                let (c, t) = match s.outcome {
                    Outcome::Passed => (OK, "BESTANDEN"),
                    Outcome::Failed => (BAD, "FEHLER GEFUNDEN"),
                    Outcome::Cancelled => (WARN, "ABGEBROCHEN"),
                    Outcome::Aborted => (BAD, "NICHT GESTARTET"),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t).size(20.0).strong().color(c));
                    if let Some(p) = &self.last_report {
                        if ui.button("Bericht öffnen").clicked() {
                            paths::open_folder(p);
                        }
                    }
                });
            }
        }

        if !self.running() {
            if let Some(d) = self.sessions.iter().rev().find(|s| s.label == "test") {
                ui.separator();
                crate::chart::show(ui, d, &mut self.chart_test, "chart-test");
            }
        }
        if !self.errors.is_empty() {
            heading(ui, "Betroffene Datenbits");
            self.bit_strip(ui);
            heading(
                ui,
                &format!(
                    "Fehler ({} gesamt, die neuesten 500)",
                    self.run.as_ref().map(|r| r.sink.count()).unwrap_or(0)
                ),
            );
            egui::ScrollArea::vertical()
                .id_salt("errs")
                .max_height(320.0)
                .show(ui, |ui| {
                    egui::Grid::new("errtab")
                        .num_columns(8)
                        .striped(true)
                        .show(ui, |ui| {
                            for h in [
                                "Zeit", "Test", "Durchl.", "Thread", "Adresse", "Soll", "Ist",
                                "Bits",
                            ] {
                                ui.label(RichText::new(h).strong());
                            }
                            ui.end_row();
                            for e in self.errors.iter().rev().take(500) {
                                ui.label(e.time.format("%H:%M:%S").to_string());
                                ui.label(if e.simulated {
                                    format!("{} (Selbsttest)", e.test)
                                } else {
                                    e.test.clone()
                                });
                                ui.label((e.pass + 1).to_string());
                                ui.label(e.thread.to_string());
                                ui.monospace(format!("0x{:012X}", e.address));
                                ui.monospace(format!("{:016X}", e.expected));
                                ui.monospace(
                                    RichText::new(format!("{:016X}", e.actual)).color(BAD),
                                );
                                ui.monospace(format!("{} ({:016X})", e.flipped_bits(), e.xor()));
                                ui.end_row();
                            }
                        });
                });
        }
    }

    fn bit_strip(&self, ui: &mut egui::Ui) {
        let max = self.bits.iter().copied().max().unwrap_or(0).max(1) as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for b in (0..64).rev() {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(10.0, 22.0), egui::Sense::hover());
                let n = self.bits[b];
                let col = if n == 0 {
                    ui.visuals().faint_bg_color
                } else {
                    BAD.gamma_multiply(0.35 + 0.65 * n as f32 / max)
                };
                ui.painter().rect_filled(rect, 2.0, col);
                resp.on_hover_text(format!("Bit {b}: {n}× falsch"));
            }
        });
        ui.label(RichText::new("Bit 63 links … Bit 0 rechts. Immer dasselbe Bit deutet auf einen Chip bzw. eine Datenleitung, verstreute Bits eher auf Takt, Timings oder Spannung.").small().weak());
    }

    fn tab_bench(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Benchmark");
        ui.add_enabled_ui(!self.busy(), |ui| {
            egui::Grid::new("benchcfg")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.vertical(|ui| ui.label("Benchmarks"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(
                                &mut self.bench_sel.cpu,
                                "CPU (Ganzzahl, Gleitkomma, SIMD)",
                            );
                            ui.checkbox(&mut self.bench_sel.ram, "RAM (Bandbreite, Latenz)");
                            ui.checkbox(&mut self.bench_sel.stress, "CPU+RAM unter Last");
                            ui.checkbox(&mut self.bench_sel.gpu, "GPU (Zusatzmodul)");
                            if ui.button("Alle").clicked() {
                                self.bench_sel = crate::benchsuite::Selection {
                                    cpu: true,
                                    ram: true,
                                    stress: true,
                                    gpu: true,
                                };
                            }
                        });
                        self.gpu_module_row(ui);
                    });
                    ui.end_row();
                    ui.label("Dauer Last-Test");
                    ui.horizontal(|ui| {
                        for s in [30u64, 60, 120, 300] {
                            if ui
                                .selectable_label(self.bench_stress_secs == s, format!("{s} s"))
                                .clicked()
                            {
                                self.bench_stress_secs = s;
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Threads");
                    ui.add(egui::Slider::new(
                        &mut self.bench_threads,
                        1..=self.sys.cpu.logical_processors.max(1),
                    ));
                    ui.end_row();
                    ui.label("Puffer je Thread");
                    ui.add(egui::Slider::new(&mut self.bench_mib, 32..=1024).suffix(" MiB"));
                    ui.end_row();
                });
        });
        ui.horizontal(|ui| {
            let b = egui::Button::new(
                RichText::new("▶  Benchmark starten (F5)")
                    .size(16.0)
                    .strong(),
            )
            .fill(ACCENT);
            if ui.add_enabled(!self.busy() && self.accepted(), b).clicked() {
                self.start_bench();
            }
            if ui
                .add_enabled(self.bench.is_some(), egui::Button::new("⏹  Abbrechen"))
                .clicked()
            {
                self.stop_test();
            }
        });
        if let Some(b) = &self.bench {
            let (f, step, detail) = b.progress.lock().unwrap().clone();
            ui.add(
                egui::ProgressBar::new(f)
                    .show_percentage()
                    .text(if detail.is_empty() {
                        format!("{step} …")
                    } else {
                        format!("{step} · {detail}")
                    }),
            );
        }
        ui.add_enabled_ui(!self.busy(), |ui| self.record_controls(ui));
        if let Some(run) = self.bench_run.clone() {
            self.bench_suite_results(ui, &run);
        }
        if !self.bench_history.is_empty() {
            self.bench_history_ui(ui);
        }
        if let Some(r) = &self.bench_result {
            heading(ui, "RAM");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (name, v) in [
                    ("Lesen", r.read_gbs),
                    ("Schreiben", r.write_gbs),
                    ("Kopieren", r.copy_gbs),
                ] {
                    egui::Frame::group(ui.style())
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(name).weak());
                                ui.label(RichText::new(format!("{v:.2} GB/s")).size(24.0).strong());
                            });
                        });
                }
                if let Some(ns) = r.dram_latency_ns() {
                    egui::Frame::group(ui.style())
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new("Latenz (RAM)").weak());
                                ui.label(RichText::new(format!("{ns:.1} ns")).size(24.0).strong());
                            });
                        });
                }
            });
            heading(ui, "Latenz nach Blockgröße");
            let max = r.latency.iter().map(|l| l.1).fold(1.0, f64::max);
            egui::Grid::new("lat").num_columns(3).show(ui, |ui| {
                for (size, ns) in &r.latency {
                    ui.label(report::size_label(*size));
                    ui.add(egui::ProgressBar::new((*ns / max) as f32).desired_width(320.0));
                    ui.monospace(format!("{ns:.1} ns"));
                    ui.end_row();
                }
            });
            ui.label(
                RichText::new(format!(
                    "{} Threads, {} Puffer, bester von 4 Durchläufen. Kleine Blöcke messen die CPU-Caches (L1/L2/L3), große den Arbeitsspeicher. „Kopieren“ zählt kopierte Bytes. \
                     Die Latenz enthält TLB-Fehlgriffe (normale 4-KiB-Seiten); Werkzeuge mit Large Pages zeigen niedrigere Werte. \
                     Schreiben nutzt normale Stores ohne Non-Temporal-Optimierung.",
                    r.threads,
                    report::gib(r.buffer_bytes)
                ))
                .small()
                .weak(),
            );
        }
        if self.bench.is_none() {
            if let Some(d) = self.sessions.iter().rev().find(|s| s.label == "bench") {
                ui.separator();
                heading(ui, "Sensoren während des Benchmarks");
                crate::chart::show(ui, d, &mut self.chart_bench, "chart-bench");
                ui.separator();
            }
        }
    }

    fn bench_suite_results(&mut self, ui: &mut egui::Ui, run: &crate::benchsuite::BenchRun) {
        ui.add_space(6.0);
        ui.label(RichText::new(run.settings_text()).small().weak());
        if run.cancelled {
            ui.label(
                RichText::new("Abgebrochen – Ergebnisse unvollständig und nicht im Verlauf.")
                    .color(WARN),
            );
        }
        if !run.cpu.is_empty() {
            heading(ui, "CPU");
            let (st, mt) = run.cpu_points();
            ui.horizontal(|ui| {
                for (name, v) in [("Single-Thread", st), ("Multi-Thread", mt)] {
                    egui::Frame::group(ui.style())
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(name).weak());
                                ui.label(RichText::new(format!("{v} Punkte")).size(24.0).strong());
                            });
                        });
                }
            });
            egui::Grid::new("cpu-res")
                .num_columns(7)
                .striped(true)
                .show(ui, |ui| {
                    for h in [
                        "Test",
                        "Befehlssatz",
                        "1 Thread",
                        "Alle Threads",
                        "Punkte 1T / nT",
                        "Skalierung",
                        "Prüfung",
                    ] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for c in &run.cpu {
                        ui.label(c.kind.name());
                        ui.label(&c.variant);
                        if let Some(why) = &c.skipped {
                            ui.label(RichText::new(why).weak());
                            for _ in 0..4 {
                                ui.label("");
                            }
                        } else {
                            ui.monospace(format!("{:.1} {}", c.single_raw, c.kind.unit()));
                            ui.monospace(format!("{:.1} {}", c.multi_raw, c.kind.unit()));
                            ui.monospace(format!("{} / {}", c.single_points, c.multi_points));
                            ui.monospace(format!("{:.1}× ({} T)", c.scaling, c.threads));
                        }
                        if c.errors > 0 {
                            ui.label(
                                RichText::new(format!("{} Rechenfehler", c.errors))
                                    .color(BAD)
                                    .strong(),
                            );
                        } else {
                            ui.label(RichText::new("ok").color(OK));
                        }
                        ui.end_row();
                    }
                });
            ui.label(RichText::new("Punkte: feste Arbeitslast, 1000 Punkte = Referenzwert. Jede Runde wird gegen eine Prüfsumme kontrolliert; Rechenfehler deuten auf instabile Übertaktung oder zu wenig Spannung.").small().weak());
        }
        if let Some(g) = &run.gpu {
            Self::gpu_results(ui, g);
        }
        if let Some(s) = &run.stress {
            heading(ui, "CPU+RAM unter Last");
            ui.horizontal(|ui| {
                for (name, v) in [
                    ("Ø", s.avg_gbs),
                    ("Minimum", s.min_gbs),
                    ("Maximum", s.max_gbs),
                ] {
                    egui::Frame::group(ui.style())
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(name).weak());
                                ui.label(RichText::new(format!("{v:.1} GB/s")).size(22.0).strong());
                            });
                        });
                }
                egui::Frame::group(ui.style())
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Fehler").weak());
                            ui.label(
                                RichText::new(s.errors.to_string())
                                    .size(22.0)
                                    .strong()
                                    .color(if s.errors > 0 { BAD } else { OK }),
                            );
                        });
                    });
            });
            let pts: Vec<[f64; 2]> = s
                .samples
                .iter()
                .enumerate()
                .map(|(i, v)| [i as f64 + 1.0, *v])
                .collect();
            egui_plot::Plot::new("stress-plot")
                .height(140.0)
                .allow_scroll(false)
                .include_y(0.0)
                .x_axis_formatter(|m, _| crate::chart::fmt_tick(m.value, m.step_size))
                .show(ui, |p| {
                    p.line(
                        egui_plot::Line::new("GB/s", egui_plot::PlotPoints::from(pts))
                            .color(ACCENT),
                    )
                });
            ui.label(RichText::new(format!(
                "Stabilität {:.0} % (langsamste Sekunde im Verhältnis zum Durchschnitt), {} geprüft, {} Threads, {}.",
                s.stability() * 100.0,
                report::gib(s.verified_bytes),
                s.threads,
                report::gib(s.buffer_bytes)
            )).small().weak());
        }
    }

    /// Status of the optional GPU module, with a verified download if missing.
    fn gpu_module_row(&mut self, ui: &mut egui::Ui) {
        match ramforge_gpu::find_module() {
            Some(p) => {
                ui.label(
                    RichText::new(format!(
                        "GPU-Modul installiert: {}",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    ))
                    .small()
                    .weak(),
                );
            }
            None => {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(
                            "GPU-Benchmark ist ein Zusatzmodul (≈ 6 MB), nicht installiert.",
                        )
                        .small()
                        .weak(),
                    );
                    let busy = self.gpu_download.lock().unwrap().is_some();
                    if ui
                        .add_enabled(!busy, egui::Button::new("GPU-Modul herunterladen").small())
                        .clicked()
                    {
                        self.download_gpu_module();
                    }
                    if let Some(s) = self.gpu_download.lock().unwrap().as_ref() {
                        ui.label(RichText::new(s).small());
                    }
                });
            }
        }
    }

    /// Downloads RAMForge-GPU.exe through the signed manifest (Ed25519 +
    /// SHA-256, GitHub first, mirror second) and puts it next to RAMForge.exe.
    fn download_gpu_module(&mut self) {
        let status = self.gpu_download.clone();
        let msgs = self.async_msgs.clone();
        *status.lock().unwrap() = Some("lädt …".into());
        std::thread::spawn(move || {
            let res = (|| -> Result<String, String> {
                let m = ramforge_update::fetch_verified_manifest().map_err(|e| e.to_string())?;
                let asset = m
                    .assets
                    .get(ramforge_update::AssetKind::GpuModule.key())
                    .ok_or("Das GPU-Modul ist für diese Version noch nicht veröffentlicht")?;
                ramforge_update::check_asset(asset).map_err(|e| e.to_string())?;
                let dir = std::env::current_exe()
                    .map_err(|e| e.to_string())?
                    .parent()
                    .ok_or("kein Programmordner")?
                    .to_path_buf();
                let tmp = dir.join(format!("{}.new", ramforge_gpu::MODULE_FILE));
                let got =
                    ramforge_update::download(asset, &tmp, &|_, _| {}, &AtomicBool::new(false))
                        .map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, dir.join(ramforge_gpu::MODULE_FILE))
                    .map_err(|e| e.to_string())?;
                Ok(format!(
                    "GPU-Modul installiert (Quelle: {}, Signatur und SHA-256 geprüft)",
                    got.source
                ))
            })();
            let (lvl, text) = match res {
                Ok(t) => (Level::Info, t),
                Err(e) => (Level::Warn, format!("GPU-Modul: {e}")),
            };
            msgs.lock().unwrap().push((lvl, text));
            *status.lock().unwrap() = None;
        });
    }

    fn gpu_results(ui: &mut egui::Ui, g: &ramforge_gpu::GpuResult) {
        heading(ui, "GPU");
        if let Some(why) = &g.skipped {
            ui.label(RichText::new(why).color(WARN));
            return;
        }
        ui.label(
            RichText::new(format!(
                "{} · {} · {} · Treiber {}",
                g.adapter, g.device_type, g.backend, g.driver
            ))
            .small(),
        );
        ui.horizontal(|ui| {
            for (name, v) in [
                ("VRAM", format!("{:.0} GB/s", g.vram_gbs)),
                ("FP32 (FMA)", format!("{:.2} TFLOPS", g.fp32_tflops)),
                ("zur GPU", format!("{:.1} GB/s", g.upload_gbs)),
                ("zum PC", format!("{:.1} GB/s", g.download_gbs)),
            ] {
                egui::Frame::group(ui.style())
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(name).weak());
                            ui.label(RichText::new(v).size(22.0).strong());
                        });
                    });
            }
            egui::Frame::group(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Prüfung").weak());
                        ui.label(
                            RichText::new(if g.errors > 0 {
                                format!("{} Fehler", g.errors)
                            } else {
                                "ok".into()
                            })
                            .size(22.0)
                            .strong()
                            .color(if g.errors > 0 {
                                BAD
                            } else {
                                OK
                            }),
                        );
                    });
                });
        });
        ui.label(RichText::new("Eigene Arbeitslast über wgpu: VRAM zählt gelesene + geschriebene Bytes einer Kopie; FP32 misst 8 unabhängige FMA-Ketten je Thread und liegt unter dem Datenblatt-Spitzenwert; Übertragungen enthalten die Zwischenpuffer des Treibers. Kopierte Daten und Rechenergebnisse werden geprüft.").small().weak());
    }

    fn bench_history_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(RichText::new(format!("Verlauf ({} Läufe)", self.bench_history.len())).strong())
            .id_salt("bench-history")
            .show(ui, |ui| {
                ui.label(RichText::new("Vergleichbar sind nur Läufe mit gleicher Version und gleichen Einstellungen (≈ in der letzten Spalte). Die Notiz ist frei, z. B. „XMP an“.").small().weak());
                let last = self.bench_run.clone();
                let mut changed = false;
                egui::Grid::new("hist").num_columns(9).striped(true).show(ui, |ui| {
                    for h in ["Datum", "CPU 1T / nT", "RAM Lesen", "Latenz", "Last Ø", "GPU VRAM / FP32", "Fehler", "Notiz", ""] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for r in self.bench_history.iter_mut().rev() {
                        ui.label(r.time.format("%d.%m. %H:%M").to_string()).on_hover_text(r.settings_text());
                        let (st, mt) = r.cpu_points();
                        ui.monospace(if st > 0 { format!("{st} / {mt}") } else { "–".into() });
                        ui.monospace(r.ram.as_ref().map(|x| format!("{:.1} GB/s", x.read_gbs)).unwrap_or("–".into()));
                        ui.monospace(r.ram.as_ref().and_then(|x| x.dram_latency_ns()).map(|v| format!("{v:.0} ns")).unwrap_or("–".into()));
                        ui.monospace(r.stress.as_ref().map(|x| format!("{:.1} GB/s", x.avg_gbs)).unwrap_or("–".into()));
                        ui.monospace(
                            r.gpu
                                .as_ref()
                                .filter(|g| g.skipped.is_none())
                                .map(|g| format!("{:.0} GB/s / {:.1} TF", g.vram_gbs, g.fp32_tflops))
                                .unwrap_or("–".into()),
                        );
                        let e = r.errors();
                        ui.label(RichText::new(e.to_string()).color(if e > 0 { BAD } else { OK }));
                        if ui.add(egui::TextEdit::singleline(&mut r.note).desired_width(140.0)).changed() {
                            changed = true;
                        }
                        let cmp = last.as_ref().map(|l| l.comparable(r)).unwrap_or(false);
                        ui.label(if cmp { "≈" } else { "" }).on_hover_text("vergleichbar mit dem letzten Lauf");
                        ui.end_row();
                    }
                });
                if changed {
                    crate::benchsuite::save_history(&self.paths.root.join("bench-history.json"), &self.bench_history);
                }
            });
    }

    fn tab_sensors(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Sensoren");
        ui.horizontal(|ui| {
            ui.label("Intervall");
            let mut ms = self.settings.sensor_interval_ms as f64;
            if ui
                .add(
                    egui::Slider::new(&mut ms, 250.0..=5000.0)
                        .suffix(" ms")
                        .logarithmic(true),
                )
                .changed()
            {
                self.settings.sensor_interval_ms = ms as u64;
                self.hub.set_interval(Duration::from_millis(ms as u64));
                self.save_settings();
            }
            ui.separator();
            match self.hub.recording_path() {
                Some(_) => {
                    if ui.button("⏹ Aufzeichnung beenden").clicked() {
                        if let Some((p, rows)) = self.hub.stop_recording() {
                            self.log.info(format!(
                                "Sensor-Aufzeichnung beendet: {rows} Zeilen in {}",
                                p.display()
                            ));
                        }
                    }
                }
                None => {
                    if ui
                        .button("⏺ Aufzeichnen (CSV)")
                        .on_hover_text("Während eines RAM-Tests wird automatisch aufgezeichnet")
                        .clicked()
                    {
                        let p = self
                            .paths
                            .sensors
                            .join(format!("sensors-{}.csv", paths::stamp()));
                        match self.hub.start_recording(&p) {
                            Ok(()) => self
                                .log
                                .info(format!("Sensor-Aufzeichnung: {}", p.display())),
                            Err(e) => self.log.error(format!("Aufzeichnung fehlgeschlagen: {e}")),
                        }
                    }
                }
            }
            if ui.button("Min/Max zurücksetzen").clicked() {
                self.hub.reset_stats();
            }
            if ui.button("📂 Ordner").clicked() {
                paths::open_folder(&self.paths.sensors);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.text_edit_singleline(&mut self.sensor_filter);
            ui.checkbox(&mut self.show_threads, "Werte je Thread zeigen");
        });
        egui::CollapsingHeader::new("Quellen und Hinweise").default_open(false).show(ui, |ui| {
            for (n, s) in self.hub.source_status() {
                ui.label(format!("{n}: {s}"));
            }
            if cfg!(windows) {
            ui.label(RichText::new(
                "Windows liefert ohne Treiber nur Auslastung, Takt, RAM-Belegung und ACPI-Temperaturen. \
                 Für CPU-, DIMM- und Mainboard-Temperaturen, Spannungen, Lüfter, WHEA-Fehler und aktuelle Timings: \
                 HWiNFO64 starten → Einstellungen → „Shared Memory Support“ aktivieren. RAMForge liest die Werte dann automatisch mit. \
                 Ein eigener Treiber (PawnIO) ist geplant. Die Boot-Variante liest alles direkt über Linux-Treiber.",
            ).small());
            } else {
                ui.label(RichText::new(
                    "Hier im Boot-System kommen Temperaturen, Spannungen und Lüfter von den Linux-hwmon-Treibern, \
                     SPD-Daten über i2c (ee1004 für DDR4, spd5118 für DDR5). In virtuellen Maschinen fehlen diese Sensoren meist.",
                ).small());
            }
        });

        let snap = self.hub.snapshot_light();
        if !self.plot_ids.is_empty() {
            let series: Vec<(String, Vec<[f64; 2]>)> = self
                .plot_ids
                .iter()
                .filter_map(|id| snap.iter().find(|s| &s.reading.id == id))
                .map(|s| {
                    (
                        format!("{} [{}]", s.reading.label, s.reading.unit),
                        self.hub.history(&s.reading.id),
                    )
                })
                .collect();
            let now = self.hub.now_secs();
            egui_plot::Plot::new("sensor_plot")
                .height(190.0)
                .legend(egui_plot::Legend::default())
                .x_axis_label("Sekunden")
                .allow_scroll(false)
                .show(ui, move |p| {
                    for (name, pts) in series {
                        let pts: Vec<[f64; 2]> =
                            pts.into_iter().map(|[t, v]| [t - now, v]).collect();
                        p.line(egui_plot::Line::new(name, egui_plot::PlotPoints::from(pts)));
                    }
                });
            if ui.small_button("Diagramm leeren").clicked() {
                self.plot_ids.clear();
            }
        } else {
            ui.label(RichText::new("Auf einen Sensornamen klicken, um seinen Verlauf als Diagramm zu sehen (bis zu 6).").small().weak());
        }

        let filter = self.sensor_filter.to_lowercase();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for g in SensorGroup::ALL {
                    let rows: Vec<&SensorStat> = snap
                        .iter()
                        .filter(|s| s.reading.group == g)
                        .filter(|s| self.show_threads || !s.reading.label.starts_with("Thread "))
                        .filter(|s| {
                            filter.is_empty()
                                || s.reading.label.to_lowercase().contains(&filter)
                                || s.reading.device.to_lowercase().contains(&filter)
                        })
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    egui::CollapsingHeader::new(
                        RichText::new(format!("{}  ({})", g.name(), rows.len())).strong(),
                    )
                    .id_salt(g)
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new(("sg", g))
                            .num_columns(7)
                            .striped(true)
                            .min_col_width(40.0)
                            .show(ui, |ui| {
                                for h in ["", "Gerät", "Sensor", "Aktuell", "Min", "Max", "Ø"] {
                                    ui.label(RichText::new(h).weak());
                                }
                                ui.end_row();
                                let mut last_dev = String::new();
                                for s in rows {
                                    let id = &s.reading.id;
                                    let pinned = self.settings.pinned_sensors.contains(id);
                                    if ui
                                        .selectable_label(pinned, "📌")
                                        .on_hover_text("In der Seitenleiste anzeigen")
                                        .clicked()
                                    {
                                        if pinned {
                                            self.settings.pinned_sensors.retain(|p| p != id);
                                        } else {
                                            self.settings.pinned_sensors.push(id.clone());
                                        }
                                        self.save_settings();
                                    }
                                    if s.reading.device != last_dev {
                                        ui.label(RichText::new(&s.reading.device).small());
                                        last_dev = s.reading.device.clone();
                                    } else {
                                        ui.label("");
                                    }
                                    let plotted = self.plot_ids.contains(id);
                                    if ui.selectable_label(plotted, &s.reading.label).clicked() {
                                        if plotted {
                                            self.plot_ids.retain(|p| p != id);
                                        } else if self.plot_ids.len() < 6 {
                                            self.plot_ids.push(id.clone());
                                        }
                                    }
                                    let u = &s.reading.unit;
                                    let mut cur =
                                        RichText::new(fmt_val(s.reading.value, u)).monospace();
                                    if let Some(c) = temp_color(s) {
                                        cur = cur.color(c).strong();
                                    }
                                    ui.label(cur);
                                    ui.monospace(fmt_val(s.min, u));
                                    ui.monospace(fmt_val(s.max, u));
                                    ui.monospace(fmt_val(s.avg(), u));
                                    ui.end_row();
                                }
                            });
                    });
                }
            });
    }

    fn tab_spd(&mut self, ui: &mut egui::Ui) {
        heading(ui, "SPD · Timings · XMP/EXPO");
        #[cfg(windows)]
        ui.label(RichText::new(
            "Unter Windows braucht das Auslesen des SPD-Chips einen Kerneltreiber (SMBus), den diese Version noch nicht mitbringt. \
             Die Boot-Variante liest den SPD direkt aus. Hier können Sie einen SPD-Dump (.bin, 256 Byte DDR3, 512 Byte DDR4, 1024 Byte DDR5) laden, \
             z. B. von der Boot-Variante oder aus einem anderen Tool. Die aktuell laufenden Timings zeigt HWiNFO im Tab Sensoren („Memory Timings“).",
        ).small());
        // The boot edition reads SPD directly and has no file dialog.
        if !boot_edition() && ui.button("📂 SPD-Dump laden…").clicked() {
            if let Some(files) = rfd::FileDialog::new()
                .add_filter("SPD-Dump", &["bin", "spd", "dmp", "rom"])
                .pick_files()
            {
                for f in files {
                    let name = f
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let res = std::fs::read(&f)
                        .map_err(|e| e.to_string())
                        .and_then(|b| spd::decode(&b));
                    match &res {
                        Ok(i) => self.log.info(format!(
                            "SPD-Dump {name}: {} {} {}",
                            i.dram_type,
                            report::gib(i.capacity_bytes),
                            i.part_number
                        )),
                        Err(e) => self.log.warn(format!("SPD-Dump {name}: {e}")),
                    }
                    self.spd_files.push((name, res));
                }
            }
        }
        let mut entries: Vec<(String, Result<SpdInfo, String>)> = self
            .sys
            .spd
            .iter()
            .map(|(n, i)| (format!("Modul {n}"), Ok(i.clone())))
            .collect();
        entries.extend(self.spd_files.iter().cloned());
        for (name, res) in entries {
            ui.separator();
            ui.label(RichText::new(&name).strong().color(ACCENT));
            match res {
                Err(e) => {
                    ui.label(RichText::new(e).color(BAD));
                }
                Ok(i) => {
                    egui::Grid::new(("spd", name.clone()))
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            let mut row = |k: &str, v: String| {
                                ui.label(RichText::new(k).weak());
                                ui.label(v);
                                ui.end_row();
                            };
                            row("Typ", format!("{} {}", i.dram_type, i.module_type));
                            row(
                                "Kapazität",
                                format!(
                                    "{} · {} Rank(s) · x{} Chips · {} Bit{}",
                                    report::gib(i.capacity_bytes),
                                    i.ranks,
                                    i.device_width,
                                    i.bus_width,
                                    if i.ecc { " + ECC" } else { "" }
                                ),
                            );
                            row(
                                "Hersteller",
                                format!("{} (DRAM: {})", i.manufacturer, i.dram_manufacturer),
                            );
                            row("Teilenummer", i.part_number.clone());
                            row("Herstellung", i.manufacture_date.clone());
                            row(
                                "XMP / EXPO",
                                format!(
                                    "{} / {}",
                                    if i.xmp_present { "vorhanden" } else { "nein" },
                                    if i.expo_present { "vorhanden" } else { "nein" }
                                ),
                            );
                        });
                    egui::Grid::new(("spdt", name.clone()))
                        .num_columns(8)
                        .striped(true)
                        .show(ui, |ui| {
                            for h in [
                                "Profil",
                                "MT/s",
                                "CL",
                                "tRCD",
                                "tRP",
                                "tRAS",
                                "tRC",
                                "tRFC1 / V",
                            ] {
                                ui.label(RichText::new(h).strong());
                            }
                            ui.end_row();
                            for t in std::iter::once(&i.jedec).chain(i.profiles.iter()) {
                                ui.label(&t.name);
                                ui.label(t.speed_mts.to_string());
                                for v in [t.cl, t.trcd, t.trp, t.tras, t.trc] {
                                    ui.monospace(v.to_string());
                                }
                                ui.label(format!(
                                    "{:.0} ns / {}",
                                    t.trfc1_ns,
                                    t.voltage_v.map(|v| format!("{v:.2} V")).unwrap_or_default()
                                ));
                                ui.end_row();
                            }
                        });
                    for n in &i.notes {
                        ui.label(RichText::new(n).small().color(WARN));
                    }
                }
            }
        }
    }

    fn tab_log(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            heading(ui, "Protokoll");
            if ui.button("📂 Logs").clicked() {
                paths::open_folder(&self.paths.logs);
            }
            if ui.button("📂 Berichte").clicked() {
                paths::open_folder(&self.paths.reports);
            }
            if ui.button("📂 Screenshots").clicked() {
                paths::open_folder(&self.paths.screenshots);
            }
            if ui.button("📂 Datenordner").clicked() {
                paths::open_folder(&self.paths.root);
            }
        });
        ui.label(
            RichText::new(format!("Logdatei: {}", self.log.path.display()))
                .small()
                .weak(),
        );
        if !self.sessions.is_empty() {
            egui::CollapsingHeader::new(
                RichText::new(format!("Sensorverläufe ({})", self.sessions.len())).strong(),
            )
            .id_salt("log-sessions")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, s) in self.sessions.iter().enumerate() {
                        let name = format!(
                            "{} {} ({})",
                            if s.label == "bench" {
                                "Benchmark"
                            } else {
                                "RAM-Test"
                            },
                            s.started.format("%H:%M"),
                            crate::chart::fmt_secs(s.duration_secs)
                        );
                        if ui.selectable_label(self.log_session == i, name).clicked() {
                            self.log_session = i;
                        }
                    }
                });
                let i = self.log_session.min(self.sessions.len() - 1);
                crate::chart::show(ui, &self.sessions[i], &mut self.chart_log, "chart-log");
            });
            ui.separator();
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for e in &self.log.entries {
                    let c = match e.level {
                        Level::Info => ui.visuals().text_color(),
                        Level::Warn => WARN,
                        Level::Error => BAD,
                    };
                    ui.horizontal(|ui| {
                        ui.monospace(RichText::new(e.time.format("%H:%M:%S").to_string()).weak());
                        ui.label(RichText::new(&e.text).color(c));
                    });
                }
            });
    }

    fn tab_about(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("RAM").size(30.0).strong().color(ACCENT));
            ui.add_space(-8.0);
            ui.label(RichText::new("Forge").size(30.0).strong());
            ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).weak());
        });
        ui.label("Portabler RAM-Test, Benchmark und Hardware-Monitor. Freie Software unter der GPL-3.0-or-later.");
        ui.label(RichText::new("© 2026 Daniel Airich und RAMForge-Mitwirkende · Idee, Projektleitung und Code: Daniel Airich").strong());
        ui.add_space(6.0);
        egui::Grid::new("about").num_columns(2).show(ui, |ui| {
            for (k, url) in [
                ("Website", URL_SITE),
                ("Haftungsausschluss", URL_DISCLAIMER),
                ("Datenschutz", URL_PRIVACY),
                ("Quellcode", URL_SOURCE),
            ] {
                ui.label(RichText::new(k).weak());
                ui.hyperlink_to(url, url);
                ui.end_row();
            }
            ui.label(RichText::new("Datenordner").weak());
            if ui.link(self.paths.root.display().to_string()).clicked() {
                paths::open_folder(&self.paths.root);
            }
            ui.end_row();
        });
        ui.add_space(6.0);
        egui::Frame::group(ui.style()).inner_margin(10.0).show(ui, |ui| {
            ui.label(RichText::new("Updates, Neuigkeiten und Statistik").strong());
            ui.horizontal(|ui| {
                if ui.checkbox(&mut self.settings.check_updates, "Beim Start nach Updates und Neuigkeiten suchen").changed() {
                    self.save_settings();
                }
                let can = ramforge_update::key::is_configured() && !self.updater.is_checking();
                if ui.add_enabled(can, egui::Button::new("Jetzt suchen")).clicked() {
                    self.updater.check_in_background("");
                }
            });
            ui.label(RichText::new(format!(
                "Fragt {} ab. Updates werden nur installiert, wenn die Ed25519-Signatur und die SHA-256-Prüfsumme stimmen. Übertragen werden nur Programmversion und Sprache.",
                ramforge_update::MANIFEST_URL
            )).small().weak());
            ui.label(RichText::new("Fingerabdrücke der Update-Schlüssel (SHA-256, müssen mit ramforge.org übereinstimmen):").small());
            egui::Grid::new("fingerprints").num_columns(2).show(ui, |ui| {
                for (name, fp) in ramforge_update::key::fingerprints() {
                    ui.label(RichText::new(name).small().strong());
                    ui.monospace(RichText::new(fp).small());
                    ui.end_row();
                }
            });
            if ramforge_update::key::is_placeholder() {
                ui.label(RichText::new("Entwicklungsversion: Test- bzw. Platzhalterschlüssel, keine offizielle Release-Version.").small().color(WARN));
            }
            let mut stats = self.settings.stats_consent;
            if ui.checkbox(&mut stats, "Anonyme Statistik senden").changed() {
                self.set_stats_consent(stats);
            }
            ui.label(
                RichText::new(
                    "Freiwillig, standardmäßig aus. Höchstens einmal am Tag: zufällige Installations-ID, Version, Edition, grobe Windows-Version. \
                     Ausschalten sendet eine Löschanfrage an den Server und löscht die lokale ID.",
                )
                .small()
                .weak(),
            );
        });
        let news = self.news.lock().unwrap().clone();
        if let Some(items) = news {
            heading(ui, "Neuigkeiten");
            if items.is_empty() {
                ui.label(RichText::new("Keine Neuigkeiten.").weak());
            }
            for n in &items {
                egui::Frame::group(ui.style())
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let c = match n.level.as_str() {
                                "critical" => BAD,
                                "warning" => WARN,
                                _ => ui.visuals().text_color(),
                            };
                            ui.label(RichText::new(&n.title).strong().color(c));
                            if n.id > self.news_seen_before {
                                ui.label(RichText::new("neu").small().color(ACCENT));
                            }
                            ui.label(
                                RichText::new(n.created.get(..10).unwrap_or(&n.created))
                                    .small()
                                    .weak(),
                            );
                        });
                        // Plain text only, never interpreted as HTML or markup.
                        ui.label(&n.body);
                        if !n.url.is_empty() {
                            ui.hyperlink_to("Mehr dazu", &n.url);
                        }
                    });
            }
            self.mark_news_read();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(SPONSOR_NOTE).small().weak());
            ui.hyperlink_to(
                RichText::new("nerdscave-consulting.com").small(),
                URL_SPONSOR,
            );
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .checkbox(
                    &mut self.settings.anonymize_reports,
                    "Rechnername und Seriennummern in Berichten ausblenden",
                )
                .changed()
            {
                self.save_settings();
            }
        });
        md_view(ui, DISCLAIMER_MD);
        heading(ui, "Danksagung");
        ui.label(RichText::new(
            "Testverfahren nach öffentlicher Fachliteratur (Moving Inversions, March-Tests, Modulo-X, Bit Fade, Row Hammer). \
             Oberfläche mit egui/eframe. Optionaler Booteintrag: Memtest86+ (GPL-2.0). Sensorwerte optional über die Shared-Memory-Schnittstelle von HWiNFO.",
        ).small());
    }
}
