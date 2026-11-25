//! Memory test algorithms.
//!
//! All algorithms are clean-room implementations of well-known methods from
//! memory test literature (address-in-address, moving inversions, MATS+/March
//! style sweeps, modulo-X, block move, bit fade, row hammer). Every write and
//! read goes through volatile pointers so the compiler can not optimise the
//! memory traffic away.

use crate::errors::{ErrorSink, MemError};
use crate::rng::{splitmix, Rng};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Everything a test needs from the runner.
pub struct TestCtx<'a> {
    pub cancel: &'a AtomicBool,
    /// Words processed in the current test by this thread (for progress).
    pub progress: &'a AtomicU64,
    pub sink: &'a ErrorSink,
    pub thread: usize,
    pub pass: u32,
    pub test: TestKind,
    pub seed: u64,
    pub base_addr: usize,
    /// Index at which the self test injects one simulated bit flip.
    pub fault_index: usize,
    pub bitfade_secs: u64,
}

impl TestCtx<'_> {
    #[inline(always)]
    fn check(&self, i: usize, actual: u64, expected: u64) {
        let mut actual = actual;
        let mut simulated = false;
        if i == self.fault_index && self.sink.take_fault() {
            actual ^= 1 << 13;
            simulated = true;
        }
        if actual != expected {
            self.fail(i, actual, expected, simulated);
        }
    }

    #[cold]
    #[inline(never)]
    fn fail(&self, i: usize, actual: u64, expected: u64, simulated: bool) {
        self.sink.report(MemError {
            time: chrono::Local::now(),
            test: self.test.name().to_string(),
            pass: self.pass,
            thread: self.thread,
            address: (self.base_addr + i * 8) as u64,
            expected,
            actual,
            simulated,
        });
    }

    #[inline]
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    #[inline]
    fn tick(&self, words: usize) {
        self.progress.fetch_add(words as u64, Ordering::Relaxed);
    }
}

const CHUNK: usize = 1 << 16;

/// Runs `f(start, end)` over `0..n` in chunks, ascending. Returns false on cancel.
#[inline(always)]
fn sweep(ctx: &TestCtx, n: usize, mut f: impl FnMut(usize, usize)) -> bool {
    let mut s = 0;
    while s < n {
        if ctx.cancelled() {
            return false;
        }
        let e = (s + CHUNK).min(n);
        f(s, e);
        ctx.tick(e - s);
        s = e;
    }
    true
}

/// Like `sweep`, but chunks are visited from the top down. `f` must iterate
/// its range in reverse itself.
#[inline(always)]
fn sweep_down(ctx: &TestCtx, n: usize, mut f: impl FnMut(usize, usize)) -> bool {
    let mut e = n;
    while e > 0 {
        if ctx.cancelled() {
            return false;
        }
        let s = e.saturating_sub(CHUNK);
        f(s, e);
        ctx.tick(e - s);
        e = s;
    }
    true
}

