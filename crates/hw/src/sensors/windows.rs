//! Sensors Windows provides without any driver.
//!
//! Real CPU package temperatures, SuperIO voltages/fans and DIMM temperatures
//! need ring-0 access on Windows. That is planned via the signed PawnIO driver
//! (see docs/roadmap.md); until then HWiNFO's shared memory fills the gap.

use super::{Reading, SensorGroup, SensorKind, SensorSource};
#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use windows_sys::Wdk::System::SystemInformation::NtQuerySystemInformation;
use windows_sys::Win32::System::Performance::*;
use windows_sys::Win32::System::Power::CallNtPowerInformation;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION: i32 = 8;
const PROCESSOR_INFORMATION: i32 = 11;
const PDH_MORE_DATA: u32 = 0x8000_07D2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcPerf {
    idle: i64,
    kernel: i64, // includes idle time
    user: i64,
    dpc: i64,
    interrupt: i64,
    interrupt_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcPower {
    number: u32,
    max_mhz: u32,
    current_mhz: u32,
    mhz_limit: u32,
    max_idle_state: u32,
    current_idle_state: u32,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct BuiltinSource {
    cores: usize,
    last: Vec<ProcPerf>,
    max_mhz: u32,
    query: PDH_HQUERY,
    perf_counter: Option<PDH_HCOUNTER>,
    thermal_counter: Option<PDH_HCOUNTER>,
    values: usize,
}

// PDH handles are only used from the sensor thread that owns the source.
unsafe impl Send for BuiltinSource {}

impl BuiltinSource {
    pub fn new() -> Self {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let mut s = Self {
            cores,
            last: Vec::new(),
            max_mhz: 0,
            query: std::ptr::null_mut(),
            perf_counter: None,
            thermal_counter: None,
            values: 0,
        };
        // Base ("max") clock as reported by the power manager.
        let mut pw = vec![ProcPower::default(); cores];
        let ok = unsafe {
            CallNtPowerInformation(
                PROCESSOR_INFORMATION,
                std::ptr::null(),
                0,
                pw.as_mut_ptr() as *mut _,
                (pw.len() * std::mem::size_of::<ProcPower>()) as u32,
            )
        } == 0;
        if ok {
            s.max_mhz = pw.iter().map(|p| p.max_mhz).max().unwrap_or(0);
        }
        unsafe {
            if PdhOpenQueryW(std::ptr::null(), 0, &mut s.query) == 0 {
                // English counter names work on every Windows display language.
                let mut c: PDH_HCOUNTER = std::ptr::null_mut();
                let p = wide("\\Processor Information(_Total)\\% Processor Performance");
                if PdhAddEnglishCounterW(s.query, p.as_ptr(), 0, &mut c) == 0 {
                    s.perf_counter = Some(c);
                }
                let mut c: PDH_HCOUNTER = std::ptr::null_mut();
                let p = wide("\\Thermal Zone Information(*)\\Temperature");
                if PdhAddEnglishCounterW(s.query, p.as_ptr(), 0, &mut c) == 0 {
                    s.thermal_counter = Some(c);
                }
                PdhCollectQueryData(s.query);
            }
        }
        // First CPU time sample, so the very first poll already reports load
        // (otherwise a recording started right away would miss those columns).
        s.cpu_load(&mut Vec::new());
        s
    }

    fn cpu_load(&mut self, out: &mut Vec<Reading>) {
        let mut cur = vec![ProcPerf::default(); self.cores];
        let mut ret = 0u32;
        let st = unsafe {
            NtQuerySystemInformation(
                SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION,
                cur.as_mut_ptr() as *mut _,
                (cur.len() * std::mem::size_of::<ProcPerf>()) as u32,
                &mut ret,
            )
        };
        if st != 0 {
            return;
        }
        cur.truncate(ret as usize / std::mem::size_of::<ProcPerf>());
        if self.last.len() == cur.len() {
            let (mut busy_all, mut total_all) = (0i64, 0i64);
            let mut per_core = Vec::with_capacity(cur.len());
            for (a, b) in self.last.iter().zip(&cur) {
                let total = (b.kernel - a.kernel) + (b.user - a.user);
                let idle = b.idle - a.idle;
                let busy = (total - idle).max(0);
                busy_all += busy;
                total_all += total;
                per_core.push(if total > 0 {
                    busy as f64 * 100.0 / total as f64
                } else {
                    0.0
                });
            }
            let total = if total_all > 0 {
                busy_all as f64 * 100.0 / total_all as f64
            } else {
                0.0
            };
            out.push(reading(
                "cpu/load",
                SensorGroup::Cpu,
                "CPU",
                t!("Auslastung gesamt", "Total load"),
                SensorKind::Load,
                total,
                "%",
            ));
            out.push(reading(
                "cpu/load_max",
                SensorGroup::Cpu,
                "CPU",
                t!("Auslastung höchster Thread", "Load of busiest thread"),
                SensorKind::Load,
                per_core.iter().cloned().fold(0.0, f64::max),
                "%",
            ));
            for (i, v) in per_core.into_iter().enumerate() {
                out.push(reading(
                    &format!("cpu/load/{i}"),
                    SensorGroup::Cpu,
                    "CPU",
                    &tf!("Thread {i} Auslastung", "Thread {i} load"),
                    SensorKind::Load,
                    v,
                    "%",
                ));
            }
        }
        self.last = cur;
    }

    fn pdh(&mut self, out: &mut Vec<Reading>) {
        if self.query.is_null() {
            return;
        }
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return;
            }
            if let Some(c) = self.perf_counter {
                let mut v: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
                if PdhGetFormattedCounterValue(c, PDH_FMT_DOUBLE, std::ptr::null_mut(), &mut v) == 0
                    && self.max_mhz > 0
                {
                    let mhz = v.Anonymous.doubleValue / 100.0 * self.max_mhz as f64;
                    out.push(reading(
                        "cpu/clock",
                        SensorGroup::Cpu,
                        "CPU",
                        t!("Effektiver Takt (Mittel)", "Effective clock (avg)"),
                        SensorKind::Clock,
                        mhz,
                        "MHz",
                    ));
                }
            }
            if let Some(c) = self.thermal_counter {
                let mut size = 0u32;
                let mut count = 0u32;
                let r = PdhGetFormattedCounterArrayW(
                    c,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    std::ptr::null_mut(),
                );
                if r == PDH_MORE_DATA && size > 0 {
                    // Buffer must be 8-byte aligned for the item structs.
                    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
                    let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
                    if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, items)
                        == 0
                    {
                        for i in 0..count as usize {
                            let it = &*items.add(i);
                            let name = wstr(it.szName);
                            let kelvin = it.FmtValue.Anonymous.doubleValue;
                            if kelvin > 200.0 {
                                out.push(reading(
                                    &format!("acpi/{name}"),
                                    SensorGroup::Mainboard,
                                    "ACPI",
                                    &tf!("Thermalzone {name}", "Thermal zone {name}"),
                                    SensorKind::Temperature,
                                    kelvin - 273.15,
                                    "°C",
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    fn memory(&mut self, out: &mut Vec<Reading>) {
        let mut s: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut s) } == 0 {
            return;
        }
        let gib = |b: u64| b as f64 / (1u64 << 30) as f64;
        out.push(reading(
            "mem/used",
            SensorGroup::Memory,
            "RAM",
            t!("Belegt", "Used"),
            SensorKind::Other,
            gib(s.ullTotalPhys - s.ullAvailPhys),
            "GiB",
        ));
        out.push(reading(
            "mem/avail",
            SensorGroup::Memory,
            "RAM",
            t!("Frei", "Free"),
            SensorKind::Other,
            gib(s.ullAvailPhys),
            "GiB",
        ));
        out.push(reading(
            "mem/load",
            SensorGroup::Memory,
            "RAM",
            t!("Auslastung", "Load"),
            SensorKind::Load,
            s.dwMemoryLoad as f64,
            "%",
        ));
        let commit_used = s.ullTotalPageFile.saturating_sub(s.ullAvailPageFile);
        out.push(reading(
            "mem/commit",
            SensorGroup::Memory,
            "RAM",
            t!("Zugesichert (Commit)", "Committed"),
            SensorKind::Other,
            gib(commit_used),
            "GiB",
        ));
    }
}

impl Default for BuiltinSource {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BuiltinSource {
    fn drop(&mut self) {
        if !self.query.is_null() {
            unsafe { PdhCloseQuery(self.query) };
        }
    }
}

unsafe fn wstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

fn reading(
    id: &str,
    group: SensorGroup,
    device: &str,
    label: &str,
    kind: SensorKind,
    value: f64,
    unit: &str,
) -> Reading {
    Reading {
        id: format!("win/{id}"),
        source: "Windows".into(),
        group,
        device: device.into(),
        label: label.into(),
        kind,
        value,
        unit: unit.into(),
    }
}

impl SensorSource for BuiltinSource {
    fn name(&self) -> &str {
        t!("Windows (integriert)", "Windows (built-in)")
    }
    fn status(&self) -> String {
        tf!("aktiv, {} Werte", "active, {} values", self.values)
    }
    fn poll(&mut self, out: &mut Vec<Reading>) {
        let before = out.len();
        self.cpu_load(out);
        self.pdh(out);
        self.memory(out);
        self.values = out.len() - before;
    }
}
