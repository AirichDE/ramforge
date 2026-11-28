//! Live sensors (temperatures, voltages, fans, clocks, load) and recording.
//!
//! Sources:
//! * [`windows::BuiltinSource`]: no driver needed. CPU load per core, effective
//!   clock, RAM usage, ACPI thermal zone.
//! * [`hwinfo::HwinfoSource`]: reads HWiNFO's documented shared-memory interface
//!   if HWiNFO is running with "Shared Memory Support" enabled. Gives the full
//!   sensor set (SuperIO, DIMM temperatures, VRM, WHEA errors, timings).
//! * [`linux::HwmonSource`]: the Linux hwmon subsystem (boot edition).
//!
//! A [`SensorHub`] polls all sources on a background thread, keeps
//! min/max/avg and a history per sensor, and can record everything to CSV with
//! an event column (for example "memory error at address X").

#[cfg(windows)]
pub mod hwinfo;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod series;
#[cfg(windows)]
pub mod windows;

pub use series::SeriesBuf;

#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SensorGroup {
    Cpu,
    Memory,
    Mainboard,
    Storage,
    Gpu,
    System,
    Other,
}

impl SensorGroup {
    pub const ALL: [SensorGroup; 7] = [
        SensorGroup::Cpu,
        SensorGroup::Memory,
        SensorGroup::Mainboard,
        SensorGroup::Storage,
        SensorGroup::Gpu,
        SensorGroup::System,
        SensorGroup::Other,
    ];
    pub fn name(self) -> &'static str {
        match self {
            SensorGroup::Cpu => "CPU",
            SensorGroup::Memory => t!("Arbeitsspeicher", "Memory"),
            SensorGroup::Mainboard => "Mainboard",
            SensorGroup::Storage => t!("Laufwerke / Controller", "Drives / controllers"),
            SensorGroup::Gpu => t!("Grafik", "Graphics"),
            SensorGroup::System => "System",
            SensorGroup::Other => t!("Sonstige", "Other"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SensorKind {
    Temperature,
    Voltage,
    Fan,
    Current,
    Power,
    Clock,
    Load,
    Other,
}

impl SensorKind {
    pub fn name(self) -> &'static str {
        match self {
            SensorKind::Temperature => t!("Temperatur", "Temperature"),
            SensorKind::Voltage => t!("Spannung", "Voltage"),
            SensorKind::Fan => t!("Lüfter", "Fan"),
            SensorKind::Current => t!("Strom", "Current"),
            SensorKind::Power => t!("Leistung", "Power"),
            SensorKind::Clock => t!("Takt", "Clock"),
            SensorKind::Load => t!("Auslastung", "Load"),
            SensorKind::Other => t!("Sonstiges", "Other"),
        }
    }
}

/// One value of one sensor at one point in time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reading {
    /// Stable id, unique across sources.
    pub id: String,
    pub source: String,
    pub group: SensorGroup,
    /// Chip or device, e.g. "Nuvoton NCT6798D" or "CPU".
    pub device: String,
    pub label: String,
    pub kind: SensorKind,
    pub value: f64,
    pub unit: String,
}

pub trait SensorSource: Send {
    fn name(&self) -> &str;
    /// Short status for the UI, e.g. "aktiv, 84 Werte" or "HWiNFO läuft nicht".
    fn status(&self) -> String;
    fn poll(&mut self, out: &mut Vec<Reading>);
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SensorStat {
    pub reading: Reading,
    pub min: f64,
    pub max: f64,
    sum: f64,
    count: u64,
    /// (seconds since hub start, value)
    #[serde(skip)]
    pub history: VecDeque<[f64; 2]>,
    #[serde(skip)]
    last_seen: Option<Instant>,
}

impl SensorStat {
    pub fn avg(&self) -> f64 {
        if self.count == 0 {
            self.reading.value
        } else {
            self.sum / self.count as f64
        }
    }
    fn new(r: Reading) -> Self {
        Self {
            min: r.value,
            max: r.value,
            sum: 0.0,
            count: 0,
            reading: r,
            history: VecDeque::new(),
            last_seen: None,
        }
    }
    fn update(&mut self, r: Reading, t: f64, now: Instant) {
        let v = r.value;
        self.reading = r;
        if self.count == 0 {
            self.min = v;
            self.max = v;
        }
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.sum += v;
        self.count += 1;
        self.history.push_back([t, v]);
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.last_seen = Some(now);
    }
    fn reset(&mut self) {
        self.sum = 0.0;
        self.count = 0;
        self.min = self.reading.value;
        self.max = self.reading.value;
    }
    /// True if the sensor delivered a value within the last few seconds.
    pub fn is_live(&self) -> bool {
        self.last_seen
            .map(|t| t.elapsed() < Duration::from_secs(5))
            .unwrap_or(false)
    }
}

const HISTORY: usize = 1800;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    #[default]
    Info,
    /// Test phase or benchmark step change (grey marker).
    Phase,
    /// Memory error (red marker).
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SensorEvent {
    pub time: chrono::DateTime<chrono::Local>,
    pub text: String,
    #[serde(default)]
    pub kind: EventKind,
    /// Seconds since the start of the recording session (or of the hub).
    #[serde(default)]
    pub t: f64,
}

/// One sensor over a whole recording session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSeries {
    pub id: String,
    pub group: SensorGroup,
    pub device: String,
    pub label: String,
    pub kind: SensorKind,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub samples: u64,
    /// (seconds since session start, value); at most series::MAX_POINTS,
    /// min and max of every time bucket kept.
    pub points: Vec<[f64; 2]>,
}

impl SessionSeries {
    /// Value at (or just before) `t` seconds.
    pub fn value_at(&self, t: f64) -> Option<f64> {
        let i = self.points.partition_point(|p| p[0] <= t);
        if i == 0 {
            self.points.first().map(|p| p[1])
        } else {
            Some(self.points[i - 1][1])
        }
    }
    /// The points reduced to at most `limit`, peaks kept (for reports).
    pub fn reduced(&self, limit: usize) -> Vec<[f64; 2]> {
        let mut s = SeriesBuf::with_limit(limit);
        for p in &self.points {
            s.push(p[0], p[1]);
        }
        s.points()
    }
}

/// Everything recorded between `start_session` and `stop_session`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionData {
    /// "test" or "bench"
    pub label: String,
    pub started: chrono::DateTime<chrono::Local>,
    pub duration_secs: f64,
    pub csv: Option<PathBuf>,
    pub series: Vec<SessionSeries>,
    pub events: Vec<SensorEvent>,
}

