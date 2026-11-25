//! CPU identification with the CPUID instruction (no driver needed).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CpuInfo {
    pub vendor: String,
    pub brand: String,
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
    pub logical_processors: usize,
    pub features: Vec<String>,
    pub hypervisor: bool,
}

pub fn read() -> CpuInfo {
    let mut info = CpuInfo {
        logical_processors: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        ..Default::default()
    };
    // CPUID is a safe intrinsic on x86_64.
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{__cpuid, __cpuid_count};
        let l0 = __cpuid(0);
        let mut v = Vec::with_capacity(12);
        v.extend_from_slice(&l0.ebx.to_le_bytes());
        v.extend_from_slice(&l0.edx.to_le_bytes());
        v.extend_from_slice(&l0.ecx.to_le_bytes());
        info.vendor = String::from_utf8_lossy(&v).to_string();

        let l1 = __cpuid(1);
        let base_family = (l1.eax >> 8) & 0xF;
        let base_model = (l1.eax >> 4) & 0xF;
        info.stepping = l1.eax & 0xF;
        info.family = if base_family == 0xF { base_family + ((l1.eax >> 20) & 0xFF) } else { base_family };
        info.model = if base_family == 0x6 || base_family == 0xF {
            base_model | (((l1.eax >> 16) & 0xF) << 4)
        } else {
            base_model
        };
        info.hypervisor = l1.ecx & (1 << 31) != 0;
        let mut f = Vec::new();
        let mut flag = |cond: bool, name: &str| {
            if cond {
                f.push(name.to_string())
            }
        };
        flag(l1.edx & (1 << 26) != 0, "SSE2");
        flag(l1.ecx & (1 << 20) != 0, "SSE4.2");
        flag(l1.ecx & (1 << 28) != 0, "AVX");
        if l0.eax >= 7 {
            let l7 = __cpuid_count(7, 0);
            flag(l7.ebx & (1 << 5) != 0, "AVX2");
            flag(l7.ebx & (1 << 16) != 0, "AVX-512F");
            flag(l7.ebx & (1 << 23) != 0, "CLFLUSHOPT");
        }
        flag(l1.edx & (1 << 19) != 0, "CLFLUSH");
        info.features = f;

        let ext = __cpuid(0x8000_0000).eax;
        if ext >= 0x8000_0004 {
            let mut b = Vec::with_capacity(48);
            for leaf in 0x8000_0002u32..=0x8000_0004 {
                let r = __cpuid(leaf);
                for x in [r.eax, r.ebx, r.ecx, r.edx] {
                    b.extend_from_slice(&x.to_le_bytes());
                }
            }
            info.brand = String::from_utf8_lossy(&b).trim_matches(char::from(0)).trim().to_string();
        }
    }
    info
}
