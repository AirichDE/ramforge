//! SMBIOS / DMI table parser (DMTF DSP0134).
//!
//! The firmware describes the board, BIOS, CPU sockets and every memory slot in
//! this table. On Windows it is read with GetSystemFirmwareTable('RSMB'), on
//! Linux from /sys/firmware/dmi/tables. No driver and no admin rights needed.

#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SmbiosInfo {
    pub version: String,
    pub bios_vendor: String,
    pub bios_version: String,
    pub bios_date: String,
    pub system_manufacturer: String,
    pub system_product: String,
    pub board_manufacturer: String,
    pub board_product: String,
    pub board_version: String,
    pub processors: Vec<SmbiosCpu>,
    pub memory_slots_total: u32,
    pub memory_max_capacity_bytes: u64,
    pub memory: Vec<MemoryDevice>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SmbiosCpu {
    pub socket: String,
    pub manufacturer: String,
    pub version: String,
    pub external_clock_mhz: u16,
    pub max_speed_mhz: u16,
    pub current_speed_mhz: u16,
    pub cores: u16,
    pub threads: u16,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryDevice {
    pub locator: String,
    pub bank: String,
    pub size_bytes: u64,
    pub memory_type: String,
    pub form_factor: String,
    /// Rated speed of the module in MT/s.
    pub speed_mts: u32,
    /// Speed the memory controller actually runs it at, in MT/s.
    pub configured_speed_mts: u32,
    pub manufacturer: String,
    pub part_number: String,
    pub serial: String,
    pub ranks: u8,
    pub data_width: u16,
    pub total_width: u16,
    pub min_voltage_mv: u16,
    pub max_voltage_mv: u16,
    pub configured_voltage_mv: u16,
}

impl MemoryDevice {
    pub fn is_installed(&self) -> bool {
        self.size_bytes > 0
    }
    pub fn has_ecc(&self) -> bool {
        self.total_width > self.data_width && self.data_width > 0
    }
}

struct Structure<'a> {
    kind: u8,
    data: &'a [u8],
    strings: Vec<&'a [u8]>,
}

impl<'a> Structure<'a> {
    fn byte(&self, off: usize) -> Option<u8> {
        self.data.get(off).copied()
    }
    fn word(&self, off: usize) -> Option<u16> {
        Some(u16::from_le_bytes(
            self.data.get(off..off + 2)?.try_into().ok()?,
        ))
    }
    fn dword(&self, off: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            self.data.get(off..off + 4)?.try_into().ok()?,
        ))
    }
    fn qword(&self, off: usize) -> Option<u64> {
        Some(u64::from_le_bytes(
            self.data.get(off..off + 8)?.try_into().ok()?,
        ))
    }
    fn string(&self, off: usize) -> String {
        let idx = self.byte(off).unwrap_or(0) as usize;
        if idx == 0 {
            return String::new();
        }
        self.strings
            .get(idx - 1)
            .map(|s| String::from_utf8_lossy(s).trim().to_string())
            .unwrap_or_default()
    }
}

fn structures(table: &[u8]) -> Vec<Structure<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= table.len() {
        let kind = table[pos];
        let len = table[pos + 1] as usize;
        if len < 4 || pos + len > table.len() {
            break;
        }
        let data = &table[pos..pos + len];
        // String set: NUL-terminated strings, ended by a double NUL.
        let mut p = pos + len;
        let mut strings = Vec::new();
        if p + 1 < table.len() && table[p] == 0 && table[p + 1] == 0 {
            p += 2;
        } else {
            while p < table.len() {
                let start = p;
                while p < table.len() && table[p] != 0 {
                    p += 1;
                }
                strings.push(&table[start..p]);
                p += 1;
                if p < table.len() && table[p] == 0 {
                    p += 1;
                    break;
                }
            }
        }
        out.push(Structure {
            kind,
            data,
            strings,
        });
        if kind == 127 {
            break; // end-of-table
        }
        pos = p;
    }
    out
}

fn memory_type_name(t: u8) -> String {
    match t {
        0x0F => "SDRAM",
        0x12 => "DDR",
        0x13 => "DDR2",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        0x02 => t!("Unbekannt", "Unknown"),
        _ => return tf!("Typ 0x{t:02X}", "Type 0x{t:02X}"),
    }
    .to_string()
}

