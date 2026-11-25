//! Sensor chart for a recorded RAM test or benchmark session.

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, LineStyle, Plot, PlotPoints, VLine};
use ramforge_hw::sensors::{EventKind, SensorGroup, SensorKind, SessionData, SessionSeries};
use std::collections::BTreeMap;

pub const ERROR_RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
const PHASE_GREY: Color32 = Color32::from_rgb(0x8A, 0x90, 0x9A);

/// Series colours, also used for the SVG in the HTML report.
pub const PALETTE: [Color32; 10] = [
    Color32::from_rgb(0xF2, 0x6B, 0x2A),
    Color32::from_rgb(0x3B, 0x9E, 0xFF),
    Color32::from_rgb(0x2E, 0xB8, 0x6A),
    Color32::from_rgb(0xC0, 0x7C, 0xFF),
    Color32::from_rgb(0xE8, 0xC2, 0x2C),
    Color32::from_rgb(0x20, 0xC5, 0xC7),
    Color32::from_rgb(0xFF, 0x7E, 0xB6),
    Color32::from_rgb(0x9C, 0xB3, 0x3B),
    Color32::from_rgb(0xB0, 0x8D, 0x6B),
    Color32::from_rgb(0x7A, 0x8C, 0xFF),
];

pub fn color_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

pub struct ChartState {
    key: String,
    pub selected: Vec<String>,
    pub normalized: bool,
    pub table_all: bool,
}

impl Default for ChartState {
    fn default() -> Self {
        Self {
            key: String::new(),
            selected: Vec::new(),
            normalized: false,
            table_all: false,
        }
    }
}

fn lab(s: &SessionSeries, words: &[&str]) -> bool {
    let l = format!("{} {}", s.label, s.device).to_lowercase();
    words.iter().any(|w| l.contains(w))
}

/// The sensors that matter for a RAM test, picked from what was recorded.
pub fn default_selection(d: &SessionData) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut take = |pred: &dyn Fn(&SessionSeries) -> bool, max: usize| {
        for s in d.series.iter().filter(|s| pred(s)).take(max) {
            if !out.contains(&s.id) {
                out.push(s.id.clone());
            }
        }
    };
    let temp = |s: &SessionSeries| s.kind == SensorKind::Temperature;
    take(
        &|s| {
            s.group == SensorGroup::Cpu
                && temp(s)
                && lab(
                    s,
                    &[
                        "tctl",
                        "tdie",
                        "package",
                        "core max",
                        "hottest",
                        "cpu (tctl",
                    ],
                )
        },
        2,
    );
    take(&|s| s.group == SensorGroup::Cpu && temp(s), 1);
    take(
        &|s| {
            s.group == SensorGroup::Cpu
                && s.kind == SensorKind::Load
                && lab(s, &["gesamt", "total"])
        },
        1,
    );
    take(
        &|s| {
            s.group == SensorGroup::Cpu
                && s.kind == SensorKind::Clock
                && lab(s, &["effektiv", "effective"])
                && !lab(s, &["thread", "core #"])
        },
        1,
    );
    take(&|s| s.group == SensorGroup::Memory && temp(s), 4);
    take(
        &|s| {
            s.kind == SensorKind::Voltage
                && lab(
                    s,
                    &[
                        "vdimm", "dram", "vddq", "vdd ", "vdd", "vsoc", "vccsa", "vcore",
                        "cpu core",
                    ],
                )
        },
        5,
    );
    take(
        &|s| s.group == SensorGroup::System && lab(s, &["whea", "error"]),
        2,
    );
    take(
        &|s| s.group == SensorGroup::Memory && s.kind == SensorKind::Load,
        1,
    );
    out
}

fn unit_group(u: &str) -> String {
    match u {
        "°C" => "Temperatur (°C)".into(),
        "V" => "Spannung (V)".into(),
        "%" => "Auslastung (%)".into(),
        "MHz" => "Takt (MHz)".into(),
        "RPM" => "Lüfter (RPM)".into(),
        "W" => "Leistung (W)".into(),
        "" => "ohne Einheit".into(),
        other => other.to_string(),
    }
}