struct Session {
    label: String,
    started: chrono::DateTime<chrono::Local>,
    t0: f64,
    csv: Option<PathBuf>,
    series: HashMap<String, SeriesBuf>,
    meta: HashMap<String, Reading>,
    events: Vec<SensorEvent>,
}

struct Recorder {
    path: PathBuf,
    out: std::io::BufWriter<std::fs::File>,
    columns: Vec<String>,
    rows: u64,
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[derive(Default)]
struct HubState {
    stats: HashMap<String, SensorStat>,
    order: Vec<String>,
    source_status: Vec<(String, String)>,
    pending_events: Vec<String>,
    events: Vec<SensorEvent>,
    recorder: Option<Recorder>,
    session: Option<Session>,
    polls: u64,
}

struct Inner {
    state: Mutex<HubState>,
    stop: AtomicBool,
    interval_ms: AtomicU64,
    started: Instant,
}

/// Polls all sources on a background thread. Cheap to clone.
#[derive(Clone)]
pub struct SensorHub {
    inner: Arc<Inner>,
}

impl SensorHub {
    /// Starts a hub with all sources available on this platform.
    pub fn start_default(interval: Duration) -> Self {
        Self::start(default_sources(), interval)
    }

    pub fn start(mut sources: Vec<Box<dyn SensorSource>>, interval: Duration) -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::new(HubState::default()),
            stop: AtomicBool::new(false),
            interval_ms: AtomicU64::new(interval.as_millis().max(100) as u64),
            started: Instant::now(),
        });
        let i2 = inner.clone();
        std::thread::Builder::new()
            .name("ramforge-sensors".into())
            .spawn(move || {
                let mut buf = Vec::with_capacity(256);
                while !i2.stop.load(Ordering::Relaxed) {
                    let t0 = Instant::now();
                    buf.clear();
                    for s in sources.iter_mut() {
                        s.poll(&mut buf);
                    }
                    let status = sources
                        .iter()
                        .map(|s| (s.name().to_string(), s.status()))
                        .collect();
                    Self::ingest(&i2, std::mem::take(&mut buf), status);
                    let wait = Duration::from_millis(i2.interval_ms.load(Ordering::Relaxed));
                    while t0.elapsed() < wait && !i2.stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            })
            .expect("spawn sensor thread");
        Self { inner }
    }

    fn ingest(inner: &Inner, readings: Vec<Reading>, status: Vec<(String, String)>) {
        let now = Instant::now();
        let t = inner.started.elapsed().as_secs_f64();
        let mut st = inner.state.lock().unwrap();
        st.source_status = status;
        st.polls += 1;
        if let Some(sess) = st.session.as_mut() {
            let ts = t - sess.t0;
            for r in &readings {
                sess.series
                    .entry(r.id.clone())
                    .or_insert_with(SeriesBuf::new)
                    .push(ts, r.value);
                sess.meta.insert(r.id.clone(), r.clone());
            }
        }
        let mut new_ids = false;
        for r in readings {
            match st.stats.get_mut(&r.id) {
                Some(s) => s.update(r, t, now),
                None => {
                    let id = r.id.clone();
                    let mut s = SensorStat::new(r.clone());
                    s.update(r, t, now);
                    st.stats.insert(id.clone(), s);
                    st.order.push(id);
                    new_ids = true;
                }
            }
        }
        if new_ids {
            let HubState { stats, order, .. } = &mut *st;
            order.sort_by(|a, b| {
                let (x, y) = (&stats[a].reading, &stats[b].reading);
                (x.group, &x.device, x.kind as u8, &x.label).cmp(&(
                    y.group,
                    &y.device,
                    y.kind as u8,
                    &y.label,
                ))
            });
        }
        let events = std::mem::take(&mut st.pending_events);
        let HubState {
            stats, recorder, ..
        } = &mut *st;
        if let Some(rec) = recorder.as_mut() {
            let mut line = format!(
                "{},{:.1}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                t
            );
            for id in &rec.columns {
                match stats.get(id) {
                    Some(s) if s.last_seen == Some(now) => {
                        line.push_str(&format!(",{}", s.reading.value))
                    }
                    _ => line.push(','),
                }
            }
            line.push(',');
            line.push_str(&csv_field(&events.join(" | ")));
            let _ = writeln!(rec.out, "{line}");
            rec.rows += 1;
            if rec.rows % 10 == 0 || !events.is_empty() {
                let _ = rec.out.flush();
            }
        }
    }

    /// All sensors in display order.
    pub fn snapshot(&self) -> Vec<SensorStat> {
        let st = self.inner.state.lock().unwrap();
        st.order
            .iter()
            .filter_map(|id| st.stats.get(id).cloned())
            .collect()
    }

    /// Snapshot without the (large) history, for tables and reports.
    pub fn snapshot_light(&self) -> Vec<SensorStat> {
        let st = self.inner.state.lock().unwrap();
        st.order
            .iter()
            .filter_map(|id| st.stats.get(id))
            .map(|s| SensorStat {
                history: VecDeque::new(),
                ..s.clone()
            })
            .collect()
    }

    pub fn history(&self, id: &str) -> Vec<[f64; 2]> {
        let st = self.inner.state.lock().unwrap();
        st.stats
            .get(id)
            .map(|s| s.history.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn source_status(&self) -> Vec<(String, String)> {
        self.inner.state.lock().unwrap().source_status.clone()
    }

    pub fn polls(&self) -> u64 {
        self.inner.state.lock().unwrap().polls
    }

    /// Seconds since the hub started (the time axis of `history`).
    pub fn now_secs(&self) -> f64 {
        self.inner.started.elapsed().as_secs_f64()
    }

    /// Resets min/max/avg, e.g. when a memory test starts.
    pub fn reset_stats(&self) {
        let mut st = self.inner.state.lock().unwrap();
        for s in st.stats.values_mut() {
            s.reset();
        }
    }

    /// Adds a marker to the event list and the next CSV row.
    pub fn mark(&self, text: impl Into<String>) {
        self.mark_kind(EventKind::Info, text)
    }

    /// Adds a typed marker (phase change, memory error, …). During a
    /// recording session it also becomes a marker in the session chart.
    pub fn mark_kind(&self, kind: EventKind, text: impl Into<String>) {
        let text = text.into();
        let now = self.inner.started.elapsed().as_secs_f64();
        let mut st = self.inner.state.lock().unwrap();
        let t = st.session.as_ref().map(|s| now - s.t0).unwrap_or(now);
        let ev = SensorEvent {
            time: chrono::Local::now(),
            text: text.clone(),
            kind,
            t,
        };
        if let Some(sess) = st.session.as_mut() {
            sess.events.push(ev.clone());
        }
        st.events.push(ev);
        let prefix = match kind {
            EventKind::Info => "",
            EventKind::Phase => t!("Phase: ", "Phase: "),
            EventKind::Error => t!("FEHLER: ", "ERROR: "),
        };
        st.pending_events.push(format!("{prefix}{text}"));
    }

    /// Starts a recording session: resets min/max/avg, keeps the full series
    /// of every sensor in memory and, if `csv` is given, writes the CSV log.
    /// A running session is stopped first.
    pub fn start_session(&self, label: &str, csv: Option<&Path>) -> std::io::Result<()> {
        let _ = self.stop_session();
        self.reset_stats();
        let rec = match csv {
            Some(p) => self.start_recording(p),
            None => Ok(()),
        };
        let mut st = self.inner.state.lock().unwrap();
        st.session = Some(Session {
            label: label.to_string(),
            started: chrono::Local::now(),
            t0: self.inner.started.elapsed().as_secs_f64(),
            csv: if rec.is_ok() {
                csv.map(Path::to_path_buf)
            } else {
                None
            },
            series: HashMap::new(),
            meta: HashMap::new(),
            events: Vec::new(),
        });
        rec
    }

    pub fn session_active(&self) -> bool {
        self.inner.state.lock().unwrap().session.is_some()
    }

    /// Ends the session and the CSV recording and returns what was recorded.
    pub fn stop_session(&self) -> Option<SessionData> {
        // Give the CSV a last row so the final event (test result) is written.
        let sess = {
            let mut st = self.inner.state.lock().unwrap();
            st.session.take()?
        };
        let pending = !self.inner.state.lock().unwrap().pending_events.is_empty();
        if pending {
            self.flush_events_row();
        }
        self.stop_recording();
        let duration = self.inner.started.elapsed().as_secs_f64() - sess.t0;
        let order = self.inner.state.lock().unwrap().order.clone();
        let mut ids: Vec<&String> = order
            .iter()
            .filter(|id| sess.series.contains_key(*id))
            .collect();
        for id in sess.series.keys() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let series = ids
            .into_iter()
            .filter_map(|id| {
                let buf = &sess.series[id];
                let r = sess.meta.get(id)?;
                (!buf.is_empty()).then(|| SessionSeries {
                    id: id.clone(),
                    group: r.group,
                    device: r.device.clone(),
                    label: r.label.clone(),
                    kind: r.kind,
                    unit: r.unit.clone(),
                    min: buf.min(),
                    max: buf.max(),
                    avg: buf.avg(),
                    samples: buf.samples(),
                    points: buf.points(),
                })
            })
            .collect();
        Some(SessionData {
            label: sess.label,
            started: sess.started,
            duration_secs: duration,
            csv: sess.csv,
            series,
            events: sess.events,
        })
    }

    /// Writes one CSV row with the pending events right now (used at the end
    /// of a session, so the last marker is not lost).
    fn flush_events_row(&self) {
        let t = self.inner.started.elapsed().as_secs_f64();
        let mut st = self.inner.state.lock().unwrap();
        let events = std::mem::take(&mut st.pending_events);
        if let Some(rec) = st.recorder.as_mut() {
            let mut line = format!(
                "{},{:.1}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                t
            );
            for _ in &rec.columns {
                line.push(',');
            }
            line.push(',');
            line.push_str(&csv_field(&events.join(" | ")));
            let _ = writeln!(rec.out, "{line}");
            rec.rows += 1;
        }
    }

    pub fn events(&self) -> Vec<SensorEvent> {
        self.inner.state.lock().unwrap().events.clone()
    }

    pub fn set_interval(&self, d: Duration) {
        self.inner
            .interval_ms
            .store(d.as_millis().max(100) as u64, Ordering::Relaxed);
    }

    /// Starts CSV recording. Columns are the sensors known right now.
    pub fn start_recording(&self, path: &Path) -> std::io::Result<()> {
        let file = std::fs::File::create(path)?;
        let mut out = std::io::BufWriter::new(file);
        let mut st = self.inner.state.lock().unwrap();
        let columns: Vec<String> = st.order.clone();
        let mut header = String::from(t!("Zeit,Sekunden", "Time,Seconds"));
        for id in &columns {
            let r = &st.stats[id].reading;
            header.push(',');
            header.push_str(&csv_field(&format!(
                "{} / {} [{}]",
                r.device, r.label, r.unit
            )));
        }
        header.push_str(t!(",Ereignis", ",Event"));
        // UTF-8 BOM so Excel shows umlauts and the degree sign correctly.
        out.write_all(b"\xEF\xBB\xBF")?;
        writeln!(out, "{header}")?;
        st.recorder = Some(Recorder {
            path: path.to_path_buf(),
            out,
            columns,
            rows: 0,
        });
        Ok(())
    }

    /// Stops recording and returns the file path and the number of rows.
    pub fn stop_recording(&self) -> Option<(PathBuf, u64)> {
        let mut st = self.inner.state.lock().unwrap();
        let mut rec = st.recorder.take()?;
        let _ = rec.out.flush();
        Some((rec.path, rec.rows))
    }

    pub fn recording_path(&self) -> Option<PathBuf> {
        self.inner
            .state
            .lock()
            .unwrap()
            .recorder
            .as_ref()
            .map(|r| r.path.clone())
    }

    pub fn shutdown(&self) {
        self.stop_recording();
        self.inner.stop.store(true, Ordering::Relaxed);
    }
}

/// All sensor sources for the current platform.
pub fn default_sources() -> Vec<Box<dyn SensorSource>> {
    #[allow(unused_mut)]
    let mut v: Vec<Box<dyn SensorSource>> = Vec::new();
    #[cfg(windows)]
    {
        v.push(Box::new(windows::BuiltinSource::new()));
        v.push(Box::new(hwinfo::HwinfoSource::new()));
    }
    #[cfg(target_os = "linux")]
    {
        v.push(Box::new(linux::ProcSource::new()));
        v.push(Box::new(linux::HwmonSource::new()));
    }
    v
}

/// Guesses the group from a chip/device name (used for HWiNFO and hwmon).
pub fn classify_device(name: &str) -> SensorGroup {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["whea", "windows hardware errors"]) {
        SensorGroup::System
    } else if has(&["gpu", "amdgpu", "nouveau", "radeon", "geforce"]) {
        SensorGroup::Gpu
    } else if has(&[
        "dimm", "memory", "spd", "ddr4", "ddr5", "jc42", "ee1004", "spd5118",
    ]) {
        SensorGroup::Memory
    } else if has(&["cpu", "coretemp", "k10temp", "zenpower", "processor"]) {
        SensorGroup::Cpu
    } else if has(&[
        "s.m.a.r.t",
        "drive",
        "nvme",
        "ssd",
        "disk",
        "drivetemp",
        "sata",
        "ahci",
    ]) {
        SensorGroup::Storage
    } else if has(&[
        "nuvoton",
        "nct",
        "ite ",
        "it87",
        "it86",
        "fintek",
        "f718",
        "embedded controller",
        "asus",
        "msi",
        "gigabyte",
        "asrock",
        "chipset",
        "acpi",
        "acpitz",
        "motherboard",
        "mainboard",
        "ec ",
        "superio",
    ]) {
        SensorGroup::Mainboard
    } else if has(&["system", "battery", "network"]) {
        SensorGroup::System
    } else {
        SensorGroup::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(f64);
    impl SensorSource for Fake {
        fn name(&self) -> &str {
            "fake"
        }
        fn status(&self) -> String {
            "ok".into()
        }
        fn poll(&mut self, out: &mut Vec<Reading>) {
            self.0 += 1.0;
            out.push(Reading {
                id: "fake/t".into(),
                source: "fake".into(),
                group: SensorGroup::Cpu,
                device: "Test-CPU".into(),
                label: "Kern, max".into(),
                kind: SensorKind::Temperature,
                value: self.0,
                unit: "°C".into(),
            });
        }
    }

    #[test]
    fn hub_tracks_min_max_and_records_csv() {
        let hub = SensorHub::start(vec![Box::new(Fake(0.0))], Duration::from_millis(100));
        while hub.polls() < 2 {
            std::thread::sleep(Duration::from_millis(20));
        }
        let dir = std::env::temp_dir().join(format!("ramforge-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.csv");
        hub.start_recording(&path).unwrap();
        hub.mark("Fehler bei 0x1000");
        let p0 = hub.polls();
        while hub.polls() < p0 + 3 {
            std::thread::sleep(Duration::from_millis(20));
        }
        let (_, rows) = hub.stop_recording().unwrap();
        hub.shutdown();
        let snap = hub.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(snap[0].max > snap[0].min);
        assert!(rows >= 3);
        let csv = std::fs::read_to_string(&path).unwrap();
        assert!(csv.contains("\"Test-CPU / Kern, max [°C]\""));
        assert!(csv.contains("Fehler bei 0x1000"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn session_records_series_and_markers() {
        let hub = SensorHub::start(vec![Box::new(Fake(0.0))], Duration::from_millis(100));
        while hub.polls() < 1 {
            std::thread::sleep(Duration::from_millis(20));
        }
        let dir = std::env::temp_dir().join(format!("ramforge-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("sensors-test.csv");
        assert!(!hub.session_active());
        hub.start_session("test", Some(&csv)).unwrap();
        assert!(hub.session_active());
        assert!(
            hub.recording_path().is_some(),
            "CSV muss mit dem Test starten"
        );
        hub.mark_kind(EventKind::Phase, "Durchlauf 1 · Eigene Adresse");
        let p0 = hub.polls();
        while hub.polls() < p0 + 3 {
            std::thread::sleep(Duration::from_millis(20));
        }
        hub.mark_kind(
            EventKind::Error,
            "RAM-Fehler #1 Adresse 0x1000 Bit 13 (Selbsttest)",
        );
        let p1 = hub.polls();
        while hub.polls() < p1 + 2 {
            std::thread::sleep(Duration::from_millis(20));
        }
        hub.mark("Test beendet: FEHLER GEFUNDEN");
        let data = hub.stop_session().expect("Sitzung");
        assert!(!hub.session_active());
        assert!(
            hub.recording_path().is_none(),
            "CSV muss mit dem Test enden"
        );
        hub.shutdown();

        assert_eq!(data.label, "test");
        assert_eq!(data.series.len(), 1);
        let s = &data.series[0];
        assert!(s.samples >= 5 && s.points.len() as u64 == s.samples);
        assert!(s.max > s.min);
        assert!(s.points.windows(2).all(|w| w[0][0] <= w[1][0]));
        let kinds: Vec<EventKind> = data.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![EventKind::Phase, EventKind::Error, EventKind::Info]
        );
        assert!(data.events[1].t >= data.events[0].t && data.events[0].t >= 0.0);

        let text = std::fs::read_to_string(&csv).unwrap();
        assert!(text.contains("Phase: Durchlauf 1 · Eigene Adresse"));
        assert!(text.contains("FEHLER: RAM-Fehler #1 Adresse 0x1000 Bit 13 (Selbsttest)"));
        assert!(
            text.contains("Test beendet: FEHLER GEFUNDEN"),
            "letztes Ereignis fehlt in der CSV"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn classifies_devices() {
        assert_eq!(
            classify_device("CPU [#0]: AMD Ryzen 7 5800X"),
            SensorGroup::Cpu
        );
        assert_eq!(
            classify_device("DDR5 DIMM [#2] (BANK 0)"),
            SensorGroup::Memory
        );
        assert_eq!(
            classify_device("ASUS ROG STRIX B550-F (Nuvoton NCT6798D)"),
            SensorGroup::Mainboard
        );
        assert_eq!(
            classify_device("S.M.A.R.T.: Samsung SSD 980"),
            SensorGroup::Storage
        );
        assert_eq!(
            classify_device("Windows Hardware Errors (WHEA)"),
            SensorGroup::System
        );
        assert_eq!(
            classify_device("GPU [#0]: NVIDIA GeForce RTX 4070"),
            SensorGroup::Gpu
        );
    }
}
