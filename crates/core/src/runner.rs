//! Multi-threaded test runner.
//!
//! Every worker thread owns one locked buffer. All threads run the same test at
//! the same time (synchronised with barriers), like boot-time memory testers do,
//! so the memory controller is loaded by all cores together.

use crate::algo::{TestCtx, TestKind};
use crate::errors::ErrorSink;
use crate::mem::{self, TestBuffer};
use crate::rng::{splitmix, time_seed};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestConfig {
    pub tests: Vec<TestKind>,
    /// 0 = run until stopped.
    pub passes: u32,
    pub threads: usize,
    pub total_bytes: u64,
    pub lock_memory: bool,
    pub bitfade_secs: u64,
    pub stop_on_error: bool,
    /// Self test: inject one simulated bit flip to prove error reporting works.
    pub inject_fault: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    Quick,
    Standard,
    Extreme,
    BitFade,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Quick, Preset::Standard, Preset::Extreme, Preset::BitFade];
    pub fn name(self) -> &'static str {
        match self {
            Preset::Quick => "Schnell",
            Preset::Standard => "Standard",
            Preset::Extreme => "Extrem",
            Preset::BitFade => "Bit Fade (Refresh)",
        }
    }
    pub fn tests(self) -> Vec<TestKind> {
        use TestKind::*;
        match self {
            Preset::Quick => vec![AddressOwn, MovInvOnesZeros, RandomSequence, RandomAccess],
            Preset::Standard => vec![
                AddressOwn, MovInvOnesZeros, MovInvWalking8, RandomSequence, RandomAccess,
                BlockMove, WalkingBits, MovInvRandom,
            ],
            Preset::Extreme => vec![
                AddressOwn, MovInvOnesZeros, MovInvWalking8, MovInvRandom, RandomSequence,
                RandomAccess, BlockMove, WalkingBits, Modulo20, Hammer, RandomAccess,
            ],
            Preset::BitFade => vec![BitFade],
        }
    }
    pub fn passes(self) -> u32 {
        match self {
            Preset::Quick => 1,
            Preset::Standard => 2,
            Preset::Extreme => 4,
            Preset::BitFade => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Passed,
    Failed,
    Cancelled,
    Aborted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunSummary {
    pub outcome: Outcome,
    pub message: String,
    pub started: chrono::DateTime<chrono::Local>,
    pub duration_secs: f64,
    pub passes_completed: u32,
    pub tests_completed: u64,
    pub errors: u64,
    pub tested_bytes: u64,
    pub threads: usize,
    pub memory_locked: bool,
    pub config: TestConfig,
}

/// Live state shared between the runner and the GUI.
pub struct RunStatus {
    pub cancel: AtomicBool,
    pub running: AtomicBool,
    pub pass: AtomicU32,
    pub test_index: AtomicUsize,
    pub tests_completed: AtomicU64,
    progress: Vec<AtomicU64>,
    test_total_words: AtomicU64,
    pub sink: ErrorSink,
    pub started: Instant,
    pub started_wall: chrono::DateTime<chrono::Local>,
    pub allocated_bytes: AtomicU64,
    pub all_locked: AtomicBool,
    pub phase: Mutex<String>,
    /// Every test start as (pass, test index), in order. Lets the GUI mark
    /// phases that are shorter than one screen refresh.
    pub phase_log: Mutex<Vec<(u32, usize)>>,
    pub summary: Mutex<Option<RunSummary>>,
    pub config: TestConfig,
}

impl RunStatus {
    /// Progress of the current test, 0..1.
    pub fn test_fraction(&self) -> f64 {
        let total = self.test_total_words.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let done: u64 = self.progress.iter().map(|p| p.load(Ordering::Relaxed)).sum();
        (done as f64 / total as f64).min(1.0)
    }
    /// Progress of the current pass, 0..1.
    pub fn pass_fraction(&self) -> f64 {
        let n = self.config.tests.len().max(1) as f64;
        (self.test_index.load(Ordering::Relaxed) as f64 + self.test_fraction()) / n
    }
    pub fn current_test(&self) -> Option<TestKind> {
        self.config.tests.get(self.test_index.load(Ordering::Relaxed)).copied()
    }
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    fn set_phase(&self, s: impl Into<String>) {
        *self.phase.lock().unwrap() = s.into();
    }
}

/// Starts a test run on background threads and returns its shared status.
pub fn start(cfg: TestConfig) -> Arc<RunStatus> {
    let threads = cfg.threads.max(1);
    let status = Arc::new(RunStatus {
        cancel: AtomicBool::new(false),
        running: AtomicBool::new(true),
        pass: AtomicU32::new(0),
        test_index: AtomicUsize::new(0),
        tests_completed: AtomicU64::new(0),
        progress: (0..threads).map(|_| AtomicU64::new(0)).collect(),
        test_total_words: AtomicU64::new(0),
        sink: ErrorSink::new(10_000, cfg.inject_fault),
        started: Instant::now(),
        started_wall: chrono::Local::now(),
        allocated_bytes: AtomicU64::new(0),
        all_locked: AtomicBool::new(true),
        phase: Mutex::new("Speicher wird reserviert…".into()),
        phase_log: Mutex::new(Vec::new()),
        summary: Mutex::new(None),
        config: cfg,
    });
    let st = status.clone();
    std::thread::Builder::new()
        .name("ramforge-runner".into())
        .spawn(move || coordinate(st))
        .expect("spawn runner");
    status
}

fn coordinate(st: Arc<RunStatus>) {
    let cfg = st.config.clone();
    let threads = cfg.threads.max(1);
    let per_thread = (cfg.total_bytes / threads as u64) as usize;
    if cfg.lock_memory {
        mem::prepare_lock(cfg.total_bytes);
    }
    let barrier = Barrier::new(threads);
    let stop_now = AtomicBool::new(false);
    let alloc_failed = Mutex::new(None::<String>);
    let base_seed = time_seed();

    std::thread::scope(|s| {
        for t in 0..threads {
            let (st, barrier, stop_now, alloc_failed) = (&st, &barrier, &stop_now, &alloc_failed);
            let cfg = &cfg;
            std::thread::Builder::new()
                .name(format!("ramforge-worker-{t}"))
                .spawn_scoped(s, move || {
                    // Each thread allocates its own buffer: faster, and on NUMA
                    // systems the memory lands next to the core that tests it.
                    let mut buf = match TestBuffer::allocate(per_thread, cfg.lock_memory) {
                        Ok(b) => {
                            st.allocated_bytes.fetch_add(b.len_bytes() as u64, Ordering::Relaxed);
                            if !b.is_locked() {
                                st.all_locked.store(false, Ordering::Relaxed);
                            }
                            Some(b)
                        }
                        Err(e) => {
                            *alloc_failed.lock().unwrap() = Some(e);
                            None
                        }
                    };
                    let words_per_thread = per_thread as u64 / 8;
                    let mut pass = 0u32;
                    'passes: loop {
                        for (ti, &test) in cfg.tests.iter().enumerate() {
                            // Leader decides, between two barriers, whether to stop.
                            // All threads read the same decision: no thread is left
                            // waiting on a barrier the others never reach.
                            if barrier.wait().is_leader() {
                                let failed = alloc_failed.lock().unwrap().clone();
                                let stop = st.cancel.load(Ordering::Relaxed)
                                    || failed.is_some()
                                    || (cfg.stop_on_error && st.sink.count() > 0);
                                stop_now.store(stop, Ordering::Relaxed);
                                if let Some(e) = failed {
                                    st.set_phase(e);
                                } else if !stop {
                                    for p in &st.progress {
                                        p.store(0, Ordering::Relaxed);
                                    }
                                    st.test_total_words.store(
                                        test.sweeps() * words_per_thread * threads as u64,
                                        Ordering::Relaxed,
                                    );
                                    st.test_index.store(ti, Ordering::Relaxed);
                                    st.pass.store(pass, Ordering::Relaxed);
                                    st.set_phase(format!("Durchlauf {} · {}", pass + 1, test.name()));
                                    st.phase_log.lock().unwrap().push((pass, ti));
                                }
                            }
                            barrier.wait();
                            if stop_now.load(Ordering::Relaxed) {
                                break 'passes;
                            }
                            let b = buf.as_mut().expect("allocated");
                            let base_addr = b.base_addr();
                            let slice = b.as_mut_slice();
                            let ctx = TestCtx {
                                cancel: &st.cancel,
                                progress: &st.progress[t],
                                sink: &st.sink,
                                thread: t,
                                pass,
                                test,
                                seed: splitmix(base_seed ^ ((pass as u64) << 32) ^ (ti as u64) << 8 ^ t as u64),
                                base_addr,
                                fault_index: if t == 0 { slice.len() / 3 / 20 * 20 } else { usize::MAX },
                                bitfade_secs: cfg.bitfade_secs,
                            };
                            test.run(slice, &ctx);
                            if t == 0 && !st.cancel.load(Ordering::Relaxed) {
                                st.tests_completed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        pass += 1;
                        if t == 0 {
                            st.pass.store(pass, Ordering::Relaxed);
                        }
                        if cfg.passes != 0 && pass >= cfg.passes {
                            break;
                        }
                    }
                    drop(buf);
                })
                .expect("spawn worker");
        }
    });

    let errors = st.sink.count();
    let failed = alloc_failed.lock().unwrap().clone();
    // Only tests that ran to the end are counted, so a cancelled pass is not.
    let passes_completed =
        (st.tests_completed.load(Ordering::Relaxed) / cfg.tests.len().max(1) as u64) as u32;
    let (outcome, message) = if let Some(e) = failed {
        (Outcome::Aborted, e)
    } else if errors > 0 {
        (Outcome::Failed, format!("{errors} Fehler gefunden"))
    } else if st.cancel.load(Ordering::Relaxed) {
        (Outcome::Cancelled, "Vom Benutzer abgebrochen, bis dahin keine Fehler".into())
    } else {
        (Outcome::Passed, "Keine Fehler gefunden".into())
    };
    let summary = RunSummary {
        outcome,
        message: message.clone(),
        started: st.started_wall,
        duration_secs: st.elapsed().as_secs_f64(),
        passes_completed,
        tests_completed: st.tests_completed.load(Ordering::Relaxed),
        errors,
        tested_bytes: st.allocated_bytes.load(Ordering::Relaxed),
        threads,
        memory_locked: st.all_locked.load(Ordering::Relaxed),
        config: cfg,
    };
    st.set_phase(message);
    *st.summary.lock().unwrap() = Some(summary);
    st.running.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_completes_and_finds_injected_fault() {
        let st = start(TestConfig {
            tests: vec![TestKind::AddressOwn, TestKind::RandomSequence],
            passes: 2,
            threads: 4,
            total_bytes: 64 << 20,
            lock_memory: false,
            bitfade_secs: 1,
            stop_on_error: false,
            inject_fault: true,
        });
        while st.running.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let s = st.summary.lock().unwrap().clone().unwrap();
        assert_eq!(s.passes_completed, 2);
        assert_eq!(s.tests_completed, 4);
        assert_eq!(s.errors, 1);
        assert_eq!(s.outcome, Outcome::Failed);
        assert!(st.sink.errors()[0].simulated);
    }

    #[test]
    fn runner_can_be_cancelled() {
        let st = start(TestConfig {
            tests: Preset::Extreme.tests(),
            passes: 0,
            threads: 2,
            total_bytes: 256 << 20,
            lock_memory: false,
            bitfade_secs: 1,
            stop_on_error: false,
            inject_fault: false,
        });
        std::thread::sleep(Duration::from_millis(300));
        st.stop();
        let t = Instant::now();
        while st.running.load(Ordering::Acquire) {
            assert!(t.elapsed() < Duration::from_secs(10), "cancel too slow");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(st.summary.lock().unwrap().as_ref().unwrap().outcome, Outcome::Cancelled);
    }
}

#[cfg(test)]
mod phase_tests {
    use super::*;

    #[test]
    fn every_test_start_is_logged_in_order() {
        let tests = vec![TestKind::AddressOwn, TestKind::MovInvOnesZeros, TestKind::RandomSequence];
        let st = start(TestConfig {
            tests: tests.clone(),
            passes: 2,
            threads: 2,
            total_bytes: 16 << 20,
            lock_memory: false,
            bitfade_secs: 1,
            stop_on_error: false,
            inject_fault: false,
        });
        while st.running.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let log = st.phase_log.lock().unwrap().clone();
        assert_eq!(log, vec![(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2)]);
    }
}
