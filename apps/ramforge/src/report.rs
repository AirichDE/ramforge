//! Report export (JSON for machines, HTML and TXT for people).

use ramforge_core::bench::BenchResult;
use ramforge_core::{MemError, RunSummary};
use ramforge_hw::sensors::{EventKind, SensorEvent, SensorStat, SessionData};
use ramforge_hw::SystemInfo;
#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct SensorRow {
    pub group: String,
    pub device: String,
    pub label: String,
    pub unit: String,
    pub current: f64,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
}

#[derive(Serialize)]
pub struct Report {
    pub app: String,
    pub version: String,
    pub generated: chrono::DateTime<chrono::Local>,
    pub anonymized: bool,
    pub system: SystemInfo,
    pub test: Option<RunSummary>,
    pub errors: Vec<MemError>,
    pub bit_histogram: Vec<u64>,
    pub benchmark: Option<BenchResult>,
    /// CPU and CPU+RAM load results with version and settings.
    pub bench_suite: Option<crate::benchsuite::BenchRun>,
    pub sensors: Vec<SensorRow>,
    pub events: Vec<SensorEvent>,
    /// Sensor recordings of the last RAM test and benchmark.
    pub sensor_sessions: Vec<SessionReport>,
}

#[derive(Serialize)]
pub struct SessionSeriesReport {
    pub id: String,
    pub group: String,
    pub device: String,
    pub label: String,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub samples: u64,
    /// Shown in the HTML chart by default.
    pub charted: bool,
    /// (seconds since start, value), downsampled with peaks kept.
    pub points: Vec<[f64; 2]>,
}

#[derive(Serialize)]
pub struct SessionReport {
    /// "test" or "bench"
    pub kind: String,
    pub started: chrono::DateTime<chrono::Local>,
    pub duration_secs: f64,
    pub csv: Option<String>,
    pub events: Vec<SensorEvent>,
    pub series: Vec<SessionSeriesReport>,
}

/// Points per series in the JSON/HTML report (the CSV has every sample).
const REPORT_POINTS: usize = 800;

fn session_report(d: &SessionData) -> SessionReport {
    let charted = crate::chart::default_selection(d);
    SessionReport {
        kind: d.label.clone(),
        started: d.started,
        duration_secs: d.duration_secs,
        csv: d.csv.as_ref().map(|p| p.display().to_string()),
        events: d.events.clone(),
        series: d
            .series
            .iter()
            .filter(|s| !s.label.starts_with("Thread "))
            .map(|s| SessionSeriesReport {
                id: s.id.clone(),
                group: s.group.name().into(),
                device: s.device.clone(),
                label: s.label.clone(),
                unit: s.unit.clone(),
                min: s.min,
                max: s.max,
                avg: s.avg,
                samples: s.samples,
                charted: charted.contains(&s.id),
                points: s.reduced(REPORT_POINTS),
            })
            .collect(),
    }
}

pub struct ReportInput<'a> {
    pub system: &'a SystemInfo,
    pub test: Option<&'a RunSummary>,
    pub errors: &'a [MemError],
    pub bit_histogram: &'a [u64; 64],
    pub benchmark: Option<&'a BenchResult>,
    pub bench_run: Option<&'a crate::benchsuite::BenchRun>,
    pub sensors: &'a [SensorStat],
    pub events: Vec<SensorEvent>,
    pub sessions: Vec<&'a SessionData>,
    pub anonymize: bool,
}

pub fn build(i: ReportInput) -> Report {
    let mut system = i.system.clone();
    if i.anonymize {
        system.hostname = t!("(ausgeblendet)", "(hidden)").into();
        if let Some(s) = system.smbios.as_mut() {
            for m in &mut s.memory {
                if !m.serial.is_empty() {
                    m.serial = t!("(ausgeblendet)", "(hidden)").into();
                }
            }
        }
        for (_, s) in &mut system.spd {
            s.serial = t!("(ausgeblendet)", "(hidden)").into();
        }
    }
    Report {
        app: "RAMForge".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        generated: chrono::Local::now(),
        anonymized: i.anonymize,
        system,
        test: i.test.cloned(),
        errors: i.errors.iter().take(1000).cloned().collect(),
        bit_histogram: i.bit_histogram.to_vec(),
        benchmark: i.benchmark.cloned(),
        bench_suite: i.bench_run.cloned(),
        sensors: i
            .sensors
            .iter()
            // Per-thread load values make the report long without adding much.
            .filter(|s| !s.reading.label.starts_with("Thread "))
            .map(|s| SensorRow {
                group: s.reading.group.name().into(),
                device: s.reading.device.clone(),
                label: s.reading.label.clone(),
                unit: s.reading.unit.clone(),
                current: s.reading.value,
                min: s.min,
                max: s.max,
                avg: s.avg(),
            })
            .collect(),
        events: i.events,
        sensor_sessions: i.sessions.iter().map(|d| session_report(d)).collect(),
    }
}

fn session_title(r: &SessionReport) -> &'static str {
    if r.kind == "bench" {
        t!("Sensorverlauf Benchmark", "Sensor history benchmark")
    } else {
        t!("Sensorverlauf RAM-Test", "Sensor history RAM test")
    }
}

fn fmt_num(v: f64, unit: &str) -> String {
    crate::app::fmt_val(v, unit)
}