fn form_factor_name(f: u8) -> String {
    match f {
        0x09 => "DIMM",
        0x0D => "SO-DIMM",
        0x0F => "FB-DIMM",
        0x10 => "Die",
        0x11 => "CAMM",
        0x0B => "Row of chips",
        _ => return format!("0x{f:02X}"),
    }
    .to_string()
}

pub fn parse(table: &[u8], major: u8, minor: u8) -> SmbiosInfo {
    let mut info = SmbiosInfo {
        version: format!("{major}.{minor}"),
        ..Default::default()
    };
    for s in structures(table) {
        match s.kind {
            0 => {
                info.bios_vendor = s.string(0x04);
                info.bios_version = s.string(0x05);
                info.bios_date = s.string(0x08);
            }
            1 => {
                info.system_manufacturer = s.string(0x04);
                info.system_product = s.string(0x05);
            }
            2 => {
                info.board_manufacturer = s.string(0x04);
                info.board_product = s.string(0x05);
                info.board_version = s.string(0x06);
            }
            4 => {
                let mut cores = s.byte(0x23).unwrap_or(0) as u16;
                let mut threads = s.byte(0x25).unwrap_or(0) as u16;
                if cores == 0xFF {
                    cores = s.word(0x2A).unwrap_or(cores);
                }
                if threads == 0xFF {
                    threads = s.word(0x2E).unwrap_or(threads);
                }
                info.processors.push(SmbiosCpu {
                    socket: s.string(0x04),
                    manufacturer: s.string(0x07),
                    version: s.string(0x10),
                    external_clock_mhz: s.word(0x12).unwrap_or(0),
                    max_speed_mhz: s.word(0x14).unwrap_or(0),
                    current_speed_mhz: s.word(0x16).unwrap_or(0),
                    cores,
                    threads,
                });
            }
            16 => {
                // Only count arrays used for system memory (use = 0x03).
                if s.byte(0x05) == Some(0x03) {
                    info.memory_slots_total += s.word(0x0D).unwrap_or(0) as u32;
                    let kb = s.dword(0x07).unwrap_or(0);
                    let bytes = if kb == 0x8000_0000 {
                        s.qword(0x0F).unwrap_or(0)
                    } else {
                        kb as u64 * 1024
                    };
                    info.memory_max_capacity_bytes += bytes;
                }
            }
            17 => info.memory.push(parse_memory_device(&s)),
            _ => {}
        }
    }
    info
}

fn parse_memory_device(s: &Structure) -> MemoryDevice {
    let size_raw = s.word(0x0C).unwrap_or(0);
    let size_bytes = match size_raw {
        0 | 0xFFFF => 0,
        0x7FFF => (s.dword(0x1C).unwrap_or(0) & 0x7FFF_FFFF) as u64 * (1 << 20),
        v if v & 0x8000 != 0 => (v & 0x7FFF) as u64 * 1024,
        v => v as u64 * (1 << 20),
    };
    let mut speed = s.word(0x15).unwrap_or(0) as u32;
    if speed == 0xFFFF {
        speed = s.dword(0x54).unwrap_or(0);
    }
    let mut conf = s.word(0x20).unwrap_or(0) as u32;
    if conf == 0xFFFF {
        conf = s.dword(0x58).unwrap_or(0);
    }
    MemoryDevice {
        locator: s.string(0x10),
        bank: s.string(0x11),
        size_bytes,
        memory_type: memory_type_name(s.byte(0x12).unwrap_or(2)),
        form_factor: form_factor_name(s.byte(0x0E).unwrap_or(2)),
        speed_mts: speed,
        configured_speed_mts: conf,
        manufacturer: module_manufacturer(&s.string(0x17), &s.string(0x1A)),
        part_number: s.string(0x1A),
        serial: s.string(0x18),
        ranks: s.byte(0x1B).unwrap_or(0) & 0x0F,
        data_width: s.word(0x0A).filter(|&w| w != 0xFFFF).unwrap_or(0),
        total_width: s.word(0x08).filter(|&w| w != 0xFFFF).unwrap_or(0),
        min_voltage_mv: s.word(0x22).unwrap_or(0),
        max_voltage_mv: s.word(0x24).unwrap_or(0),
        configured_voltage_mv: s.word(0x26).unwrap_or(0),
    }
}