/// Seconds as m:ss or h:mm:ss.
pub fn fmt_secs(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A round tick step (0.1 s … hours) of at least `min` seconds.
pub fn nice_step(min: f64) -> f64 {
    const STEPS: [f64; 17] = [
        0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0,
        3600.0, 7200.0,
    ];
    STEPS
        .iter()
        .copied()
        .find(|s| *s >= min)
        .unwrap_or_else(|| (min / 3600.0).ceil() * 3600.0)
}

/// Axis label: whole seconds as m:ss / h:mm:ss, sub-second steps as "1.5 s".
pub fn fmt_tick(t: f64, step: f64) -> String {
    if step < 1.0 {
        format!("{:.1} s", t.max(0.0))
    } else {
        fmt_secs(t.round())
    }
}

fn series_name(s: &SessionSeries) -> String {
    if s.device.is_empty() {
        s.label.clone()
    } else {
        format!("{} · {}", s.label, s.device)
    }
}

/// Groups the selected series by unit (or one normalised group).
/// Returns (title, [(colour, series)]) in a stable order.
pub fn groups<'a>(
    d: &'a SessionData,
    selected: &[String],
    normalized: bool,
) -> Vec<(String, Vec<(Color32, &'a SessionSeries)>)> {
    let sel: Vec<(Color32, &SessionSeries)> = selected
        .iter()
        .filter_map(|id| d.series.iter().find(|s| &s.id == id))
        .enumerate()
        .map(|(i, s)| (PALETTE[i % PALETTE.len()], s))
        .collect();
    if normalized {
        return if sel.is_empty() {
            vec![]
        } else {
            vec![(
                "Normalisiert (0 % = Minimum, 100 % = Maximum je Sensor)".into(),
                sel,
            )]
        };
    }
    let mut map: BTreeMap<String, Vec<(Color32, &SessionSeries)>> = BTreeMap::new();
    for (c, s) in sel {
        map.entry(unit_group(&s.unit)).or_default().push((c, s));
    }
    map.into_iter().collect()
}

pub fn normalize(s: &SessionSeries) -> Vec<[f64; 2]> {
    let span = s.max - s.min;
    s.points
        .iter()
        .map(|p| {
            [
                p[0],
                if span > 0.0 {
                    (p[1] - s.min) / span * 100.0
                } else {
                    50.0
                },
            ]
        })
        .collect()
}

/// Draws the whole chart block: controls, one plot per unit, table, events.
pub fn show(ui: &mut egui::Ui, d: &SessionData, st: &mut ChartState, id: &str) {
    let key = format!("{}-{}", d.label, d.started.to_rfc3339());
    if st.key != key {
        st.key = key;
        st.selected = default_selection(d);
    }
    let errors = d
        .events
        .iter()
        .filter(|e| e.kind == EventKind::Error)
        .count();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "Sensorverlauf · {} · {} · Beginn {} · {} Sensoren",
                if d.label == "bench" {
                    "Benchmark"
                } else {
                    "RAM-Test"
                },
                fmt_secs(d.duration_secs),
                d.started.format("%H:%M:%S"),
                d.series.len()
            ))
            .strong(),
        );
        if errors > 0 {
            ui.label(RichText::new(format!("{errors} Fehler-Markierungen")).color(ERROR_RED));
        }
        if let Some(csv) = &d.csv {
            if ui
                .small_button("CSV öffnen")
                .on_hover_text(csv.display().to_string())
                .clicked()
            {
                crate::paths::open_folder(csv);
            }
        }
    });
    if d.series.is_empty() {
        ui.label(RichText::new("Keine Sensorwerte aufgezeichnet.").weak());
        return;
    }
    ui.horizontal(|ui| {
        ui.checkbox(
            &mut st.normalized,
            "Normalisiert (alle Sensoren in einem Diagramm)",
        );
        egui::CollapsingHeader::new(format!(
            "Sensoren auswählen ({} von {})",
            st.selected.len(),
            d.series.len()
        ))
        .id_salt((id, "picker"))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("Standard").clicked() {
                    st.selected = default_selection(d);
                }
                if ui.small_button("Keine").clicked() {
                    st.selected.clear();
                }
            });
            egui::ScrollArea::vertical()
                .id_salt((id, "pick-scroll"))
                .max_height(220.0)
                .show(ui, |ui| {
                    for g in SensorGroup::ALL {
                        let items: Vec<&SessionSeries> = d
                            .series
                            .iter()
                            .filter(|s| s.group == g && !s.label.starts_with("Thread "))
                            .collect();
                        if items.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(g.name()).small().strong());
                        for s in items {
                            let mut on = st.selected.contains(&s.id);
                            if ui
                                .checkbox(&mut on, format!("{} [{}]", series_name(s), s.unit))
                                .changed()
                            {
                                if on {
                                    st.selected.push(s.id.clone());
                                } else {
                                    st.selected.retain(|x| x != &s.id);
                                }
                            }
                        }
                    }
                });
        });
    });

    let link = egui::Id::unique((id, "link"));
    let groups = groups(d, &st.selected, st.normalized);
    if groups.is_empty() {
        ui.label(RichText::new("Keine Sensoren ausgewählt.").weak());
    }
    for (title, items) in &groups {
        ui.label(RichText::new(title).small().strong());
        let normalized = st.normalized;
        let resp = Plot::new((id, title.as_str()))
            .height(if groups.len() > 2 { 140.0 } else { 170.0 })
            .legend(Legend::default())
            .link_axis(link, [true, false])
            .link_cursor(link, [true, false])
            .allow_scroll(false)
            .include_x(0.0)
            .include_x(d.duration_secs.max(1.0))
            .x_axis_formatter(|m, _| fmt_tick(m.value, m.step_size))
            .show(ui, |p| {
                for (c, s) in items {
                    let pts = if normalized {
                        normalize(s)
                    } else {
                        s.points.clone()
                    };
                    p.line(
                        Line::new(series_name(s), PlotPoints::from(pts))
                            .color(*c)
                            .width(1.6),
                    );
                }
                for e in &d.events {
                    let line = match e.kind {
                        EventKind::Error => VLine::new("", e.t).color(ERROR_RED).width(1.5),
                        EventKind::Phase => VLine::new("", e.t)
                            .color(PHASE_GREY)
                            .style(LineStyle::dashed_dense())
                            .width(1.0),
                        EventKind::Info => VLine::new("", e.t)
                            .color(PHASE_GREY.gamma_multiply(0.5))
                            .width(1.0),
                    };
                    p.vline(line);
                }
                // Which marker is under the mouse (within 6 px)?
                let ptr = p.pointer_coordinate()?;
                let per_px = p.transform().dvalue_dpos()[0].abs();
                d.events
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (i, (e.t - ptr.x).abs()))
                    .filter(|(_, dist)| *dist <= 6.0 * per_px)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i)
            });
        if let Some(i) = resp.inner {
            let e = &d.events[i];
            resp.response.on_hover_ui(|ui| {
                let c = if e.kind == EventKind::Error {
                    ERROR_RED
                } else {
                    ui.visuals().text_color()
                };
                ui.label(RichText::new(&e.text).strong().color(c));
                ui.label(
                    RichText::new(format!(
                        "bei {} ({})",
                        fmt_secs(e.t),
                        e.time.format("%H:%M:%S")
                    ))
                    .small(),
                );
                egui::Grid::new("ev-vals").num_columns(2).show(ui, |ui| {
                    for id in &st.selected {
                        if let Some(s) = d.series.iter().find(|s| &s.id == id) {
                            ui.label(RichText::new(series_name(s)).small());
                            ui.monospace(crate::app::fmt_val(
                                s.value_at(e.t).unwrap_or(f64::NAN),
                                &s.unit,
                            ));
                            ui.end_row();
                        }
                    }
                });
            });
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Min / Max / Ø im Testzeitraum")
                .small()
                .strong(),
        );
        ui.checkbox(&mut st.table_all, "alle Sensoren");
    });
    egui::Grid::new((id, "stats"))
        .num_columns(6)
        .striped(true)
        .show(ui, |ui| {
            for h in ["Gerät", "Sensor", "Min", "Max", "Ø", "Werte"] {
                ui.label(RichText::new(h).small().weak());
            }
            ui.end_row();
            for s in d.series.iter().filter(|s| {
                if st.table_all {
                    !s.label.starts_with("Thread ")
                } else {
                    st.selected.contains(&s.id)
                }
            }) {
                ui.label(RichText::new(&s.device).small());
                ui.label(&s.label);
                ui.monospace(crate::app::fmt_val(s.min, &s.unit));
                ui.monospace(crate::app::fmt_val(s.max, &s.unit));
                ui.monospace(crate::app::fmt_val(s.avg, &s.unit));
                ui.label(RichText::new(s.samples.to_string()).small().weak());
                ui.end_row();
            }
        });
    if !d.events.is_empty() {
        egui::CollapsingHeader::new(format!("Ereignisse ({})", d.events.len()))
            .id_salt((id, "events"))
            .show(ui, |ui| {
                for e in &d.events {
                    let c = match e.kind {
                        EventKind::Error => ERROR_RED,
                        _ => ui.visuals().text_color(),
                    };
                    ui.horizontal(|ui| {
                        ui.monospace(RichText::new(fmt_secs(e.t)).weak());
                        ui.label(RichText::new(&e.text).color(c));
                    });
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_labels_are_unique_for_short_and_long_runs() {
        for dur in [0.7, 2.0, 9.0, 61.0, 3_000.0, 40_000.0] {
            let step = nice_step(dur / 6.0);
            let mut labels = Vec::new();
            let mut t = 0.0;
            while t <= dur + 1e-9 {
                labels.push(fmt_tick(t, step));
                t += step;
            }
            let mut uniq = labels.clone();
            uniq.dedup();
            assert_eq!(
                uniq, labels,
                "doppelte Beschriftung bei {dur} s: {labels:?}"
            );
            assert!(labels.len() <= 8, "{dur} s: zu viele Striche {labels:?}");
        }
    }
}