#[inline(always)]
fn wr(p: *mut u64, i: usize, v: u64) {
    unsafe { p.add(i).write_volatile(v) }
}
#[inline(always)]
fn rd(p: *const u64, i: usize) -> u64 {
    unsafe { p.add(i).read_volatile() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TestKind {
    AddressOwn,
    MovInvOnesZeros,
    MovInvWalking8,
    MovInvRandom,
    RandomSequence,
    RandomAccess,
    BlockMove,
    WalkingBits,
    Modulo20,
    Hammer,
    BitFade,
}

impl TestKind {
    pub const ALL: [TestKind; 11] = [
        TestKind::AddressOwn,
        TestKind::MovInvOnesZeros,
        TestKind::MovInvWalking8,
        TestKind::MovInvRandom,
        TestKind::RandomSequence,
        TestKind::RandomAccess,
        TestKind::BlockMove,
        TestKind::WalkingBits,
        TestKind::Modulo20,
        TestKind::Hammer,
        TestKind::BitFade,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TestKind::AddressOwn => "Eigene Adresse",
            TestKind::MovInvOnesZeros => "Moving Inversions 0/1",
            TestKind::MovInvWalking8 => "Moving Inversions 8-Bit",
            TestKind::MovInvRandom => "Moving Inversions Zufall",
            TestKind::RandomSequence => "Zufallsfolge",
            TestKind::RandomAccess => "Zufallszugriff",
            TestKind::BlockMove => "Block verschieben",
            TestKind::WalkingBits => "Wandernde Bits",
            TestKind::Modulo20 => "Modulo 20",
            TestKind::Hammer => "Row Hammer (experimentell)",
            TestKind::BitFade => "Bit Fade",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            TestKind::AddressOwn => "Jedes Wort enthält seine eigene Adresse. Findet Adressierungsfehler.",
            TestKind::MovInvOnesZeros => "Füllt mit 0 bzw. 1, prüft und invertiert auf- und abwärts.",
            TestKind::MovInvWalking8 => "Moving Inversions mit 8 wandernden Bitmustern. Findet Kopplungsfehler.",
            TestKind::MovInvRandom => "Moving Inversions mit Zufallsmustern.",
            TestKind::RandomSequence => "Reproduzierbare Zufallsfolge schreiben, prüfen, invertieren.",
            TestKind::RandomAccess => "Schreibt in zufälliger Reihenfolge. Belastet Speichercontroller und Zeilenwechsel.",
            TestKind::BlockMove => "Kopiert große Blöcke (Burst-Transfers) und prüft das Ergebnis.",
            TestKind::WalkingBits => "Rotierende Einzelbits und Einzelnullen. Findet Datenleitungsfehler.",
            TestKind::Modulo20 => "Muster an jeder 20. Stelle, Störmuster dazwischen. Findet Nachbareffekte.",
            TestKind::Hammer => "Liest Adresspaare sehr oft mit Cache-Flush. Kann Bitkipper in Nachbarzeilen auslösen.",
            TestKind::BitFade => "Füllt, wartet und prüft. Findet Zellen, die ihre Ladung verlieren (Refresh).",
        }
    }

    /// Number of full-buffer sweeps, used for progress estimation.
    pub fn sweeps(self) -> u64 {
        match self {
            TestKind::AddressOwn => 4,
            TestKind::MovInvOnesZeros => 6,
            TestKind::MovInvWalking8 => 24,
            TestKind::MovInvRandom => 6,
            TestKind::RandomSequence => 3,
            TestKind::RandomAccess => 4,
            TestKind::BlockMove => 3,
            TestKind::WalkingBits => 16,
            TestKind::Modulo20 => 60,
            TestKind::Hammer => 6,
            TestKind::BitFade => 6,
        }
    }

    pub fn run(self, buf: &mut [u64], ctx: &TestCtx) {
        let p = buf.as_mut_ptr();
        let n = buf.len();
        match self {
            TestKind::AddressOwn => address_own(p, n, ctx),
            TestKind::MovInvOnesZeros => {
                let _ = movinv(p, n, ctx, 0) && movinv(p, n, ctx, !0);
            }
            TestKind::MovInvWalking8 => {
                for k in 0..8 {
                    if !movinv(p, n, ctx, 0x0101_0101_0101_0101u64 << k) {
                        return;
                    }
                }
            }
            TestKind::MovInvRandom => {
                let mut r = Rng::new(ctx.seed);
                let _ = movinv(p, n, ctx, r.next()) && movinv(p, n, ctx, r.next());
            }
            TestKind::RandomSequence => random_sequence(p, n, ctx),
            TestKind::RandomAccess => random_access(p, n, ctx),
            TestKind::BlockMove => block_move(p, n, ctx),
            TestKind::WalkingBits => walking_bits(p, n, ctx),
            TestKind::Modulo20 => modulo_x(p, n, ctx, 20),
            TestKind::Hammer => hammer(p, n, ctx),
            TestKind::BitFade => bit_fade(p, n, ctx),
        }
    }
}

fn address_own(p: *mut u64, n: usize, ctx: &TestCtx) {
    let base = ctx.base_addr as u64;
    for inv in [0u64, !0] {
        if !sweep(ctx, n, |s, e| {
            for i in s..e {
                wr(p, i, (base + 8 * i as u64) ^ inv)
            }
        }) {
            return;
        }
        if !sweep(ctx, n, |s, e| {
            for i in s..e {
                ctx.check(i, rd(p, i), (base + 8 * i as u64) ^ inv)
            }
        }) {
            return;
        }
    }
}

/// Moving inversions: fill, ascending verify+invert, descending verify+restore.
fn movinv(p: *mut u64, n: usize, ctx: &TestCtx, pat: u64) -> bool {
    let inv = !pat;
    sweep(ctx, n, |s, e| {
        for i in s..e {
            wr(p, i, pat)
        }
    }) && sweep(ctx, n, |s, e| {
        for i in s..e {
            ctx.check(i, rd(p, i), pat);
            wr(p, i, inv);
        }
    }) && sweep_down(ctx, n, |s, e| {
        for i in (s..e).rev() {
            ctx.check(i, rd(p, i), inv);
            wr(p, i, pat);
        }
    })
}

fn random_sequence(p: *mut u64, n: usize, ctx: &TestCtx) {
    let mut r = Rng::new(ctx.seed);
    if !sweep(ctx, n, |s, e| {
        for i in s..e {
            wr(p, i, r.next())
        }
    }) {
        return;
    }
    let mut r = Rng::new(ctx.seed);
    if !sweep(ctx, n, |s, e| {
        for i in s..e {
            let v = r.next();
            ctx.check(i, rd(p, i), v);
            wr(p, i, !v);
        }
    }) {
        return;
    }
    let mut r = Rng::new(ctx.seed);
    sweep(ctx, n, |s, e| {
        for i in s..e {
            ctx.check(i, rd(p, i), !r.next())
        }
    });
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Writes every word exactly once in a scattered order (i -> i*step mod n with
/// gcd(step, n) = 1 is a bijection), then verifies sequentially.
fn random_access(p: *mut u64, n: usize, ctx: &TestCtx) {
    if n < 2 {
        return;
    }
    let nn = n as u64;
    let mut step = (0x9E37_79B9_7F4A_7C15u64 ^ ctx.seed) % nn;
    if step == 0 {
        step = 1;
    }
    while gcd(step, nn) != 1 {
        step = (step + 1) % nn;
        if step == 0 {
            step = 1;
        }
    }
    for round in 0..2u64 {
        let key = ctx.seed ^ round.wrapping_mul(0xA5A5_A5A5_5A5A_5A5A);
        let mut idx = ctx.seed % nn;
        if !sweep(ctx, n, |s, e| {
            for _ in s..e {
                let i = idx as usize;
                wr(p, i, splitmix(idx ^ key));
                idx += step;
                if idx >= nn {
                    idx -= nn;
                }
            }
        }) {
            return;
        }
        if !sweep(ctx, n, |s, e| {
            for i in s..e {
                ctx.check(i, rd(p, i), splitmix(i as u64 ^ key))
            }
        }) {
            return;
        }
    }
}

fn block_move(p: *mut u64, n: usize, ctx: &TestCtx) {
    let h = n / 2;
    if h < 1024 {
        return;
    }
    let seed = ctx.seed;
    let f = move |i: usize| (0x0123_4567_89AB_CDEFu64 ^ seed).rotate_left((i % 64) as u32);
    let (a, b) = (p, unsafe { p.add(h) });
    // Fill A, copy A -> B in large bursts, verify B.
    let ok = sweep(ctx, h, |s, e| {
        for i in s..e {
            wr(a, i, f(i))
        }
    }) && sweep(ctx, h, |s, e| unsafe {
        std::ptr::copy_nonoverlapping(a.add(s), b.add(s), e - s)
    }) && sweep(ctx, h, |s, e| {
        for i in s..e {
            ctx.check(h + i, rd(b, i), f(i))
        }
    });
    if !ok {
        return;
    }
    // Fill B with the inverse, copy B -> A shifted by `sh` words, verify A.
    let sh = 1021usize % h;
    let _ = sweep(ctx, h, |s, e| {
        for i in s..e {
            wr(b, i, !f(i))
        }
    }) && sweep(ctx, h, |s, e| {
        for i in s..e {
            wr(a, (i + sh) % h, rd(b, i))
        }
    }) && sweep(ctx, h, |s, e| {
        for i in s..e {
            ctx.check((i + sh) % h, rd(a, (i + sh) % h), !f(i))
        }
    });
}

fn walking_bits(p: *mut u64, n: usize, ctx: &TestCtx) {
    for k in 0..4usize {
        for inv in [0u64, !0] {
            let pat = |i: usize| (1u64 << ((i + k * 16) % 64)) ^ inv;
            if !sweep(ctx, n, |s, e| {
                for i in s..e {
                    wr(p, i, pat(i))
                }
            }) {
                return;
            }
            if !sweep(ctx, n, |s, e| {
                for i in s..e {
                    ctx.check(i, rd(p, i), pat(i))
                }
            }) {
                return;
            }
        }
    }
}

fn modulo_x(p: *mut u64, n: usize, ctx: &TestCtx, x: usize) {
    let mut r = Rng::new(ctx.seed);
    for off in 0..x {
        let pat = r.next();
        let first = |s: usize| s + (off + x - s % x) % x;
        let ok = sweep(ctx, n, |s, e| {
            let mut i = first(s);
            while i < e {
                wr(p, i, pat);
                i += x;
            }
        }) && sweep(ctx, n, |s, e| {
            for i in s..e {
                if i % x != off {
                    wr(p, i, !pat)
                }
            }
        }) && sweep(ctx, n, |s, e| {
            let mut i = first(s);
            while i < e {
                ctx.check(i, rd(p, i), pat);
                i += x;
            }
        });
        if !ok {
            return;
        }
    }
}

#[inline(always)]
fn flush(_p: *const u64) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_clflush(_p as *const u8)
    }
}

/// Experimental row hammer: without physical address mapping we pick random
/// address pairs; this finds weak modules only by chance and is no substitute
/// for the address-aware variant planned for the boot edition.
fn hammer(p: *mut u64, n: usize, ctx: &TestCtx) {
    const ITERS: usize = 200_000;
    let window = n.min(8 << 20); // 64 MiB window
    let pairs = (n >> 20).clamp(8, 128);
    for pat in [0x5555_5555_5555_5555u64, 0xAAAA_AAAA_AAAA_AAAA] {
        if !sweep(ctx, n, |s, e| {
            for i in s..e {
                wr(p, i, pat)
            }
        }) {
            return;
        }
        let mut r = Rng::new(ctx.seed ^ pat);
        for _ in 0..pairs {
            if ctx.cancelled() {
                return;
            }
            let start = r.below((n - window + 1) as u64) as usize;
            let a = start + r.below(window as u64) as usize;
            let b = start + r.below(window as u64) as usize;
            let (pa, pb) = unsafe { (p.add(a) as *const u64, p.add(b) as *const u64) };
            for _ in 0..ITERS {
                unsafe {
                    pa.read_volatile();
                    pb.read_volatile();
                }
                flush(pa);
                flush(pb);
            }
            ctx.tick(n / pairs);
        }
        if !sweep(ctx, n, |s, e| {
            for i in s..e {
                ctx.check(i, rd(p, i), pat)
            }
        }) {
            return;
        }
    }
}

fn bit_fade(p: *mut u64, n: usize, ctx: &TestCtx) {
    for pat in [0u64, !0] {
        if !sweep(ctx, n, |s, e| {
            for i in s..e {
                wr(p, i, pat)
            }
        }) {
            return;
        }
        // Wait without touching the memory, so only DRAM refresh keeps the data.
        let secs = ctx.bitfade_secs.max(1);
        for _ in 0..secs {
            if ctx.cancelled() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            ctx.tick(n / secs as usize);
        }
        if !sweep(ctx, n, |s, e| {
            for i in s..e {
                ctx.check(i, rd(p, i), pat)
            }
        }) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_one(kind: TestKind, words: usize, fault: Option<usize>) -> u64 {
        let mut buf = vec![0u64; words];
        let cancel = AtomicBool::new(false);
        let progress = AtomicU64::new(0);
        let sink = ErrorSink::new(100, fault.is_some());
        let ctx = TestCtx {
            cancel: &cancel,
            progress: &progress,
            sink: &sink,
            thread: 0,
            pass: 0,
            test: kind,
            seed: 42,
            base_addr: buf.as_ptr() as usize,
            fault_index: fault.unwrap_or(usize::MAX),
            bitfade_secs: 1,
        };
        kind.run(&mut buf, &ctx);
        sink.count()
    }

    #[test]
    fn healthy_memory_has_no_errors() {
        for k in TestKind::ALL {
            assert_eq!(run_one(k, 300_001, None), 0, "{k:?}");
        }
    }

    #[test]
    fn simulated_fault_is_detected() {
        for k in TestKind::ALL {
            // Modulo-X only reads every 20th word; index 20*k is always checked.
            assert_eq!(run_one(k, 300_001, Some(4000)), 1, "{k:?}");
        }
    }
}
