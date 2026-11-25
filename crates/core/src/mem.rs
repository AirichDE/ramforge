//! Allocation of large, page-locked test buffers and physical memory status.
//!
//! Locking keeps the operating system from paging the buffer out while a test
//! runs. If the pages were swapped to disk we would be testing the SSD, not the RAM.

use std::ptr::NonNull;

/// Physical memory status of the machine.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct MemStatus {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

impl MemStatus {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }
    pub fn load_percent(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            self.used_bytes() as f64 * 100.0 / self.total_bytes as f64
        }
    }
}

/// A page-aligned buffer of `u64` words owned by one test thread.
pub struct TestBuffer {
    ptr: NonNull<u64>,
    bytes: usize,
    locked: bool,
}

// The buffer is plain memory; ownership moves to exactly one worker thread.
unsafe impl Send for TestBuffer {}

pub const PAGE: usize = 4096;

impl TestBuffer {
    /// Allocates `bytes` (rounded down to whole pages) and optionally locks it in RAM.
    /// Locking failure is not fatal: `is_locked()` reports the outcome.
    pub fn allocate(bytes: usize, lock: bool) -> Result<Self, String> {
        let bytes = bytes / PAGE * PAGE;
        if bytes == 0 {
            return Err("Puffergröße ist 0".into());
        }
        let ptr = sys::alloc(bytes).ok_or_else(|| {
            format!("Speicher konnte nicht reserviert werden ({} MiB)", bytes >> 20)
        })?;
        let locked = lock && sys::lock(ptr, bytes);
        Ok(Self {
            ptr: NonNull::new(ptr as *mut u64).expect("non-null"),
            bytes,
            locked,
        })
    }

    pub fn as_mut_slice(&mut self) -> &mut [u64] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.bytes / 8) }
    }
    pub fn len_bytes(&self) -> usize {
        self.bytes
    }
    pub fn is_locked(&self) -> bool {
        self.locked
    }
    pub fn base_addr(&self) -> usize {
        self.ptr.as_ptr() as usize
    }
}

impl Drop for TestBuffer {
    fn drop(&mut self) {
        unsafe { sys::free(self.ptr.as_ptr() as *mut u8, self.bytes, self.locked) }
    }
}

/// Raises the process working-set quota so that `total_bytes` can be locked.
/// Must be called once before allocating locked buffers.
pub fn prepare_lock(total_bytes: u64) -> bool {
    sys::prepare_lock(total_bytes)
}

pub fn mem_status() -> MemStatus {
    sys::mem_status()
}

#[cfg(windows)]
mod sys {
    use super::MemStatus;
    use windows_sys::Win32::System::Memory::*;
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};

    pub fn alloc(bytes: usize) -> Option<*mut u8> {
        let p = unsafe {
            VirtualAlloc(std::ptr::null(), bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE)
        };
        (!p.is_null()).then_some(p as *mut u8)
    }
    pub fn lock(p: *mut u8, bytes: usize) -> bool {
        unsafe { VirtualLock(p as *const _, bytes) != 0 }
    }
    pub unsafe fn free(p: *mut u8, bytes: usize, locked: bool) {
        if locked {
            VirtualUnlock(p as *const _, bytes);
        }
        VirtualFree(p as *mut _, 0, MEM_RELEASE);
    }
    pub fn prepare_lock(total: u64) -> bool {
        let min = total as usize + (128 << 20);
        let max = total as usize + (512 << 20);
        unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), min, max) != 0 }
    }
    pub fn mem_status() -> MemStatus {
        let mut s: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut s) } == 0 {
            return MemStatus::default();
        }
        MemStatus { total_bytes: s.ullTotalPhys, available_bytes: s.ullAvailPhys }
    }
}

#[cfg(unix)]
mod sys {
    use super::MemStatus;

    pub fn alloc(bytes: usize) -> Option<*mut u8> {
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        (p != libc::MAP_FAILED).then_some(p as *mut u8)
    }
    pub fn lock(p: *mut u8, bytes: usize) -> bool {
        unsafe { libc::mlock(p as *const _, bytes) == 0 }
    }
    pub unsafe fn free(p: *mut u8, bytes: usize, locked: bool) {
        if locked {
            libc::munlock(p as *const _, bytes);
        }
        libc::munmap(p as *mut _, bytes);
    }
    pub fn prepare_lock(_total: u64) -> bool {
        // On the live system we run as root; RLIMIT_MEMLOCK is raised by the launcher.
        let lim = libc::rlimit { rlim_cur: libc::RLIM_INFINITY, rlim_max: libc::RLIM_INFINITY };
        unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &lim) == 0 }
    }
    pub fn mem_status() -> MemStatus {
        let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let field = |name: &str| -> u64 {
            text.lines()
                .find(|l| l.starts_with(name))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
                * 1024
        };
        MemStatus { total_bytes: field("MemTotal:"), available_bytes: field("MemAvailable:") }
    }
}
