//! Collection of detected memory errors.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// One mismatch between the value written and the value read back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemError {
    pub time: DateTime<Local>,
    pub test: String,
    pub pass: u32,
    pub thread: usize,
    /// Virtual address. Physical addresses are only available in the boot variant.
    pub address: u64,
    pub expected: u64,
    pub actual: u64,
    /// True if this error was injected by the self test, not real.
    pub simulated: bool,
}

impl MemError {
    pub fn xor(&self) -> u64 {
        self.expected ^ self.actual
    }
    pub fn flipped_bits(&self) -> u32 {
        self.xor().count_ones()
    }
}

/// Thread-safe error sink shared by all worker threads.
pub struct ErrorSink {
    count: AtomicU64,
    list: Mutex<Vec<MemError>>,
    max_keep: usize,
    /// How often each of the 64 data bits was wrong. A single bit that keeps
    /// failing points to one DRAM chip or data lane.
    bit_hist: [AtomicU64; 64],
    fault_armed: AtomicBool,
}

impl ErrorSink {
    pub fn new(max_keep: usize, inject_fault: bool) -> Self {
        Self {
            count: AtomicU64::new(0),
            list: Mutex::new(Vec::new()),
            max_keep,
            bit_hist: std::array::from_fn(|_| AtomicU64::new(0)),
            fault_armed: AtomicBool::new(inject_fault),
        }
    }

    pub fn report(&self, mut e: MemError) {
        self.count.fetch_add(1, Ordering::Relaxed);
        let x = e.xor();
        for b in 0..64 {
            if x & (1 << b) != 0 {
                self.bit_hist[b].fetch_add(1, Ordering::Relaxed);
            }
        }
        e.time = Local::now();
        let mut l = self.list.lock().unwrap();
        if l.len() < self.max_keep {
            l.push(e);
        }
    }

    /// Returns true exactly once if a simulated fault is still pending.
    #[inline]
    pub fn take_fault(&self) -> bool {
        self.fault_armed.load(Ordering::Relaxed) && self.fault_armed.swap(false, Ordering::AcqRel)
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }
    pub fn errors(&self) -> Vec<MemError> {
        self.list.lock().unwrap().clone()
    }
    pub fn errors_since(&self, start: usize) -> Vec<MemError> {
        let l = self.list.lock().unwrap();
        l.get(start..).map(|s| s.to_vec()).unwrap_or_default()
    }
    pub fn bit_histogram(&self) -> [u64; 64] {
        std::array::from_fn(|i| self.bit_hist[i].load(Ordering::Relaxed))
    }
}