/// Reads and parses the SMBIOS table of the running machine.
pub fn read() -> Result<SmbiosInfo, String> {
    let (table, major, minor) = sys::raw_table()?;
    Ok(parse(&table, major, minor))
}

#[cfg(windows)]
mod sys {
    use windows_sys::Win32::System::SystemInformation::GetSystemFirmwareTable;

    pub fn raw_table() -> Result<(Vec<u8>, u8, u8), String> {
        const RSMB: u32 = u32::from_be_bytes(*b"RSMB");
        let size = unsafe { GetSystemFirmwareTable(RSMB, 0, std::ptr::null_mut(), 0) };
        if size < 8 {
            return Err(ramforge_i18n::t!(
                "SMBIOS-Tabelle nicht verfügbar",
                "SMBIOS table not available"
            )
            .into());
        }
        let mut buf = vec![0u8; size as usize];
        let got = unsafe { GetSystemFirmwareTable(RSMB, 0, buf.as_mut_ptr() as *mut _, size) };
        if got < 8 {
            return Err(ramforge_i18n::t!(
                "SMBIOS-Tabelle konnte nicht gelesen werden",
                "SMBIOS table could not be read"
            )
            .into());
        }
        // RawSMBIOSData: u8 calling method, u8 major, u8 minor, u8 dmi rev, u32 length, data
        let (major, minor) = (buf[1], buf[2]);
        let len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
        let end = (8 + len).min(buf.len());
        Ok((buf[8..end].to_vec(), major, minor))
    }
}

