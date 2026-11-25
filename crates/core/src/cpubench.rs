//! CPU benchmarks: integer, floating point and SIMD, single- and multi-thread.
//!
//! Clean-room workloads with fixed sizes. Every working set fits into the
//! L2/L3 caches, so the memory bus does not dominate. Every result carries a
//! checksum that must equal the single-thread reference: a wrong checksum
//! means the CPU computed something wrong (unstable overclock, too little
//! voltage) and is reported as an error, like a memory error.

use crate::rng::{splitmix, Rng};
use serde::{Deserialize, Serialize};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Barrier;
use std::time::Instant;

/// Bumped whenever a workload changes. Scores are only comparable between
/// runs with the same workload version.
pub const WORKLOAD_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuKind {
    Integer,
    Float,
    Simd,
}

impl CpuKind {
    pub const ALL: [CpuKind; 3] = [CpuKind::Integer, CpuKind::Float, CpuKind::Simd];
    pub fn name(self) -> &'static str {
        match self {
            CpuKind::Integer => "Ganzzahl (Hash + Sortieren)",
            CpuKind::Float => "Gleitkomma (Matrix 128×128, f64)",
            CpuKind::Simd => "SIMD (FMA, f32)",
        }
    }
    pub fn unit(self) -> &'static str {
        match self {
            CpuKind::Integer => "MB/s",
            CpuKind::Float | CpuKind::Simd => "GFLOPS",
        }
    }
    /// Raw throughput that gives 1000 points (an arbitrary, fixed reference).
    fn reference(self) -> f64 {
        match self {
            CpuKind::Integer => 400.0,
            CpuKind::Float => 2.0,
            CpuKind::Simd => 20.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CpuScore {
    pub kind: CpuKind,
    /// Instruction set actually used (e.g. "AVX2+FMA", "AVX-512F", "skalar").
    pub variant: String,
    pub single_raw: f64,
    pub multi_raw: f64,
    pub single_points: u32,
    pub multi_points: u32,
    pub threads: usize,
    /// multi / single
    pub scaling: f64,
    /// Number of iterations whose checksum did not match the reference.
    pub errors: u64,
    /// Set if the test could not run (e.g. no AVX2).
    pub skipped: Option<String>,
}

fn points(kind: CpuKind, raw: f64) -> u32 {
    (raw / kind.reference() * 1000.0).round().max(0.0) as u32
}

// ------------------------------------------------------------- workloads

/// Integer: 256 KiB of data, hashed with a 64-bit mixing function, then a
/// radix sort of 32 Ki keys. Returns (checksum, bytes processed).
fn integer_round(seed: u64, data: &mut [u64], keys: &mut [u32], tmp: &mut [u32]) -> (u64, u64) {
    let mut r = Rng::new(seed);
    for d in data.iter_mut() {
        *d = r.next();
    }
    let mut h = seed;
    for (i, &d) in data.iter().enumerate() {
        h = splitmix(h ^ d).rotate_left((i & 31) as u32).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    for (i, k) in keys.iter_mut().enumerate() {
        *k = (data[i % data.len()] >> 17) as u32;
    }
    // LSD radix sort, 4 passes of 8 bits.
    for pass in 0..4 {
        let shift = pass * 8;
        let mut count = [0usize; 257];
        for &k in keys.iter() {
            count[((k >> shift) & 0xFF) as usize + 1] += 1;
        }
        for i in 0..256 {
            count[i + 1] += count[i];
        }
        for &k in keys.iter() {
            let b = ((k >> shift) & 0xFF) as usize;
            tmp[count[b]] = k;
            count[b] += 1;
        }
        keys.copy_from_slice(tmp);
    }
    let sorted_sum = keys.iter().enumerate().fold(0u64, |a, (i, &k)| a.wrapping_add((k as u64).wrapping_mul(i as u64 + 1)));
    let bytes = (data.len() * 8 + keys.len() * 4 * 4) as u64;
    (h ^ sorted_sum, bytes)
}

const N: usize = 128;

/// Float: C = A × B for 128×128 f64 matrices (3 × 128 KiB). Returns (checksum bits, flops).
fn float_round(seed: u64, a: &mut [f64], b: &mut [f64], c: &mut [f64]) -> (u64, u64) {
    let mut r = Rng::new(seed);
    for x in a.iter_mut().chain(b.iter_mut()) {
        *x = (r.next() >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
    }
    c.fill(0.0);
    // i-k-j order: unit stride in the inner loop, fixed summation order.
    for i in 0..N {
        for k in 0..N {
            let aik = a[i * N + k];
            let row_b = &b[k * N..k * N + N];
            let row_c = &mut c[i * N..i * N + N];
            for j in 0..N {
                row_c[j] += aik * row_b[j];
            }
        }
    }
    let sum: f64 = c.iter().sum();
    (sum.to_bits(), 2 * (N * N * N) as u64)
}

const SIMD_LEN: usize = 16 * 1024; // 64 KiB per f32 array
const SIMD_REPS: usize = 32;

fn simd_fill(seed: u64, x: &mut [f32], y: &mut [f32]) {
    let mut r = Rng::new(seed);
    for v in x.iter_mut().chain(y.iter_mut()) {
        *v = ((r.next() >> 40) as f32 / (1u64 << 24) as f32) - 0.5;
    }
}

/// Scalar reference with the same summation structure as the vector kernels
/// would need for bit-exact results, so it is only used when no AVX2 exists.
fn simd_scalar(x: &[f32], y: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    for _ in 0..SIMD_REPS {
        for (cx, cy) in x.chunks_exact(8).zip(y.chunks_exact(8)) {
            for l in 0..8 {
                acc[l] = cx[l].mul_add(cy[l], acc[l]);
            }
        }
    }
    acc.iter().sum()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn simd_avx2(x: &[f32], y: &[f32]) -> f32 {
    use std::arch::x86_64::*;
    // Four independent accumulators hide the FMA latency.
    let mut a0 = _mm256_setzero_ps();
    let mut a1 = _mm256_setzero_ps();
    let mut a2 = _mm256_setzero_ps();
    let mut a3 = _mm256_setzero_ps();
    for _ in 0..SIMD_REPS {
        let mut i = 0;
        while i + 32 <= x.len() {
            let px = x.as_ptr().add(i);
            let py = y.as_ptr().add(i);
            a0 = _mm256_fmadd_ps(_mm256_loadu_ps(px), _mm256_loadu_ps(py), a0);
            a1 = _mm256_fmadd_ps(_mm256_loadu_ps(px.add(8)), _mm256_loadu_ps(py.add(8)), a1);
            a2 = _mm256_fmadd_ps(_mm256_loadu_ps(px.add(16)), _mm256_loadu_ps(py.add(16)), a2);
            a3 = _mm256_fmadd_ps(_mm256_loadu_ps(px.add(24)), _mm256_loadu_ps(py.add(24)), a3);
            i += 32;
        }
    }
    let s = _mm256_add_ps(_mm256_add_ps(a0, a1), _mm256_add_ps(a2, a3));
    let mut out = [0f32; 8];
    _mm256_storeu_ps(out.as_mut_ptr(), s);
    out.iter().sum()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn simd_avx512(x: &[f32], y: &[f32]) -> f32 {
    use std::arch::x86_64::*;
    let mut a0 = _mm512_setzero_ps();
    let mut a1 = _mm512_setzero_ps();
    for _ in 0..SIMD_REPS {
        let mut i = 0;
        while i + 32 <= x.len() {
            let px = x.as_ptr().add(i);
            let py = y.as_ptr().add(i);
            a0 = _mm512_fmadd_ps(_mm512_loadu_ps(px), _mm512_loadu_ps(py), a0);
            a1 = _mm512_fmadd_ps(_mm512_loadu_ps(px.add(16)), _mm512_loadu_ps(py.add(16)), a1);
            i += 32;
        }
    }
    _mm512_reduce_add_ps(_mm512_add_ps(a0, a1))
}

#[derive(Clone, Copy, PartialEq)]
enum SimdPath {
    Avx512,
    Avx2,
    None,
}

fn simd_path() -> SimdPath {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx512f") {
            return SimdPath::Avx512;
        }
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma") {
            return SimdPath::Avx2;
        }
    }
    SimdPath::None
}

/// One SIMD round. Returns (checksum bits, flops).
fn simd_round(path: SimdPath, x: &[f32], y: &[f32]) -> (u64, u64) {
    let v = match path {
        #[cfg(target_arch = "x86_64")]
        SimdPath::Avx512 => unsafe { simd_avx512(x, y) },
        #[cfg(target_arch = "x86_64")]
        SimdPath::Avx2 => unsafe { simd_avx2(x, y) },
        _ => simd_scalar(x, y),
    };
    (v.to_bits() as u64, 2 * (SIMD_LEN * SIMD_REPS) as u64)
}

/// Per-thread working memory, allocated once.
struct Work {
    data: Vec<u64>,
    keys: Vec<u32>,
    tmp: Vec<u32>,
    a: Vec<f64>,
    b: Vec<f64>,
    c: Vec<f64>,
    x: Vec<f32>,
    y: Vec<f32>,
}

impl Work {
    fn new() -> Self {
        Self {
            data: vec![0; 32 * 1024],
            keys: vec![0; 32 * 1024],
            tmp: vec![0; 32 * 1024],
            a: vec![0.0; N * N],
            b: vec![0.0; N * N],
            c: vec![0.0; N * N],
            x: vec![0.0; SIMD_LEN],
            y: vec![0.0; SIMD_LEN],
        }
    }
    /// One round with a fixed seed per round index: identical on every thread.
    fn round(&mut self, kind: CpuKind, path: SimdPath, i: u64) -> (u64, u64) {
        let seed = 0x5241_4D46_4F52_4745 ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        match kind {
            CpuKind::Integer => integer_round(seed, &mut self.data, &mut self.keys, &mut self.tmp),
            CpuKind::Float => float_round(seed, &mut self.a, &mut self.b, &mut self.c),
            CpuKind::Simd => {
                simd_fill(seed, &mut self.x, &mut self.y);
                simd_round(path, &self.x, &self.y)
            }
        }
    }
}

/// Distinct round seeds; the checksums of these rounds are the reference.
const SEEDS: u64 = 8;

/// Runs `kind` on `threads` threads for `secs`. Returns (work per second, errors).
fn measure(kind: CpuKind, path: SimdPath, threads: usize, secs: f64, reference: &[u64], cancel: &AtomicBool) -> (f64, u64) {
    let barrier = Barrier::new(threads);
    let results: Vec<(u64, u64, f64)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let barrier = &barrier;
                s.spawn(move || {
                    let mut w = Work::new();
                    let _ = w.round(kind, path, 0); // warm up caches
                    barrier.wait();
                    let t0 = Instant::now();
                    let (mut work, mut errors, mut i) = (0u64, 0u64, 0u64);
                    while t0.elapsed().as_secs_f64() < secs && !cancel.load(Ordering::Relaxed) {
                        let (sum, amount) = w.round(kind, path, i % SEEDS);
                        if sum != reference[(i % SEEDS) as usize] {
                            errors += 1;
                        }
                        work += amount;
                        i += 1;
                    }
                    (black_box(work), errors, t0.elapsed().as_secs_f64())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or((0, 1, 1.0))).collect()
    });
    let total: f64 = results.iter().map(|r| r.0 as f64 / r.2.max(1e-9)).sum();
    (total, results.iter().map(|r| r.1).sum())
}

/// Runs one CPU benchmark, single-thread then multi-thread, `secs` each.
pub fn run_one(kind: CpuKind, threads: usize, secs: f64, cancel: &AtomicBool) -> CpuScore {
    let path = if kind == CpuKind::Simd { simd_path() } else { SimdPath::None };
    let variant = match (kind, path) {
        (CpuKind::Simd, SimdPath::Avx512) => "AVX-512F",
        (CpuKind::Simd, SimdPath::Avx2) => "AVX2+FMA",
        (CpuKind::Simd, SimdPath::None) => "skalar",
        _ => "skalar",
    };
    let mut score = CpuScore {
        kind,
        variant: variant.into(),
        single_raw: 0.0,
        multi_raw: 0.0,
        single_points: 0,
        multi_points: 0,
        threads: threads.max(1),
        scaling: 0.0,
        errors: 0,
        skipped: None,
    };
    if kind == CpuKind::Simd && path == SimdPath::None {
        score.skipped = Some("Kein AVX2/FMA: SIMD-Test übersprungen".into());
        return score;
    }
    // Reference checksums from one single-threaded pass.
    let mut w = Work::new();
    let reference: Vec<u64> = (0..SEEDS).map(|i| w.round(kind, path, i).0).collect();
    let scale = match kind {
        CpuKind::Integer => 1e-6, // bytes -> MB
        _ => 1e-9,                // flops -> GFLOPS
    };
    let (st, e1) = measure(kind, path, 1, secs, &reference, cancel);
    let (mt, e2) = measure(kind, path, score.threads, secs, &reference, cancel);
    score.single_raw = st * scale;
    score.multi_raw = mt * scale;
    score.single_points = points(kind, score.single_raw);
    score.multi_points = points(kind, score.multi_raw);
    score.scaling = if st > 0.0 { mt / st } else { 0.0 };
    score.errors = e1 + e2;
    score
}

/// Runs all CPU benchmarks. `progress(fraction, step)` before each step.
pub fn run_all(threads: usize, secs: f64, cancel: &AtomicBool, progress: &dyn Fn(f32, &str)) -> Vec<CpuScore> {
    let mut out = Vec::new();
    for (i, k) in CpuKind::ALL.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(i as f32 / CpuKind::ALL.len() as f32, k.name());
        out.push(run_one(*k, threads, secs, cancel));
    }
    progress(1.0, "Fertig");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workloads_are_deterministic() {
        let mut w1 = Work::new();
        let mut w2 = Work::new();
        for k in CpuKind::ALL {
            let p = if k == CpuKind::Simd { simd_path() } else { SimdPath::None };
            for i in 0..3 {
                assert_eq!(w1.round(k, p, i).0, w2.round(k, p, i).0, "{k:?} Runde {i}");
            }
            assert_ne!(w1.round(k, p, 0).0, w1.round(k, p, 1).0, "{k:?}: Runden müssen sich unterscheiden");
        }
    }

    #[test]
    fn radix_sort_sorts() {
        let mut w = Work::new();
        let _ = integer_round(7, &mut w.data, &mut w.keys, &mut w.tmp);
        assert!(w.keys.windows(2).all(|p| p[0] <= p[1]));
    }

    #[test]
    fn matrix_multiply_matches_naive() {
        let mut w = Work::new();
        let _ = float_round(3, &mut w.a, &mut w.b, &mut w.c);
        // Spot-check a few entries against a straightforward dot product.
        for &(i, j) in &[(0, 0), (5, 77), (127, 127)] {
            let naive: f64 = (0..N).map(|k| w.a[i * N + k] * w.b[k * N + j]).sum();
            assert!((naive - w.c[i * N + j]).abs() < 1e-9);
        }
    }

    #[test]
    fn simd_matches_scalar_closely() {
        let mut w = Work::new();
        simd_fill(11, &mut w.x, &mut w.y);
        let s = simd_scalar(&w.x, &w.y);
        let v = f32::from_bits(simd_round(simd_path(), &w.x, &w.y).0 as u32);
        assert!((s - v).abs() <= 1e-2 * s.abs().max(1.0), "skalar {s} vs simd {v}");
    }

    #[test]
    fn short_run_has_no_errors_and_scales() {
        let cancel = AtomicBool::new(false);
        let s = run_one(CpuKind::Integer, 2, 0.3, &cancel);
        assert_eq!(s.errors, 0);
        assert!(s.single_raw > 0.0 && s.multi_raw > 0.0);
        assert!(s.scaling > 1.2, "2 Threads sollten schneller sein: {}", s.scaling);
    }
}