/// One self-contained SVG line chart (no external files, no scripts).
pub fn svg_chart(
    title: &str,
    series: &[(String, &SessionSeriesReport)],
    events: &[SensorEvent],
    duration: f64,
) -> String {
    const W: f64 = 960.0;
    const H: f64 = 230.0;
    const L: f64 = 64.0; // left margin (y labels)
    const R: f64 = 12.0;
    const T: f64 = 22.0;
    const B: f64 = 28.0;
    let pw = W - L - R;
    let ph = H - T - B;
    let dur = duration.max(1.0);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (_, s) in series {
        for p in &s.points {
            lo = lo.min(p[1]);
            hi = hi.max(p[1]);
        }
    }
    if !lo.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    if hi - lo < 1e-9 {
        hi += 1.0;
        lo -= 1.0;
    }
    let pad = (hi - lo) * 0.06;
    let (lo, hi) = (lo - pad, hi + pad);
    let x = |t: f64| L + (t / dur).clamp(0.0, 1.0) * pw;
    let y = |v: f64| T + (1.0 - (v - lo) / (hi - lo)) * ph;
    let unit = series.first().map(|s| s.1.unit.as_str()).unwrap_or("");
    let mut o = String::new();
    let _ = write!(
        o,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" role="img" aria-label="{t}" class="chart"><title>{t}</title>"##,
        t = esc(title)
    );
    let _ = write!(
        o,
        r##"<text x="{L}" y="15" class="ct">{}</text>"##,
        esc(title)
    );
    for k in 0..=4 {
        let v = lo + (hi - lo) * k as f64 / 4.0;
        let yy = y(v);
        let _ = write!(
            o,
            r##"<line x1="{L}" x2="{:.1}" y1="{yy:.1}" y2="{yy:.1}" class="grid"/><text x="{:.1}" y="{:.1}" class="ax" text-anchor="end">{}</text>"##,
            L + pw,
            L - 6.0,
            yy + 4.0,
            esc(&fmt_num(v, unit))
        );
    }
    // Round tick steps, so short runs never show the same label twice.
    let step = crate::chart::nice_step(dur / 6.0);
    let mut t = 0.0;
    while t <= dur + 1e-9 {
        let _ = write!(
            o,
            r##"<text x="{:.1}" y="{:.1}" class="ax" text-anchor="middle">{}</text>"##,
            x(t),
            H - 8.0,
            crate::chart::fmt_tick(t, step)
        );
        t += step;
    }
    for e in events {
        let (cls, w) = match e.kind {
            EventKind::Error => ("err", 1.4),
            EventKind::Phase => ("phase", 1.0),
            EventKind::Info => ("info", 1.0),
        };
        let xx = x(e.t);
        let _ = write!(
            o,
            r##"<line x1="{xx:.1}" x2="{xx:.1}" y1="{T}" y2="{:.1}" class="{cls}" stroke-width="{w}"><title>{} – {}</title></line>"##,
            T + ph,
            crate::chart::fmt_secs(e.t),
            esc(&e.text)
        );
    }
    for (color, s) in series {
        let mut pts = String::with_capacity(s.points.len() * 14);
        for p in &s.points {
            let _ = write!(pts, "{:.1},{:.1} ", x(p[0]), y(p[1]));
        }
        let _ = write!(
            o,
            r##"<polyline fill="none" stroke="{color}" stroke-width="1.6" points="{}"><title>{}</title></polyline>"##,
            pts.trim_end(),
            esc(&format!("{} · {}", s.label, s.device))
        );
    }
    o.push_str("</svg>");
    o
}

