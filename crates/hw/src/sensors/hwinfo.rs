//! Reader for HWiNFO's shared memory interface.
//!
//! HWiNFO publishes its sensor readings through a documented shared memory
//! block, intended for third-party tools (overlays, Rainmeter, …). The user has
//! to enable "Shared Memory Support" in HWiNFO's settings. Free versions of
//! HWiNFO may limit how long it stays enabled; that is HWiNFO's licensing and
//! out of our control. We only read the values.

use super::{classify_device, Reading, SensorKind, SensorSource};
#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ,
    MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS,
};

const SIGNATURE_ACTIVE: u32 = 0x5369_5748; // "HWiS" little endian
const NAME: &str = "Global\\HWiNFO_SENS_SM2";

// Field offsets of the packed structures (pack(1)).
const HDR_SIZE: usize = 44;
const SENSOR_NAME_ORIG: usize = 8;
const SENSOR_NAME_USER: usize = 136;
const READING_TYPE: usize = 0;
const READING_SENSOR_INDEX: usize = 4;
const READING_ID: usize = 8;
const READING_LABEL_ORIG: usize = 12;
const READING_LABEL_USER: usize = 140;
const READING_UNIT: usize = 268;
const READING_VALUE: usize = 284;
const READING_MIN_SIZE: usize = 316;

pub struct HwinfoSource {
    view: Option<(MEMORY_MAPPED_VIEW_ADDRESS, usize)>,
    handle: Option<windows_sys::Win32::Foundation::HANDLE>,
    status: String,
    retry_in: u32,
    last_count: usize,
}

unsafe impl Send for HwinfoSource {}

impl HwinfoSource {
    pub fn new() -> Self {
        Self {
            view: None,
            handle: None,
            status: t!("nicht verbunden", "not connected").into(),
            retry_in: 0,
            last_count: 0,
        }
    }

    fn open(&mut self) {
        let name: Vec<u16> = NAME.encode_utf16().chain(Some(0)).collect();
        let h = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        if h.is_null() {
            self.status = t!(
                "HWiNFO läuft nicht oder „Shared Memory Support“ ist aus",
                "HWiNFO is not running or “Shared Memory Support” is off"
            )
            .into();
            return;
        }
        let v = unsafe { MapViewOfFile(h, FILE_MAP_READ, 0, 0, 0) };
        if v.Value.is_null() {
            unsafe { CloseHandle(h) };
            self.status = t!(
                "Shared Memory konnte nicht gelesen werden",
                "shared memory could not be read"
            )
            .into();
            return;
        }
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        unsafe {
            VirtualQuery(
                v.Value,
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        self.view = Some((v, mbi.RegionSize));
        self.handle = Some(h);
    }

    fn close(&mut self) {
        if let Some((v, _)) = self.view.take() {
            unsafe { UnmapViewOfFile(v) };
        }
        if let Some(h) = self.handle.take() {
            unsafe { CloseHandle(h) };
        }
    }
}

impl Drop for HwinfoSource {
    fn drop(&mut self) {
        self.close();
    }
}

fn rd_u32(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .unwrap_or(0)
}
fn rd_f64(b: &[u8], o: usize) -> f64 {
    b.get(o..o + 8)
        .map(|s| f64::from_le_bytes(s.try_into().unwrap()))
        .unwrap_or(f64::NAN)
}
/// HWiNFO uses the ANSI code page; Latin-1 decoding keeps "°C" intact.
fn rd_str(b: &[u8], o: usize, len: usize) -> String {
    let Some(s) = b.get(o..o + len) else {
        return String::new();
    };
    let end = s.iter().position(|&c| c == 0).unwrap_or(len);
    s[..end]
        .iter()
        .map(|&c| c as char)
        .collect::<String>()
        .trim()
        .to_string()
}

impl SensorSource for HwinfoSource {
    fn name(&self) -> &str {
        "HWiNFO (Shared Memory)"
    }
    fn status(&self) -> String {
        self.status.clone()
    }

    fn poll(&mut self, out: &mut Vec<Reading>) {
        if self.view.is_none() {
            if self.retry_in > 0 {
                self.retry_in -= 1;
                return;
            }
            self.open();
            if self.view.is_none() {
                self.retry_in = 5;
                return;
            }
        }
        let (v, size) = self.view.unwrap();
        let mem = unsafe { std::slice::from_raw_parts(v.Value as *const u8, size) };
        if mem.len() < HDR_SIZE || rd_u32(mem, 0) != SIGNATURE_ACTIVE {
            self.status = t!("HWiNFO wurde beendet", "HWiNFO was closed").into();
            self.close();
            self.retry_in = 5;
            return;
        }
        // Header: sig, version, revision, i64 poll_time, then 6 x u32.
        let off_s = rd_u32(mem, 20) as usize;
        let size_s = rd_u32(mem, 24) as usize;
        let num_s = rd_u32(mem, 28) as usize;
        let off_r = rd_u32(mem, 32) as usize;
        let size_r = rd_u32(mem, 36) as usize;
        let num_r = rd_u32(mem, 40) as usize;
        if size_r < READING_MIN_SIZE
            || off_r + size_r * num_r > mem.len()
            || off_s + size_s * num_s > mem.len()
        {
            self.status = t!(
                "Unbekanntes Shared-Memory-Format",
                "Unknown shared memory format"
            )
            .into();
            return;
        }
        let sensor_names: Vec<String> = (0..num_s)
            .map(|i| {
                let e = &mem[off_s + i * size_s..off_s + (i + 1) * size_s];
                let user = rd_str(e, SENSOR_NAME_USER, 128);
                if user.is_empty() {
                    rd_str(e, SENSOR_NAME_ORIG, 128)
                } else {
                    user
                }
            })
            .collect();
        let start = out.len();
        for i in 0..num_r {
            let e = &mem[off_r + i * size_r..off_r + (i + 1) * size_r];
            let kind = match rd_u32(e, READING_TYPE) {
                1 => SensorKind::Temperature,
                2 => SensorKind::Voltage,
                3 => SensorKind::Fan,
                4 => SensorKind::Current,
                5 => SensorKind::Power,
                6 => SensorKind::Clock,
                7 => SensorKind::Load,
                8 => SensorKind::Other,
                _ => continue,
            };
            let si = rd_u32(e, READING_SENSOR_INDEX) as usize;
            let device = sensor_names.get(si).cloned().unwrap_or_default();
            let user = rd_str(e, READING_LABEL_USER, 128);
            let label = if user.is_empty() {
                rd_str(e, READING_LABEL_ORIG, 128)
            } else {
                user
            };
            out.push(Reading {
                id: format!("hwinfo:{si}:{}", rd_u32(e, READING_ID)),
                source: "HWiNFO".into(),
                group: classify_device(&device),
                device,
                label,
                kind,
                value: rd_f64(e, READING_VALUE),
                unit: rd_str(e, READING_UNIT, 16),
            });
        }
        self.last_count = out.len() - start;
        self.status = tf!(
            "verbunden, {} Werte",
            "connected, {} values",
            self.last_count
        );
    }
}
