//! Linux sensors from /sys/class/hwmon (boot edition).
//!
//! The kernel drivers provide everything the boot stick needs:
//! coretemp / k10temp (CPU), nct6775 / it87 / asus-ec-sensors (board, fans,
//! voltages), spd5118 / jc42 (DIMM temperatures), nvme / drivetemp (storage).
//!
//! NOTE: written for the boot edition and not yet run on real hardware.

use super::{classify_device, Reading, SensorGroup, SensorKind, SensorSource};
#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use std::fs;
use std::path::{Path, PathBuf};

pub struct HwmonSource {
    count: usize,
}

impl HwmonSource {
    pub fn new() -> Self {
        Self { count: 0 }
    }
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

impl SensorSource for HwmonSource {
    fn name(&self) -> &str {
        "Linux hwmon"
    }
    fn status(&self) -> String {
        tf!("{} Werte", "{} values", self.count)
    }

    fn poll(&mut self, out: &mut Vec<Reading>) {
        let start = out.len();
        let Ok(dir) = fs::read_dir("/sys/class/hwmon") else {
            return;
        };
        let mut devs: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
        devs.sort();
        for dev in devs {
            let driver = read_trim(&dev.join("name")).unwrap_or_default();
            let hw = dev.file_name().unwrap().to_string_lossy().to_string();
            let device = format!("{driver} ({hw})");
            let group = classify_device(&driver);
            let Ok(files) = fs::read_dir(&dev) else {
                continue;
            };
            for f in files.flatten() {
                let fname = f.file_name().to_string_lossy().to_string();
                let Some(base) = fname.strip_suffix("_input") else {
                    continue;
                };
                let (kind, scale, unit) = match base.trim_end_matches(|c: char| c.is_ascii_digit())
                {
                    "temp" => (SensorKind::Temperature, 0.001, "°C"),
                    "in" => (SensorKind::Voltage, 0.001, "V"),
                    "fan" => (SensorKind::Fan, 1.0, "RPM"),
                    "curr" => (SensorKind::Current, 0.001, "A"),
                    "power" => (SensorKind::Power, 0.000_001, "W"),
                    "freq" => (SensorKind::Clock, 0.000_001, "MHz"),
                    _ => continue,
                };
                let Some(raw) = read_trim(&f.path()).and_then(|s| s.parse::<f64>().ok()) else {
                    continue;
                };
                // Unconnected inputs report placeholders, e.g. nct6775 AUXTIN
                // without a probe: -128 °C (or 127/255). Never a real reading.
                if kind == SensorKind::Temperature && !plausible_temperature(raw * scale) {
                    continue;
                }
                let label = read_trim(&dev.join(format!("{base}_label")))
                    .unwrap_or_else(|| base.to_string());
                out.push(Reading {
                    id: format!("hwmon:{hw}:{base}"),
                    source: "hwmon".into(),
                    group,
                    device: device.clone(),
                    label,
                    kind,
                    value: raw * scale,
                    unit: unit.into(),
                });
            }
        }
        self.count = out.len() - start;
    }
}

/// CPU load, clock and memory usage from /proc and cpufreq.
pub struct ProcSource {
    last: Vec<(u64, u64)>,
}

impl ProcSource {
    pub fn new() -> Self {
        Self { last: Self::stat() }
    }
    fn stat() -> Vec<(u64, u64)> {
        let s = fs::read_to_string("/proc/stat").unwrap_or_default();
        s.lines()
            .filter(|l| l.starts_with("cpu"))
            .map(|l| {
                let v: Vec<u64> = l
                    .split_whitespace()
                    .skip(1)
                    .filter_map(|x| x.parse().ok())
                    .collect();
                let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
                (idle, v.iter().sum())
            })
            .collect()
    }
}

impl SensorSource for ProcSource {
    fn name(&self) -> &str {
        "Linux /proc"
    }
    fn status(&self) -> String {
        "aktiv".into()
    }
    fn poll(&mut self, out: &mut Vec<Reading>) {
        let now = Self::stat();
        for (i, (a, b)) in self.last.iter().zip(now.iter()).enumerate() {
            let all = b.1.saturating_sub(a.1) as f64;
            let idle = b.0.saturating_sub(a.0) as f64;
            if all > 0.0 {
                let (id, label) = if i == 0 {
                    (
                        "load".to_string(),
                        t!("Gesamtauslastung", "Total load").to_string(),
                    )
                } else {
                    (
                        format!("load{}", i - 1),
                        tf!("Thread {} Auslastung", "Thread {} load", i - 1),
                    )
                };
                out.push(Reading {
                    id: format!("proc:{id}"),
                    source: "proc".into(),
                    group: SensorGroup::Cpu,
                    device: "CPU".into(),
                    label,
                    kind: SensorKind::Load,
                    value: (all - idle) / all * 100.0,
                    unit: "%".into(),
                });
            }
        }
        self.last = now;
        for i in 0.. {
            let p = format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq");
            let Some(khz) = read_trim(Path::new(&p)).and_then(|s| s.parse::<f64>().ok()) else {
                break;
            };
            out.push(Reading {
                id: format!("proc:clock{i}"),
                source: "cpufreq".into(),
                group: SensorGroup::Cpu,
                device: "CPU".into(),
                label: tf!("Thread {i} Takt", "Thread {i} clock"),
                kind: SensorKind::Clock,
                value: khz / 1000.0,
                unit: "MHz".into(),
            });
        }
        let m = ramforge_mem_status();
        let gib = (1u64 << 30) as f64;
        out.push(Reading {
            id: "proc:ram_used".into(),
            source: "proc".into(),
            group: SensorGroup::Memory,
            device: t!("Arbeitsspeicher", "Memory").into(),
            label: t!("Belegt", "Used").into(),
            kind: SensorKind::Other,
            value: (m.0 - m.1) as f64 / gib,
            unit: "GiB".into(),
        });
        out.push(Reading {
            id: "proc:ram_free".into(),
            source: "proc".into(),
            group: SensorGroup::Memory,
            device: t!("Arbeitsspeicher", "Memory").into(),
            label: t!("Verfügbar", "Available").into(),
            kind: SensorKind::Other,
            value: m.1 as f64 / gib,
            unit: "GiB".into(),
        });
    }
}

fn ramforge_mem_status() -> (u64, u64) {
    let text = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let f = |n: &str| {
        text.lines()
            .find(|l| l.starts_with(n))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    (f("MemTotal:"), f("MemAvailable:"))
}

/// Temperatures outside -40..150 °C come from open or missing probes.
pub(crate) fn plausible_temperature(c: f64) -> bool {
    (-40.0..=150.0).contains(&c)
}

#[cfg(test)]
mod plausibility_tests {
    #[test]
    fn placeholders_are_dropped() {
        for bad in [-128.0, 127.5, 255.0, -273.0] {
            assert!(!super::plausible_temperature(bad), "{bad}");
        }
        for ok in [-10.0, 29.0, 65.6, 105.0] {
            assert!(super::plausible_temperature(ok), "{ok}");
        }
    }
}
