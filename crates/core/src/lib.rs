//! RAMForge core: memory test algorithms, test runner and benchmark.
//! Platform independent; used by the Windows EXE and the boot (Linux live) edition.

pub mod algo;
pub mod bench;
pub mod cpubench;
pub mod errors;
pub mod mem;
pub mod rng;
pub mod runner;
pub mod stress;

pub use algo::TestKind;
pub use errors::{ErrorSink, MemError};
pub use runner::{Outcome, Preset, RunStatus, RunSummary, TestConfig};