#[cfg(unix)]
mod sys {
    pub fn raw_table() -> Result<(Vec<u8>, u8, u8), String> {
        let table = std::fs::read("/sys/firmware/dmi/tables/DMI").map_err(|e| {
            ramforge_i18n::tf!(
                "DMI-Tabelle nicht lesbar: {e}",
                "DMI table not readable: {e}"
            )
        })?;
        let ep = std::fs::read("/sys/firmware/dmi/tables/smbios_entry_point").unwrap_or_default();
        let (major, minor) = if ep.starts_with(b"_SM3_") && ep.len() > 8 {
            (ep[7], ep[8])
        } else if ep.starts_with(b"_SM_") && ep.len() > 7 {
            (ep[6], ep[7])
        } else {
            (0, 0)
        };
        Ok((table, major, minor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type17(size_mb: u16, speed: u16) -> Vec<u8> {
        let mut d = vec![0u8; 0x28];
        d[0] = 17;
        d[1] = 0x28;
        d[0x08..0x0A].copy_from_slice(&72u16.to_le_bytes());
        d[0x0A..0x0C].copy_from_slice(&64u16.to_le_bytes());
        d[0x0C..0x0E].copy_from_slice(&size_mb.to_le_bytes());
        d[0x0E] = 0x09;
        d[0x10] = 1; // locator string #1
        d[0x12] = 0x22;
        d[0x15..0x17].copy_from_slice(&speed.to_le_bytes());
        d[0x17] = 2; // manufacturer string #2
        d[0x1A] = 3; // part number string #3
        d[0x1B] = 2;
        d[0x20..0x22].copy_from_slice(&5600u16.to_le_bytes());
        d[0x26..0x28].copy_from_slice(&1250u16.to_le_bytes());
        d.extend_from_slice(b"DIMM_A1\0Kingston\0KF556C40-16\0\0");
        d
    }

    #[test]
    fn parses_memory_device() {
        let mut table = type17(16384, 6000);
        table.extend_from_slice(&[127, 4, 0, 0, 0, 0]);
        let info = parse(&table, 3, 5);
        assert_eq!(info.memory.len(), 1);
        let m = &info.memory[0];
        assert_eq!(m.size_bytes, 16 << 30);
        assert_eq!(m.memory_type, "DDR5");
        assert_eq!(m.locator, "DIMM_A1");
        assert_eq!(m.manufacturer, "Kingston");
        assert_eq!(m.part_number, "KF556C40-16");
        assert_eq!(m.speed_mts, 6000);
        assert_eq!(m.configured_speed_mts, 5600);
        assert_eq!(m.configured_voltage_mv, 1250);
        assert_eq!(m.ranks, 2);
        assert!(m.has_ecc());
    }
}

/// Module manufacturer as shown to the user. Many boards leave the SMBIOS
/// field empty, write "Unknown" or a raw JEDEC code ("04CD", "CD04"): then
/// the JEDEC code is decoded, or the vendor is derived from the part number.
pub fn module_manufacturer(raw: &str, part: &str) -> String {
    let r = raw.trim();
    let placeholder = r.is_empty()
        || r.eq_ignore_ascii_case("unknown")
        || r.eq_ignore_ascii_case("undefined")
        || r.eq_ignore_ascii_case("not specified")
        || r.chars().all(|c| c == '0' || c == 'F' || c == 'f');
    // Raw JEDEC code: 4 hex digits, continuation count + ID or ID + count.
    if r.len() == 4 && r.chars().all(|c| c.is_ascii_hexdigit()) {
        let a = u8::from_str_radix(&r[0..2], 16).unwrap_or(0);
        let b = u8::from_str_radix(&r[2..4], 16).unwrap_or(0);
        for (cont, id) in [(a, b), (b, a)] {
            if cont < 0x10 {
                let n = crate::spd::jep106(cont, id);
                if !n.is_empty() && !n.starts_with("JEP106") {
                    return n;
                }
            }
        }
    }
    if !placeholder {
        return r.to_string();
    }
    vendor_from_part(part)
        .map(str::to_string)
        .unwrap_or_else(|| r.to_string())
}

/// Vendor from well-known part number prefixes.
pub fn vendor_from_part(part: &str) -> Option<&'static str> {
    let p = part.trim().to_ascii_uppercase();
    const PREFIXES: &[(&str, &str)] = &[
        ("F3-", "G.Skill"),
        ("F4-", "G.Skill"),
        ("F5-", "G.Skill"),
        ("CMK", "Corsair"),
        ("CMW", "Corsair"),
        ("CMT", "Corsair"),
        ("CMH", "Corsair"),
        ("CMR", "Corsair"),
        ("CMD", "Corsair"),
        ("CMSX", "Corsair"),
        ("CMSO", "Corsair"),
        ("CMG", "Corsair"),
        ("KF", "Kingston"),
        ("KHX", "Kingston"),
        ("KVR", "Kingston"),
        ("KCP", "Kingston"),
        ("ACR", "Kingston"),
        ("BLS", "Crucial"),
        ("BL", "Crucial"),
        ("CT", "Crucial"),
        ("MTA", "Micron"),
        ("MT", "Micron"),
        ("M378", "Samsung"),
        ("M471", "Samsung"),
        ("M393", "Samsung"),
        ("M391", "Samsung"),
        ("M323", "Samsung"),
        ("M425", "Samsung"),
        ("HMA", "SK hynix"),
        ("HMC", "SK hynix"),
        ("HMT", "SK hynix"),
        ("TED", "Team Group"),
        ("TF", "Team Group"),
        ("TLZ", "Team Group"),
        ("FF", "Team Group"),
        ("PVS", "Patriot"),
        ("PV", "Patriot"),
        ("PSD", "Patriot"),
        ("AX4", "ADATA"),
        ("AX5", "ADATA"),
        ("AD4", "ADATA"),
        ("AD5", "ADATA"),
        ("NT", "Nanya"),
    ];
    PREFIXES
        .iter()
        .find(|(pre, _)| p.starts_with(pre))
        .map(|(_, v)| *v)
}

#[cfg(test)]
mod vendor_tests {
    use super::*;

    #[test]
    fn manufacturer_fallbacks() {
        assert_eq!(module_manufacturer("Unknown", "F4-3466C16-8GVK"), "G.Skill");
        assert_eq!(module_manufacturer("", "CMK16GX4M2B3200C16"), "Corsair");
        assert_eq!(module_manufacturer("Kingston", "KF432C16BB/8"), "Kingston");
        assert_eq!(module_manufacturer("04CD", "x"), "G.Skill");
        assert_eq!(module_manufacturer("Unknown", "XYZ"), "Unknown");
    }
}
