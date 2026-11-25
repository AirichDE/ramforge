//! Combined CPU + RAM stress: every core generates data (CPU load), moves it
//! through large buffers (memory load) and verifies every word. Measures the
//! sustained throughput per second and reports any mismatch as a memory
//! error, just like the RAM test.

use crate::errors::{ErrorSink, MemError};
use crate::rng::{splitmix, Rng};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const TEST_NAME: &str = "CPU+RAM-Last";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StressResult {
    pub secs: f64,
    pub threads: usize,
    pub buffer_bytes: u64,
    /// Bytes written and verified, all threads.
    pub verified_bytes: u64,
    /// Throughput per second (GB/s) in time order.
    pub samples: Vec<f64>,
    pub avg_gbs: f64,
    pub min_gbs: f64,
    pub max_gbs: f64,
    pub errors: u64,
    pub first_errors: Vec<MemError>,
    pub cancelled: bool,
}

impl StressResult {
    /// Lowest second relative to the average; 1.0 = perfectly steady.
    pub fn stability(&self) -> f64 {
        if self.avg_gbs > 0.0 {
            self.min_gbs / self.avg_gbs
        } else {
            0.0
        }
    }
}

fn worker(t: usize, words: usize, stop: &AtomicBool, bytes: &AtomicU64, sink: &ErrorSink) {
    let half = (words / 2).max(1024);
    let mut a = vec![0u64; half];
    let mut b = vec![0u64; half];
    let base = b.as_ptr() as usize;
    let mut round = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let seed = splitmix(((t as u64) << 40) ^ round);
        // CPU: generate; RAM: write A.
        let mut r = Rng::new(seed);
        for x in a.iter_mut() {
            *x = r.next();
        }
        // RAM: A -> B in bursts, mixing in a cheap transform so the copy is not a plain memcpy.
        for (dst, src) in b.iter_mut().zip(a.iter()) {
            *dst = src.rotate_left(17) ^ seed;
        }
        // Verify B against the regenerated sequence.
        let mut r = Rng::new(seed);
        for (i, &got) in b.iter().enumerate() {
            let want = r.next().rotate_left(17) ^ seed;
            let got = unsafe { std::ptr::read_volatile(&got) };
            if got != want {
                sink.report(MemError {
                    time: chrono::Local::now(),
                    test: TEST_NAME.into(),
                    pass: round as u32,
                    thread: t,
                    address: (base + i * 8) as u64,
                    expected: want,
                    actual: got,
                    simulated: false,
                });
            }
        }
        bytes.fetch_add((half * 8 * 3) as u64, Ordering::Relaxed);
        round += 1;
    }
}

/// Runs for `secs` seconds on `threads` threads with `bytes_per_thread` each.
/// `progress(fraction, text)` is called about once per second.
pub fn run(
    threads: usize,
    bytes_per_thread: usize,
    secs: u64,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32, &str),
) -> StressResult {
    let threads = threads.max(1);
    let words = (bytes_per_thread / 8).max(4096);
    let stop = AtomicBool::new(false);
    let bytes = AtomicU64::new(0);
    let sink = ErrorSink::new(100, false);
    let mut samples = Vec::new();
    let t0 = Instant::now();
    std::thread::scope(|s| {
        for t in 0..threads {
            let (stop, bytes, sink) = (&stop, &bytes, &sink);
            s.spawn(move || worker(t, words, stop, bytes, sink));
        }
        let mut last = 0u64;
        let mut tick = Instant::now();
        while t0.elapsed().as_secs() < secs && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            if tick.elapsed() >= Duration::from_secs(1) {
                let now = bytes.load(Ordering::Relaxed);
                let gbs = (now - last) as f64 / tick.elapsed().as_secs_f64() / 1e9;
                samples.push(gbs);
                last = now;
                tick = Instant::now();
                progress(
                    (t0.elapsed().as_secs_f64() / secs as f64) as f32,
                    &format!("{gbs:.1} GB/s · {} Fehler", sink.count()),
                );
            }
        }
        stop.store(true, Ordering::Relaxed);
    });
    // The first second includes allocation and page faults: not counted.
    let steady: Vec<f64> = if samples.len() > 2 { samples[1..].to_vec() } else { samples.clone() };
    let avg = if steady.is_empty() { 0.0 } else { steady.iter().sum::<f64>() / steady.len() as f64 };
    StressResult {
        secs: t0.elapsed().as_secs_f64(),
        threads,
        buffer_bytes: (words * 8 * threads) as u64,
        verified_bytes: bytes.load(Ordering::Relaxed),
        avg_gbs: avg,
        min_gbs: steady.iter().cloned().fold(f64::INFINITY, f64::min).min(avg),
        max_gbs: steady.iter().cloned().fold(0.0, f64::max),
        samples,
        errors: sink.count(),
        first_errors: sink.errors(),
        cancelled: cancel.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_stress_runs_clean() {
        let cancel = AtomicBool::new(false);
        let r = run(2, 8 << 20, 3, &cancel, &|_, _| {});
        assert_eq!(r.errors, 0);
        assert!(r.samples.len() >= 2, "{:?}", r.samples);
        assert!(r.avg_gbs > 0.1 && r.verified_bytes > 0);
        assert!(r.stability() > 0.0 && r.stability() <= 1.0);
    }
}