/// HTML block for one recording: charts per unit, legend, table, events.
fn session_html(r: &SessionReport) -> String {
    let mut o = String::new();
    let _ = write!(
        o,
        "{}",
        tf!(
            "<section><h2>{}</h2><p class=\"sub\">Beginn {} · Dauer {} · {} Sensoren{}</p>",
            "<section><h2>{}</h2><p class=\"sub\">Start {} · duration {} · {} sensors{}</p>",
            session_title(r),
            r.started.format("%d.%m.%Y %H:%M:%S"),
            crate::chart::fmt_secs(r.duration_secs),
            r.series.len(),
            r.csv
                .as_ref()
                .map(|c| format!(" · CSV: <code>{}</code>", esc(c)))
                .unwrap_or_default()
        )
    );
    // Same colours as in the program.
    let charted: Vec<(String, &SessionSeriesReport)> = r
        .series
        .iter()
        .filter(|s| s.charted)
        .enumerate()
        .map(|(i, s)| {
            (
                crate::chart::color_hex(crate::chart::PALETTE[i % crate::chart::PALETTE.len()]),
                s,
            )
        })
        .collect();
    let mut units: Vec<&str> = Vec::new();
    for (_, s) in &charted {
        if !units.contains(&s.unit.as_str()) {
            units.push(&s.unit);
        }
    }
    for u in units {
        let items: Vec<(String, &SessionSeriesReport)> = charted
            .iter()
            .filter(|(_, s)| s.unit == u)
            .map(|(c, s)| (c.clone(), *s))
            .collect();
        let title = match u {
            "°C" => t!("Temperatur (°C)", "Temperature (°C)").to_string(),
            "V" => t!("Spannung (V)", "Voltage (V)").to_string(),
            "%" => t!("Auslastung (%)", "Load (%)").to_string(),
            "MHz" => t!("Takt (MHz)", "Clock (MHz)").to_string(),
            other => other.to_string(),
        };
        o.push_str(&svg_chart(&title, &items, &r.events, r.duration_secs));
        o.push_str("<p class=\"legend\">");
        for (c, s) in &items {
            let _ = write!(
                o,
                "<span><i style=\"background:{c}\"></i>{} · {}</span>",
                esc(&s.label),
                esc(&s.device)
            );
        }
        o.push_str("</p>");
    }
    if charted.is_empty() {
        o.push_str(t!(
            "<p class=\"sub\">Keine Diagramm-Sensoren aufgezeichnet.</p>",
            "<p class=\"sub\">No chart sensors recorded.</p>"
        ));
    }
    o.push_str(t!("<p class=\"legend\"><span><i class=\"mk-phase\"></i>Testphase</span><span><i class=\"mk-err\"></i>RAM-Fehler</span></p>", "<p class=\"legend\"><span><i class=\"mk-phase\"></i>Test phase</span><span><i class=\"mk-err\"></i>RAM error</span></p>"));
    o.push_str(t!("<table><tr><th>Gruppe</th><th>Gerät</th><th>Sensor</th><th>Min</th><th>Max</th><th>Ø</th><th>Werte</th></tr>", "<table><tr><th>Group</th><th>Device</th><th>Sensor</th><th>Min</th><th>Max</th><th>Avg</th><th>Values</th></tr>"));
    for s in &r.series {
        let _ = write!(
            o,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td class=n>{}</td><td class=n>{}</td><td class=n>{}</td><td class=n>{}</td></tr>",
            esc(&s.group), esc(&s.device), esc(&s.label),
            esc(&fmt_num(s.min, &s.unit)), esc(&fmt_num(s.max, &s.unit)), esc(&fmt_num(s.avg, &s.unit)), s.samples
        );
    }
    o.push_str("</table>");
    if !r.events.is_empty() {
        o.push_str(t!(
            "<h2 style=\"margin-top:16px\">Ereignisse</h2><table>",
            "<h2 style=\"margin-top:16px\">Events</h2><table>"
        ));
        for e in r.events.iter().take(300) {
            let cls = if e.kind == EventKind::Error {
                " class=\"bad-t\""
            } else {
                ""
            };
            let _ = write!(
                o,
                "<tr{cls}><td>{}</td><td>{}</td></tr>",
                crate::chart::fmt_secs(e.t),
                esc(&e.text)
            );
        }
        o.push_str("</table>");
    }
    o.push_str("</section>");
    o
}

pub fn gib(b: u64) -> String {
    format!("{:.1} GiB", b as f64 / (1u64 << 30) as f64)
}

