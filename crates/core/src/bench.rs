//! Memory bandwidth and latency benchmark.

use crate::rng::Rng;
#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use serde::{Deserialize, Serialize};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Barrier;
use std::time::Instant;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BenchResult {
    pub time: Option<chrono::DateTime<chrono::Local>>,
    pub threads: usize,
    pub buffer_bytes: u64,
    /// GB/s (10^9 bytes per second)
    pub read_gbs: f64,
    pub write_gbs: f64,
    /// Bytes copied per second (each byte is read once and written once).
    pub copy_gbs: f64,
    /// (block size in bytes, ns per dependent load)
    pub latency: Vec<(u64, f64)>,
}

impl BenchResult {
    /// Latency of the largest block, which is the DRAM latency.
    pub fn dram_latency_ns(&self) -> Option<f64> {
        self.latency.last().map(|l| l.1)
    }
}

#[derive(Clone, Copy)]
enum Op {
    Read,
    Write,
    Copy,
}

const REPEATS: usize = 4;

/// Runs all benchmarks. `progress(fraction, label)` is called between steps.
/// Returns None if cancelled.
pub fn run(
    threads: usize,
    bytes_per_thread: usize,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32, &str),
) -> Option<BenchResult> {
    let threads = threads.max(1);
    let words = (bytes_per_thread / 8).max(1 << 16);
    let mut res = BenchResult {
        time: Some(chrono::Local::now()),
        threads,
        buffer_bytes: (words * 8 * threads) as u64,
        ..Default::default()
    };
    progress(0.0, t!("Lesen", "Read"));
    res.read_gbs = bandwidth(threads, words, Op::Read, cancel)?;
    progress(0.2, t!("Schreiben", "Write"));
    res.write_gbs = bandwidth(threads, words, Op::Write, cancel)?;
    progress(0.4, t!("Kopieren", "Copy"));
    res.copy_gbs = bandwidth(threads, words, Op::Copy, cancel)?;
    let sizes: [u64; 9] = [
        16 << 10,
        64 << 10,
        256 << 10,
        1 << 20,
        4 << 20,
        16 << 20,
        64 << 20,
        256 << 20,
        512 << 20,
    ];
    for (i, &size) in sizes.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        progress(
            0.6 + 0.4 * i as f32 / sizes.len() as f32,
            t!("Latenz", "Latency"),
        );
        res.latency.push((size, latency_ns(size as usize)));
    }
    progress(1.0, t!("Fertig", "Done"));
    Some(res)
}

/// Best-of-N bandwidth over all threads in GB/s.
fn bandwidth(threads: usize, words: usize, op: Op, cancel: &AtomicBool) -> Option<f64> {
    let barrier = Barrier::new(threads + 1);
    // Each worker records its own start and end. The coordinating thread may
    // be scheduled late after a barrier, so its clock would cut the interval
    // short and report impossible bandwidth.
    let spans: std::sync::Mutex<Vec<(Instant, Instant)>> =
        std::sync::Mutex::new(Vec::with_capacity(threads));
    let mut best = 0.0f64;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (barrier, spans) = (&barrier, &spans);
                s.spawn(move || {
                    // Touch every page before timing so page faults are not measured.
                    let mut buf = vec![t as u64 + 1; words];
                    for _ in 0..=REPEATS {
                        barrier.wait();
                        let start = Instant::now();
                        match op {
                            Op::Read => {
                                // black_box: the compiler must not know the buffer
                                // contents, or it computes the sum without reading.
                                let data = black_box(buf.as_slice());
                                let mut acc = [0u64; 4];
                                for c in data.chunks_exact(4) {
                                    acc[0] = acc[0].wrapping_add(c[0]);
                                    acc[1] = acc[1].wrapping_add(c[1]);
                                    acc[2] = acc[2].wrapping_add(c[2]);
                                    acc[3] = acc[3].wrapping_add(c[3]);
                                }
                                black_box(acc);
                            }
                            Op::Write => {
                                buf.fill(black_box(0x5A5A_5A5A_5A5A_5A5A));
                                // Keeps the stores: the buffer counts as read afterwards.
                                black_box(buf.as_mut_slice());
                            }
                            Op::Copy => {
                                let h = buf.len() / 2;
                                let (a, b) = buf.split_at_mut(h);
                                b[..h].copy_from_slice(black_box(&a[..h]));
                                black_box(b);
                            }
                        }
                        spans.lock().unwrap().push((start, Instant::now()));
                        barrier.wait();
                    }
                })
            })
            .collect();
        let bytes = match op {
            Op::Copy => (words / 2 * 8 * threads) as f64,
            _ => (words * 8 * threads) as f64,
        };
        // First round is a warm-up and not counted.
        for round in 0..=REPEATS {
            barrier.wait();
            barrier.wait();
            // Wall time of the round: earliest start to latest end over all workers.
            let mut v = spans.lock().unwrap();
            let first = v.iter().map(|s| s.0).min();
            let last = v.iter().map(|s| s.1).max();
            v.clear();
            drop(v);
            if let (true, Some(a), Some(b)) = (round > 0, first, last) {
                let dt = b.duration_since(a).as_secs_f64();
                if dt > 0.0 {
                    best = best.max(bytes / dt / 1e9);
                }
            }
        }
        for h in handles {
            let _ = h.join();
        }
    });
    (!cancel.load(Ordering::Relaxed)).then_some(best)
}

/// Pointer chasing over a random cyclic permutation of cache lines.
/// Every load depends on the previous one, so prefetchers can not help.
fn latency_ns(size: usize) -> f64 {
    const LINE_WORDS: usize = 8; // 64-byte cache line
    let lines = (size / 64).max(2);
    let mut buf = vec![0usize; lines * LINE_WORDS];
    // Sattolo's algorithm: a single cycle through all lines.
    let mut order: Vec<usize> = (0..lines).collect();
    let mut r = Rng::new(size as u64);
    for i in (1..lines).rev() {
        let j = r.below(i as u64) as usize;
        order.swap(i, j);
    }
    for i in 0..lines {
        buf[order[i] * LINE_WORDS] = order[(i + 1) % lines] * LINE_WORDS;
    }
    drop(order);
    let hops = 4_000_000usize;
    let mut idx = 0usize;
    for _ in 0..lines.min(hops) {
        idx = unsafe { *buf.get_unchecked(idx) };
    }
    let t0 = Instant::now();
    for _ in 0..hops {
        idx = unsafe { *buf.get_unchecked(idx) };
    }
    let dt = t0.elapsed().as_secs_f64();
    black_box(idx);
    dt * 1e9 / hops as f64
}
