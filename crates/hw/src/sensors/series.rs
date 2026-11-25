//! Long sensor series with bounded memory.
//!
//! A RAM test can run for many hours. A series keeps every sample until it
//! would exceed [`MAX_POINTS`]; then neighbouring samples are merged into
//! time buckets. Each bucket keeps its minimum AND maximum sample (with their
//! time stamps), so a short temperature spike or voltage drop is never lost,
//! only the samples between extremes are dropped.

use serde::{Deserialize, Serialize};

/// Upper limit of points per series (two per bucket: min and max).
pub const MAX_POINTS: usize = 5000;

#[derive(Clone, Copy, Debug)]
struct Bucket {
    start: f64,
    min: [f64; 2],
    max: [f64; 2],
}

#[derive(Clone, Debug)]
pub struct SeriesBuf {
    /// Bucket width in seconds; 0 = raw samples, not merged yet.
    width: f64,
    buckets: Vec<Bucket>,
    min: f64,
    max: f64,
    sum: f64,
    count: u64,
    max_points: usize,
}

impl SeriesBuf {
    pub fn new() -> Self {
        Self::with_limit(MAX_POINTS)
    }

    pub fn with_limit(max_points: usize) -> Self {
        Self {
            width: 0.0,
            buckets: Vec::new(),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            sum: 0.0,
            count: 0,
            max_points: max_points.max(4),
        }
    }

    pub fn push(&mut self, t: f64, v: f64) {
        if !v.is_finite() {
            return;
        }
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.sum += v;
        self.count += 1;
        let p = [t, v];
        match self.buckets.last_mut() {
            Some(b) if self.width > 0.0 && t < b.start + self.width => {
                if v < b.min[1] {
                    b.min = p;
                }
                if v > b.max[1] {
                    b.max = p;
                }
            }
            _ => {
                let start = if self.width > 0.0 { (t / self.width).floor() * self.width } else { t };
                self.buckets.push(Bucket { start, min: p, max: p });
            }
        }
        if self.buckets.len() * 2 > self.max_points {
            self.compact();
        }
    }

    /// Widens the buckets until the series fits the limit again.
    fn compact(&mut self) {
        // Iterative, at most 64 doublings: no recursion, no endless loop even
        // for pathological time stamps (all equal, going backwards, …).
        for _ in 0..64 {
            if self.buckets.len() * 2 <= self.max_points {
                return;
            }
            let (Some(first), Some(last)) = (self.buckets.first(), self.buckets.last()) else { return };
            let span = (last.start - first.start).abs().max(1e-9);
            self.width = if self.width > 0.0 {
                self.width * 2.0
            } else {
                // About a quarter of the limit in buckets after the first merge.
                span / (self.max_points as f64 / 4.0)
            };
            let w = self.width;
            let mut merged: Vec<Bucket> = Vec::with_capacity(self.buckets.len() / 2 + 1);
            for b in self.buckets.drain(..) {
                let start = (b.start / w).floor() * w;
                match merged.last_mut() {
                    Some(m) if (m.start - start).abs() < w * 1e-6 => {
                        if b.min[1] < m.min[1] {
                            m.min = b.min;
                        }
                        if b.max[1] > m.max[1] {
                            m.max = b.max;
                        }
                    }
                    _ => merged.push(Bucket { start, min: b.min, max: b.max }),
                }
            }
            self.buckets = merged;
        }
        // Still too many (degenerate input): keep the extreme buckets only.
        while self.buckets.len() * 2 > self.max_points {
            let i = self.buckets.len() / 2;
            let b = self.buckets.remove(i);
            let m = &mut self.buckets[i - 1];
            if b.min[1] < m.min[1] {
                m.min = b.min;
            }
            if b.max[1] > m.max[1] {
                m.max = b.max;
            }
        }
    }

    /// Points in time order; per bucket its minimum and maximum sample.
    pub fn points(&self) -> Vec<[f64; 2]> {
        let mut out = Vec::with_capacity(self.buckets.len() * 2);
        for b in &self.buckets {
            if b.min == b.max {
                out.push(b.min); // a single sample
            } else if b.min[0] <= b.max[0] {
                out.push(b.min);
                out.push(b.max);
            } else {
                out.push(b.max);
                out.push(b.min);
            }
        }
        out
    }

    /// A copy reduced to at most `limit` points (for reports), peaks kept.
    pub fn reduced(&self, limit: usize) -> Vec<[f64; 2]> {
        let mut s = SeriesBuf::with_limit(limit);
        for p in self.points() {
            s.push(p[0], p[1]);
        }
        s.points()
    }

    pub fn min(&self) -> f64 {
        self.min
    }
    pub fn max(&self) -> f64 {
        self.max
    }
    pub fn avg(&self) -> f64 {
        if self.count == 0 {
            f64::NAN
        } else {
            self.sum / self.count as f64
        }
    }
    pub fn samples(&self) -> u64 {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    /// Value at (or just before) time `t`.
    pub fn value_at(&self, t: f64) -> Option<f64> {
        let pts = self.points();
        let i = pts.partition_point(|p| p[0] <= t);
        if i == 0 {
            pts.first().map(|p| p[1])
        } else {
            Some(pts[i - 1][1])
        }
    }
}

/// Serializable summary of one series for reports.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeriesSummary {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub samples: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_raw_samples_below_the_limit() {
        let mut s = SeriesBuf::new();
        for i in 0..1000 {
            s.push(i as f64, (i % 7) as f64);
        }
        assert_eq!(s.points().len(), 1000);
        assert_eq!(s.samples(), 1000);
    }

    #[test]
    fn downsampling_keeps_peaks_and_bounds_memory() {
        let mut s = SeriesBuf::new();
        // 10 hours at 2 samples per second, smooth 50 °C, one short spike and one dip.
        let n = 72_000;
        for i in 0..n {
            let t = i as f64 * 0.5;
            let v = match i {
                40_001 => 97.5,
                55_555 => 12.25,
                _ => 50.0 + (i as f64 / 1000.0).sin(),
            };
            s.push(t, v);
        }
        let pts = s.points();
        assert!(pts.len() <= MAX_POINTS, "{} Punkte", pts.len());
        assert!(pts.len() > MAX_POINTS / 8, "zu stark verdichtet: {}", pts.len());
        assert!(pts.iter().any(|p| p[1] == 97.5), "Spitze verloren");
        assert!(pts.iter().any(|p| p[1] == 12.25), "Einbruch verloren");
        assert_eq!(s.max(), 97.5);
        assert_eq!(s.min(), 12.25);
        assert!(pts.windows(2).all(|w| w[0][0] <= w[1][0]), "Zeit nicht sortiert");
        let r = s.reduced(500);
        assert!(r.len() <= 500);
        assert!(r.iter().any(|p| p[1] == 97.5) && r.iter().any(|p| p[1] == 12.25));
        assert!((s.avg() - 50.0).abs() < 1.0);
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;

    #[test]
    fn degenerate_timestamps_do_not_hang() {
        let mut s = SeriesBuf::with_limit(10);
        for i in 0..1000 {
            s.push(0.0, i as f64); // all at the same time
        }
        assert!(s.points().len() <= 10);
        assert_eq!(s.max(), 999.0);
        assert!(s.points().iter().any(|p| p[1] == 999.0) && s.points().iter().any(|p| p[1] == 0.0));
        let mut r = SeriesBuf::with_limit(10);
        for i in 0..1000 {
            r.push(-(i as f64), 1.0); // time going backwards
        }
        assert!(r.points().len() <= 10);
    }
}