pub fn duration(secs: f64) -> String {
    let s = secs as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

pub fn outcome_text(o: ramforge_core::Outcome) -> &'static str {
    match o {
        ramforge_core::Outcome::Passed => "BESTANDEN",
        ramforge_core::Outcome::Failed => t!("FEHLER GEFUNDEN", "ERRORS FOUND"),
        ramforge_core::Outcome::Cancelled => "ABGEBROCHEN",
        ramforge_core::Outcome::Aborted => t!("NICHT GESTARTET", "NOT STARTED"),
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Key/value lines shared by TXT and HTML.
fn system_lines(r: &Report) -> Vec<(String, String)> {
    let s = &r.system;
    let mut v = vec![
        (
            t!("Betriebssystem", "Operating system").into(),
            s.os.clone(),
        ),
        (
            t!("Rechnername", "Computer name").into(),
            s.hostname.clone(),
        ),
        (
            "CPU".into(),
            tf!(
                "{} ({} Threads)",
                "{} ({} threads)",
                s.cpu.brand,
                s.cpu.logical_processors
            ),
        ),
        (
            t!("Arbeitsspeicher", "Memory").into(),
            gib(s.memory.total_bytes),
        ),
    ];
    if let Some(b) = &s.smbios {
        v.push((
            "Mainboard".into(),
            format!(
                "{} {} {}",
                b.board_manufacturer, b.board_product, b.board_version
            ),
        ));
        v.push((
            "BIOS".into(),
            format!("{} {} ({})", b.bios_vendor, b.bios_version, b.bios_date),
        ));
        let used = b.memory.iter().filter(|m| m.is_installed()).count();
        v.push((
            t!("Speicherplätze", "Memory slots").into(),
            tf!(
                "{used} von {} belegt",
                "{used} of {} in use",
                b.memory_slots_total.max(b.memory.len() as u32)
            ),
        ));
    }
    v
}

fn module_rows(r: &Report) -> Vec<[String; 8]> {
    let Some(b) = &r.system.smbios else {
        return vec![];
    };
    b.memory
        .iter()
        .filter(|m| m.is_installed())
        .map(|m| {
            [
                format!("{} {}", m.bank, m.locator).trim().to_string(),
                gib(m.size_bytes),
                format!("{} {}", m.memory_type, m.form_factor),
                format!("{} / {} MT/s", m.speed_mts, m.configured_speed_mts),
                if m.configured_voltage_mv > 0 {
                    format!("{:.3} V", m.configured_voltage_mv as f64 / 1000.0)
                } else {
                    "–".into()
                },
                m.manufacturer.clone(),
                m.part_number.clone(),
                m.serial.clone(),
            ]
        })
        .collect()
}

pub fn to_txt(r: &Report) -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        "{}",
        tf!(
            "RAMForge {} – Bericht vom {}",
            "RAMForge {} – report of {}",
            r.version,
            r.generated.format("%d.%m.%Y %H:%M:%S")
        )
    );
    let _ = writeln!(o, "{}", "=".repeat(72));
    for (k, v) in system_lines(r) {
        let _ = writeln!(o, "{k:<18} {v}");
    }
    let _ = writeln!(o, "{}", tf!("\nModule (Steckplatz | Größe | Typ | Nenn-/Ist-Takt | Spannung | Hersteller | Teil | Serie)", "\nModules (slot | size | type | rated/actual speed | voltage | manufacturer | part | serial)"));
    for m in module_rows(r) {
        let _ = writeln!(o, "  {}", m.join(" | "));
    }
    if let Some(t) = &r.test {
        let _ = writeln!(o, "\nRAM-Test: {} – {}", outcome_text(t.outcome), t.message);
        let _ = writeln!(
            o,
            "{}",
            tf!(
                "  Beginn {}  Dauer {}",
                "  Start {}  Duration {}",
                t.started.format("%d.%m.%Y %H:%M:%S"),
                duration(t.duration_secs)
            )
        );
        let _ = writeln!(
            o,
            "{}",
            tf!(
                "  Getestet {}  Threads {}  Durchläufe {}  Tests {}  Speicher gesperrt: {}",
                "  Tested {}  Threads {}  Passes {}  Tests {}  Memory locked: {}",
                gib(t.tested_bytes),
                t.threads,
                t.passes_completed,
                t.tests_completed,
                if t.memory_locked {
                    t!("ja", "yes")
                } else {
                    t!("nein", "no")
                }
            )
        );
        let _ = writeln!(
            o,
            "{}",
            tf!("  Fehler gesamt: {}", "  Errors in total: {}", t.errors)
        );
        for e in r.errors.iter().take(50) {
            let _ = writeln!(
                o,
                "{}", tf!("  {} {:<26} Durchl.{} T{:<2} 0x{:016X} soll {:016X} ist {:016X} xor {:016X}{}", "  {} {:<26} pass {} T{:<2} 0x{:016X} expected {:016X} got {:016X} xor {:016X}{}",
                e.time.format("%H:%M:%S"),
                e.test,
                e.pass + 1,
                e.thread,
                e.address,
                e.expected,
                e.actual,
                e.xor(),
                if e.simulated { t!(" (Selbsttest)", " (self-test)") } else { "" }));
        }
        let bits: Vec<String> = r
            .bit_histogram
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(b, c)| format!("Bit {b}: {c}×"))
            .collect();
        if !bits.is_empty() {
            let _ = writeln!(o, "  Betroffene Datenbits: {}", bits.join(", "));
        }
    }
    if let Some(run) = &r.bench_suite {
        let _ = writeln!(o, "\nBenchmark-Suite ({}):", run.settings_text());
        for c in &run.cpu {
            match &c.skipped {
                Some(why) => {
                    let _ = writeln!(o, "  {:<34} {why}", c.kind.name());
                }
                None => {
                    let _ = writeln!(
                        o,
                        "{}", tf!("  {:<34} {:<9} 1T {:>9.1} {u:<6} {:>5} Pkt   {}T {:>9.1} {u:<6} {:>5} Pkt   Skalierung {:.1}x   {}", "  {:<34} {:<9} 1T {:>9.1} {u:<6} {:>5} pts   {}T {:>9.1} {u:<6} {:>5} pts   scaling {:.1}x   {}",
                        c.kind.name(), c.variant, c.single_raw, c.single_points, c.threads, c.multi_raw, c.multi_points, c.scaling,
                        if c.errors > 0 { tf!("{} RECHENFEHLER", "{} CALCULATION ERRORS", c.errors) } else { t!("Prüfsummen ok", "checksums ok").into() },
                        u = c.kind.unit()));
                }
            }
        }
        if let Some(st) = &run.stress {
            let _ = writeln!(
                o,
                "{}", tf!("  CPU+RAM-Last: Ø {:.1} GB/s, min {:.1}, max {:.1}, Stabilität {:.0} %, {} Fehler, {:.0} s", "  CPU+RAM load: avg {:.1} GB/s, min {:.1}, max {:.1}, stability {:.0} %, {} errors, {:.0} s",
                st.avg_gbs, st.min_gbs, st.max_gbs, st.stability() * 100.0, st.errors, st.secs));
        }
        if let Some(g) = &run.gpu {
            match &g.skipped {
                Some(why) => {
                    let _ = writeln!(o, "  GPU: {why}");
                }
                None => {
                    let _ = writeln!(
                        o,
                        "{}", tf!("  GPU {} ({}, {}): VRAM {:.0} GB/s, FP32 {:.2} TFLOPS, zur GPU {:.1} GB/s, zum PC {:.1} GB/s, {} Fehler", "  GPU {} ({}, {}): VRAM {:.0} GB/s, FP32 {:.2} TFLOPS, to GPU {:.1} GB/s, to PC {:.1} GB/s, {} errors",
                        g.adapter, g.backend, g.driver, g.vram_gbs, g.fp32_tflops, g.upload_gbs, g.download_gbs, g.errors));
                }
            }
        }
    }
    if let Some(b) = &r.benchmark {
        let _ = writeln!(
            o,
            "\nBenchmark ({} Threads, {}):",
            b.threads,
            gib(b.buffer_bytes)
        );
        let _ = writeln!(
            o,
            "{}",
            tf!(
                "  Lesen {:.2} GB/s  Schreiben {:.2} GB/s  Kopieren {:.2} GB/s",
                "  Read {:.2} GB/s  Write {:.2} GB/s  Copy {:.2} GB/s",
                b.read_gbs,
                b.write_gbs,
                b.copy_gbs
            )
        );
        for (size, ns) in &b.latency {
            let _ = writeln!(o, "  Latenz {:>8}: {:.1} ns", size_label(*size), ns);
        }
    }
    if !r.sensors.is_empty() {
        let _ = writeln!(o, "\nSensoren (aktuell / min / max / Durchschnitt):");
        for s in &r.sensors {
            let _ = writeln!(
                o,
                "  {:<16} {:<34} {:<30} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {}",
                s.group, s.device, s.label, s.current, s.min, s.max, s.avg, s.unit
            );
        }
    }
    if !r.events.is_empty() {
        let _ = writeln!(o, "\nEreignisse:");
        for e in &r.events {
            let _ = writeln!(o, "  {} {}", e.time.format("%H:%M:%S"), e.text);
        }
    }
    for r in &r.sensor_sessions {
        let _ = writeln!(
            o,
            "{}",
            tf!(
                "\n{} (Beginn {}, Dauer {}):",
                "\n{} (start {}, duration {}):",
                session_title(r),
                r.started.format("%H:%M:%S"),
                crate::chart::fmt_secs(r.duration_secs)
            )
        );
        if let Some(c) = &r.csv {
            let _ = writeln!(o, "  CSV: {c}");
        }
        let _ = writeln!(
            o,
            "  {:<16} {:<34} {:<30} {:>12} {:>12} {:>12}",
            t!("Gruppe", "Group"),
            t!("Gerät", "Device"),
            "Sensor",
            "Min",
            "Max",
            "Ø"
        );
        for s in &r.series {
            let _ = writeln!(
                o,
                "  {:<16} {:<34} {:<30} {:>12} {:>12} {:>12}",
                s.group,
                s.device,
                s.label,
                fmt_num(s.min, &s.unit),
                fmt_num(s.max, &s.unit),
                fmt_num(s.avg, &s.unit)
            );
        }
        let errs = r
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Error)
            .count();
        if errs > 0 {
            let _ = writeln!(
                o,
                "{}", tf!("  {errs} RAM-Fehler während der Aufzeichnung (Zeitpunkte in CSV und HTML-Bericht)", "  {errs} RAM errors during the recording (times in the CSV and HTML report)"));
        }
    }
    let _ = writeln!(o, "{}", tf!("\nHinweis: Ein bestandener Test beweist nicht, dass der Speicher fehlerfrei ist. Siehe DISCLAIMER.md.", "\nNote: a passed test does not prove that the memory is free of errors. See DISCLAIMER.md."));
    o
}

