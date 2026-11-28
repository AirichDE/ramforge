//! RAMForge hardware readout: SMBIOS, CPU, SPD decoding and live sensors.

pub mod cpu;
pub mod sensors;
pub mod smbios;
pub mod spd;

use serde::{Deserialize, Serialize};

pub use ramforge_core::mem::MemStatus;

/// Everything we know about the machine, collected once at start.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub collected: chrono::DateTime<chrono::Local>,
    pub os: String,
    pub hostname: String,
    pub cpu: cpu::CpuInfo,
    pub smbios: Option<smbios::SmbiosInfo>,
    pub smbios_error: Option<String>,
    pub memory: MemStatus,
    /// Decoded SPD per module (boot edition, or loaded dump files).
    pub spd: Vec<(String, spd::SpdInfo)>,
}

pub fn collect() -> SystemInfo {
    let (smbios, smbios_error) = match smbios::read() {
        Ok(s) => (Some(s), None),
        Err(e) => (None, Some(e)),
    };
    #[allow(unused_mut)]
    let mut spd = Vec::new();
    #[cfg(target_os = "linux")]
    for (name, bytes) in spd::read_linux_eeproms() {
        if let Ok(info) = spd::decode(&bytes) {
            spd.push((name, info));
        }
    }
    SystemInfo {
        collected: chrono::Local::now(),
        os: os_name(),
        hostname: hostname(),
        cpu: cpu::read(),
        smbios,
        smbios_error,
        memory: ramforge_core::mem::mem_status(),
        spd,
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()))
        .unwrap_or_default()
}

#[cfg(windows)]
fn os_name() -> String {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let key: Vec<u16> = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<u16>>();
    let get = |name: &str| -> String {
        let n: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut buf = [0u16; 256];
        let mut size = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                n.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut _,
                &mut size,
            )
        };
        if r != 0 {
            return String::new();
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    };
    let mut product = get("ProductName");
    let build = get("CurrentBuildNumber");
    // Windows 11 still reports "Windows 10" in ProductName.
    if build.parse::<u32>().unwrap_or(0) >= 22000 {
        product = product.replace("Windows 10", "Windows 11");
    }
    format!("{product} {} (Build {build})", get("DisplayVersion"))
        .trim()
        .to_string()
}

#[cfg(not(windows))]
fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| l[12..].trim_matches('"').to_string())
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}