pub fn size_label(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{} MiB", bytes >> 20)
    } else {
        format!("{} KiB", bytes >> 10)
    }
}

pub fn to_html(r: &Report) -> String {
    let mut o = String::new();
    let _ = write!(
        o,
        r#"<!doctype html><html lang="de"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>RAMForge Bericht {date}</title><style>
:root{{--bg:#f6f7f9;--fg:#15181d;--card:#fff;--mute:#5b6370;--line:#dde1e6;--ok:#13804a;--bad:#c0262d;--warn:#a86400;--acc:#d9531e}}
@media (prefers-color-scheme:dark){{:root{{--bg:#0f1115;--fg:#e7e9ee;--card:#171a20;--mute:#9aa3b0;--line:#2a2f38;--ok:#3ccf7d;--bad:#ff5a5f;--warn:#f0a43a;--acc:#ff7a3d}}}}
body{{font:15px/1.5 system-ui,Segoe UI,sans-serif;background:var(--bg);color:var(--fg);margin:0;padding:24px}}
main{{max-width:1100px;margin:auto}} h1{{margin:0 0 4px}} h1 span{{color:var(--acc)}} .sub{{color:var(--mute);margin-bottom:24px}}
section{{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:16px 20px;margin:16px 0}}
h2{{font-size:17px;margin:0 0 12px}} table{{border-collapse:collapse;width:100%;font-size:13.5px}}
th,td{{text-align:left;padding:5px 8px;border-bottom:1px solid var(--line);vertical-align:top}} th{{color:var(--mute);font-weight:600}}
td.n{{text-align:right;font-variant-numeric:tabular-nums}} code{{font:12.5px ui-monospace,Consolas,monospace}}
.badge{{display:inline-block;padding:4px 12px;border-radius:999px;font-weight:700;color:#fff}}
.ok{{background:var(--ok)}} .bad{{background:var(--bad)}} .warn{{background:var(--warn)}}
.kv td:first-child{{color:var(--mute);width:180px}}
svg.chart{{width:100%;height:auto;display:block;margin:6px 0 2px}} svg .grid{{stroke:var(--line)}} svg .ax{{fill:var(--mute);font-size:11px}}
svg .ct{{fill:var(--fg);font-size:12px;font-weight:600}} svg .phase{{stroke:var(--mute);stroke-dasharray:4 3}} svg .err{{stroke:var(--bad)}} svg .info{{stroke:var(--line)}}
.legend{{font-size:12.5px;color:var(--mute);margin:0 0 10px}} .legend span{{margin-right:14px;white-space:nowrap}}
.legend i{{display:inline-block;width:12px;height:3px;margin-right:5px;vertical-align:middle;background:var(--mute)}}
.legend i.mk-err{{background:var(--bad)}} .legend i.mk-phase{{background:repeating-linear-gradient(90deg,var(--mute) 0 3px,transparent 3px 5px)}} .bad-t td{{color:var(--bad)}} .foot{{color:var(--mute);font-size:12.5px;margin-top:24px}}
</style></head><body><main>
<h1><span>RAM</span>Forge Bericht</h1><div class="sub">Version {ver} · erstellt am {date}{anon}</div>"#,
        date = r.generated.format("%d.%m.%Y %H:%M:%S"),
        ver = esc(&r.version),
        anon = if r.anonymized {
            t!(
                " · Rechnername und Seriennummern ausgeblendet",
                " · computer name and serial numbers hidden"
            )
        } else {
            ""
        },
    );
    o.push_str("<section><h2>System</h2><table class=\"kv\">");
    for (k, v) in system_lines(r) {
        let _ = write!(o, "<tr><td>{}</td><td>{}</td></tr>", esc(&k), esc(&v));
    }
    o.push_str("</table></section>");
    let rows = module_rows(r);
    if !rows.is_empty() {
        o.push_str(t!("<section><h2>Speichermodule</h2><table><tr><th>Steckplatz</th><th>Größe</th><th>Typ</th><th>Nenn/Ist</th><th>Spannung</th><th>Hersteller</th><th>Teilenummer</th><th>Seriennr.</th></tr>", "<section><h2>Memory modules</h2><table><tr><th>Slot</th><th>Size</th><th>Type</th><th>Rated/actual</th><th>Voltage</th><th>Manufacturer</th><th>Part number</th><th>Serial no.</th></tr>"));
        for m in rows {
            o.push_str("<tr>");
            for c in m {
                let _ = write!(o, "<td>{}</td>", esc(&c));
            }
            o.push_str("</tr>");
        }
        o.push_str("</table></section>");
    }
    if let Some(t) = &r.test {
        let cls = match t.outcome {
            ramforge_core::Outcome::Passed => "ok",
            ramforge_core::Outcome::Failed => "bad",
            _ => "warn",
        };
        let _ = write!(
            o,
            "{}", tf!("<section><h2>RAM-Test</h2><p><span class=\"badge {cls}\">{}</span> &nbsp;{}</p><table class=\"kv\">\
             <tr><td>Beginn</td><td>{}</td></tr><tr><td>Dauer</td><td>{}</td></tr><tr><td>Getestet</td><td>{} mit {} Threads</td></tr>\
             <tr><td>Durchläufe / Tests</td><td>{} / {}</td></tr><tr><td>Speicher gesperrt</td><td>{}</td></tr><tr><td>Fehler</td><td>{}</td></tr></table>", "<section><h2>RAM test</h2><p><span class=\"badge {cls}\">{}</span> &nbsp;{}</p><table class=\"kv\"><tr><td>Start</td><td>{}</td></tr><tr><td>Duration</td><td>{}</td></tr><tr><td>Tested</td><td>{} with {} threads</td></tr><tr><td>Passes / tests</td><td>{} / {}</td></tr><tr><td>Memory locked</td><td>{}</td></tr><tr><td>Errors</td><td>{}</td></tr></table>",
            outcome_text(t.outcome), esc(&t.message), t.started.format("%d.%m.%Y %H:%M:%S"), duration(t.duration_secs),
            gib(t.tested_bytes), t.threads, t.passes_completed, t.tests_completed,
            if t.memory_locked { t!("ja", "yes") } else { t!("nein (Ergebnis bedingt aussagekräftig)", "no (result only partly meaningful)") }, t.errors));
        if !r.errors.is_empty() {
            o.push_str(t!("<h2 style=\"margin-top:16px\">Fehler</h2><table><tr><th>Zeit</th><th>Test</th><th>Durchl.</th><th>Thread</th><th>Adresse</th><th>Soll</th><th>Ist</th><th>XOR</th></tr>", "<h2 style=\"margin-top:16px\">Errors</h2><table><tr><th>Time</th><th>Test</th><th>Pass</th><th>Thread</th><th>Address</th><th>Expected</th><th>Actual</th><th>XOR</th></tr>"));
            for e in r.errors.iter().take(200) {
                let _ = write!(
                    o,
                    "<tr><td>{}</td><td>{}{}</td><td>{}</td><td>{}</td><td><code>0x{:016X}</code></td><td><code>{:016X}</code></td><td><code>{:016X}</code></td><td><code>{:016X}</code></td></tr>",
                    e.time.format("%H:%M:%S"), esc(&e.test), if e.simulated { t!(" (Selbsttest)", " (self-test)") } else { "" }, e.pass + 1, e.thread,
                    e.address, e.expected, e.actual, e.xor()
                );
            }
            o.push_str("</table>");
        }
        o.push_str("</section>");
    }
    if let Some(run) = &r.bench_suite {
        let _ = write!(
            o,
            "<section><h2>Benchmark-Suite</h2><p class=\"sub\">{}</p>",
            esc(&run.settings_text())
        );
        if !run.cpu.is_empty() {
            let (st, mt) = run.cpu_points();
            let _ = write!(o, "{}", tf!("<p><b>CPU:</b> {st} Punkte (1 Thread) · {mt} Punkte (alle Threads)</p><table><tr><th>Test</th><th>Befehlssatz</th><th>1 Thread</th><th>Alle Threads</th><th>Punkte</th><th>Skalierung</th><th>Prüfung</th></tr>", "<p><b>CPU:</b> {st} points (1 thread) · {mt} points (all threads)</p><table><tr><th>Test</th><th>Instruction set</th><th>1 thread</th><th>All threads</th><th>Points</th><th>Scaling</th><th>Check</th></tr>"));
            for c in &run.cpu {
                match &c.skipped {
                    Some(why) => {
                        let _ = write!(
                            o,
                            "<tr><td>{}</td><td>{}</td><td colspan=5>{}</td></tr>",
                            esc(c.kind.name()),
                            esc(&c.variant),
                            esc(why)
                        );
                    }
                    None => {
                        let _ = write!(
                            o,
                            "<tr><td>{}</td><td>{}</td><td class=n>{:.1} {u}</td><td class=n>{:.1} {u}</td><td class=n>{} / {}</td><td class=n>{:.1}×</td><td{}>{}</td></tr>",
                            esc(c.kind.name()), esc(&c.variant), c.single_raw, c.multi_raw, c.single_points, c.multi_points, c.scaling,
                            if c.errors > 0 { " class=\"bad-t\"" } else { "" },
                            if c.errors > 0 { tf!("{} Rechenfehler", "{} calculation errors", c.errors) } else { "ok".into() },
                            u = c.kind.unit()
                        );
                    }
                }
            }
            o.push_str("</table>");
        }
        if let Some(st) = &run.stress {
            let _ = write!(
                o,
                "{}", tf!("<p><b>CPU+RAM unter Last:</b> Ø {:.1} GB/s · min {:.1} · max {:.1} · Stabilität {:.0} % · <span{}>{} Fehler</span> · {:.0} s</p>", "<p><b>CPU+RAM under load:</b> avg {:.1} GB/s · min {:.1} · max {:.1} · stability {:.0} % · <span{}>{} errors</span> · {:.0} s</p>",
                st.avg_gbs, st.min_gbs, st.max_gbs, st.stability() * 100.0,
                if st.errors > 0 { " style=\"color:var(--bad)\"" } else { "" }, st.errors, st.secs));
        }
        if let Some(g) = &run.gpu {
            match &g.skipped {
                Some(why) => {
                    let _ = write!(o, "<p><b>GPU:</b> {}</p>", esc(why));
                }
                None => {
                    let _ = write!(
                        o,
                        "{}", tf!("<p><b>GPU:</b> {} ({}, {}) · VRAM {:.0} GB/s · FP32 {:.2} TFLOPS · zur GPU {:.1} GB/s · zum PC {:.1} GB/s · {} Fehler</p>", "<p><b>GPU:</b> {} ({}, {}) · VRAM {:.0} GB/s · FP32 {:.2} TFLOPS · to GPU {:.1} GB/s · to PC {:.1} GB/s · {} errors</p>",
                        esc(&g.adapter), esc(&g.backend), esc(&g.driver), g.vram_gbs, g.fp32_tflops, g.upload_gbs, g.download_gbs, g.errors));
                }
            }
        }
        o.push_str("</section>");
    }
    if let Some(b) = &r.benchmark {
        let _ = write!(
            o,
            "{}", tf!("<section><h2>Benchmark</h2><table class=\"kv\"><tr><td>Lesen</td><td>{:.2} GB/s</td></tr><tr><td>Schreiben</td><td>{:.2} GB/s</td></tr><tr><td>Kopieren</td><td>{:.2} GB/s</td></tr>", "<section><h2>Benchmark</h2><table class=\"kv\"><tr><td>Read</td><td>{:.2} GB/s</td></tr><tr><td>Write</td><td>{:.2} GB/s</td></tr><tr><td>Copy</td><td>{:.2} GB/s</td></tr>",
            b.read_gbs, b.write_gbs, b.copy_gbs));
        for (size, ns) in &b.latency {
            let _ = write!(
                o,
                "{}",
                tf!(
                    "<tr><td>Latenz {}</td><td>{:.1} ns</td></tr>",
                    "<tr><td>Latency {}</td><td>{:.1} ns</td></tr>",
                    size_label(*size),
                    ns
                )
            );
        }
        o.push_str("</table></section>");
    }
    if !r.sensors.is_empty() {
        o.push_str(t!("<section><h2>Sensoren</h2><table><tr><th>Gruppe</th><th>Gerät</th><th>Sensor</th><th>Aktuell</th><th>Min</th><th>Max</th><th>Ø</th><th></th></tr>", "<section><h2>Sensors</h2><table><tr><th>Group</th><th>Device</th><th>Sensor</th><th>Current</th><th>Min</th><th>Max</th><th>Avg</th><th></th></tr>"));
        for s in &r.sensors {
            let _ = write!(
                o,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=n>{:.2}</td><td class=n>{:.2}</td><td class=n>{:.2}</td><td class=n>{:.2}</td><td>{}</td></tr>",
                esc(&s.group), esc(&s.device), esc(&s.label), s.current, s.min, s.max, s.avg, esc(&s.unit)
            );
        }
        o.push_str("</table></section>");
    }
    if !r.events.is_empty() {
        o.push_str(t!(
            "<section><h2>Ereignisse</h2><table>",
            "<section><h2>Events</h2><table>"
        ));
        for e in &r.events {
            let _ = write!(
                o,
                "<tr><td>{}</td><td>{}</td></tr>",
                e.time.format("%d.%m. %H:%M:%S"),
                esc(&e.text)
            );
        }
        o.push_str("</table></section>");
    }
    for sr in &r.sensor_sessions {
        o.push_str(&session_html(sr));
    }
    o.push_str(t!("<p class=\"foot\">Ein bestandener Test beweist nicht, dass der Speicher fehlerfrei ist. Nutzung auf eigene Gefahr, ohne Gewähr. RAMForge ist freie Software unter der GPL-3.0-or-later.</p></main></body></html>", "<p class=\"foot\">A passed test does not prove that the memory is free of errors. Use at your own risk, without warranty. RAMForge is free software under the GPL-3.0-or-later.</p></main></body></html>"));
    o
}

/// Writes JSON, HTML and TXT. Returns the path of the HTML file.
pub fn save_all(r: &Report, dir: &Path) -> std::io::Result<PathBuf> {
    let base = dir.join(format!("ramforge-report-{}", crate::paths::stamp()));
    std::fs::write(
        base.with_extension("json"),
        serde_json::to_string_pretty(r).unwrap_or_default(),
    )?;
    std::fs::write(base.with_extension("txt"), to_txt(r))?;
    let html = base.with_extension("html");
    std::fs::write(&html, to_html(r))?;
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramforge_hw::sensors::{SensorGroup, SensorKind, SessionSeries};

    fn series(
        id: &str,
        label: &str,
        unit: &str,
        kind: SensorKind,
        group: SensorGroup,
        vals: &[f64],
    ) -> SessionSeries {
        SessionSeries {
            id: id.into(),
            group,
            device: "Testgerät".into(),
            label: label.into(),
            kind,
            unit: unit.into(),
            min: vals.iter().cloned().fold(f64::INFINITY, f64::min),
            max: vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            avg: vals.iter().sum::<f64>() / vals.len() as f64,
            samples: vals.len() as u64,
            points: vals
                .iter()
                .enumerate()
                .map(|(i, v)| [i as f64, *v])
                .collect(),
        }
    }

    fn session() -> SessionData {
        let ev = |t: f64, kind: EventKind, text: &str| SensorEvent {
            time: chrono::Local::now(),
            text: text.into(),
            kind,
            t,
        };
        SessionData {
            label: "test".into(),
            started: chrono::Local::now(),
            duration_secs: 60.0,
            csv: Some(PathBuf::from(
                r"I:\RAMForge\RAMForge-data\logs\sensors-x-test.csv",
            )),
            series: vec![
                series(
                    "t1",
                    "CPU (Tctl/Tdie)",
                    "°C",
                    SensorKind::Temperature,
                    SensorGroup::Cpu,
                    &[45.0, 71.5, 88.25, 60.0],
                ),
                series(
                    "l1",
                    "Auslastung gesamt",
                    "%",
                    SensorKind::Load,
                    SensorGroup::Cpu,
                    &[5.0, 99.0, 100.0, 7.0],
                ),
                series(
                    "v1",
                    "DRAM <VDIMM>",
                    "V",
                    SensorKind::Voltage,
                    SensorGroup::Memory,
                    &[1.35, 1.344, 1.351, 1.35],
                ),
                series(
                    "x1",
                    "Thread 3 Auslastung",
                    "%",
                    SensorKind::Load,
                    SensorGroup::Cpu,
                    &[1.0, 2.0],
                ),
            ],
            events: vec![
                ev(1.0, EventKind::Phase, "Durchlauf 1 · Eigene Adresse"),
                ev(
                    2.5,
                    EventKind::Error,
                    "RAM-Fehler #1 Adresse 0x1000 Bit 13 (Selbsttest, simuliert)",
                ),
            ],
        }
    }

    fn sys() -> SystemInfo {
        ramforge_hw::SystemInfo {
            collected: chrono::Local::now(),
            os: "Windows 11".into(),
            hostname: "PC".into(),
            cpu: Default::default(),
            smbios: None,
            smbios_error: None,
            memory: Default::default(),
            spd: vec![],
        }
    }

    #[test]
    fn report_contains_chart_table_and_markers() {
        let d = session();
        let r = build(ReportInput {
            system: &sys(),
            test: None,
            errors: &[],
            bit_histogram: &[0; 64],
            benchmark: None,
            bench_run: None,
            sensors: &[],
            events: vec![],
            sessions: vec![&d],
            anonymize: true,
        });
        let html = to_html(&r);
        assert!(
            html.contains("<svg") && html.contains("<polyline"),
            "kein Diagramm"
        );
        assert_eq!(
            html.matches("<svg").count(),
            3,
            "je Einheit ein Diagramm (°C, %, V)"
        );
        assert!(
            html.contains(r#"class="err""#) && html.contains(r#"class="phase""#),
            "Markierungen fehlen"
        );
        assert!(html.contains("RAM-Fehler #1 Adresse 0x1000"));
        assert!(html.contains("DRAM &lt;VDIMM&gt;"), "Text nicht escaped");
        assert!(!html.contains("<script"), "kein JavaScript im Bericht");
        assert!(
            html.contains("88.2") || html.contains("88.3"),
            "Maximum fehlt in der Tabelle"
        );
        assert!(
            !html.contains("Thread 3 Auslastung"),
            "Thread-Werte gehören nicht in den Bericht"
        );
        assert!(html.contains("sensors-x-test.csv"));
        let txt = to_txt(&r);
        assert!(
            txt.contains("Sensorverlauf RAM-Test")
                && txt.contains("CPU (Tctl/Tdie)")
                && txt.contains("1 RAM-Fehler")
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"sensor_sessions\"") && json.contains("\"points\""));
    }
}
