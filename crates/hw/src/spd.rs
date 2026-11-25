//! SPD (Serial Presence Detect) decoder for DDR4 and DDR5 modules.
//!
//! Byte layout follows the public JEDEC SPD specifications (DDR4: JEDEC 21-C
//! Annex L, DDR5: JESD400-5). XMP/EXPO detection uses the publicly documented
//! header signatures.
//!
//! Where the bytes come from:
//! * Boot edition (Linux): the kernel drivers `ee1004` (DDR4) and `spd5118` (DDR5)
//!   expose the EEPROM as a file, see [`read_linux_eeproms`].
//! * Windows: SMBus access needs a kernel driver, which this version does not
//!   ship. A dump file (.bin) can be loaded instead.
//!
//! STATUS: JEDEC base timings are implemented from the spec; the DDR4 XMP 2.0
//! profile decoder is marked experimental until it is checked against a
//! collection of real dumps (contributions welcome, see CONTRIBUTING.md).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SpdInfo {
    pub dram_type: String,
    pub module_type: String,
    pub capacity_bytes: u64,
    pub ranks: u32,
    pub device_width: u32,
    pub bus_width: u32,
    pub ecc: bool,
    pub manufacturer: String,
    pub dram_manufacturer: String,
    pub part_number: String,
    pub serial: String,
    pub manufacture_date: String,
    pub jedec: Timings,
    pub xmp_present: bool,
    pub expo_present: bool,
    /// Decoded overclocking profiles (DDR4 XMP 2.0 only so far; experimental).
    pub profiles: Vec<Timings>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Timings {
    pub name: String,
    pub speed_mts: u32,
    pub tck_ps: u32,
    pub cl: u32,
    pub trcd: u32,
    pub trp: u32,
    pub tras: u32,
    pub trc: u32,
    pub trfc1_ns: f64,
    pub voltage_v: Option<f64>,
}

impl Timings {
    pub fn short(&self) -> String {
        format!(
            "{} MT/s  {}-{}-{}-{}{}",
            self.speed_mts,
            self.cl,
            self.trcd,
            self.trp,
            self.tras,
            self.voltage_v.map(|v| format!("  {v:.2} V")).unwrap_or_default()
        )
    }
}

/// JEDEC rounding: nCK = ceil(t / tCK) with a 2.5 % guard band (JESD21-C).
fn clocks(t_ps: i64, tck_ps: i64) -> u32 {
    if tck_ps <= 0 || t_ps <= 0 {
        return 0;
    }
    ((t_ps * 1000 / tck_ps + 974) / 1000) as u32
}

fn mts(tck_ps: i64) -> u32 {
    if tck_ps <= 0 {
        0
    } else {
        // SPD stores tCK truncated to whole ps (DDR4-3600: 555 ps = 3603.6 MT/s).
        // All DDR speed grades are multiples of 100/3 MT/s, so snap to that grid.
        let raw = 2_000_000.0 / tck_ps as f64;
        let n = (raw * 3.0 / 100.0).round();
        (n * 100.0 / 3.0).floor() as u32
    }
}

fn bcd(b: u8) -> u32 {
    ((b >> 4) * 10 + (b & 0xF)) as u32
}

fn ascii(b: &[u8]) -> String {
    b.iter()
        .map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { ' ' })
        .collect::<String>()
        .trim()
        .to_string()
}

/// JEP106 manufacturer ID (continuation byte, ID byte, both with parity).
pub fn jep106(cont: u8, id: u8) -> String {
    let bank = (cont & 0x7F) + 1;
    let name = match (bank, id) {
        (1, 0xCE) => "Samsung",
        (1, 0x2C) => "Micron",
        (1, 0xAD) => "SK hynix",
        (1, 0xFE) => "Elpida",
        (2, 0x98) => "Kingston",
        (3, 0x9E) => "Corsair",
        (4, 0x0B) => "Nanya",
        (5, 0xCD) => "G.Skill",
        (5, 0xCB) => "ADATA",
        (5, 0xEF) => "Team Group",
        (6, 0x9B) => "Crucial",
        (0x01..=0x10, 0x00) | (_, 0xFF) => "",
        _ => return format!("JEP106 Bank {bank} / 0x{id:02X}"),
    };
    name.to_string()
}

pub fn decode(spd: &[u8]) -> Result<SpdInfo, String> {
    if spd.len() < 128 {
        return Err("SPD-Daten zu kurz".into());
    }
    match spd[2] {
        0x0C => decode_ddr4(spd),
        0x12 => decode_ddr5(spd),
        0x0B => Err("DDR3 wird noch nicht dekodiert".into()),
        t => Err(format!("Unbekannter DRAM-Typ 0x{t:02X}")),
    }
}

fn ddr4_module_type(b: u8) -> &'static str {
    match b & 0x0F {
        0x01 => "RDIMM",
        0x02 => "UDIMM",
        0x03 => "SO-DIMM",
        0x04 => "LRDIMM",
        _ => "Sonstige",
    }
}

fn decode_ddr4(s: &[u8]) -> Result<SpdInfo, String> {
    if s.len() < 384 {
        return Err("DDR4-SPD braucht mindestens 384 Byte".into());
    }
    let mtb = |b: u8| b as i64 * 125; // 0.125 ns
    let ftb = |b: u8| b as i8 as i64; // 1 ps, signed
    let tck = mtb(s[18]) + ftb(s[125]);
    let taa = mtb(s[24]) + ftb(s[123]);
    let trcd = mtb(s[25]) + ftb(s[122]);
    let trp = mtb(s[26]) + ftb(s[121]);
    let tras = mtb(s[28]) + (((s[27] & 0x0F) as i64) << 8) * 125;
    let trc = ((((s[27] >> 4) as i64) << 8) | s[29] as i64) * 125 + ftb(s[120]);
    let trfc1 = u16::from_le_bytes([s[30], s[31]]) as i64 * 125;

    let density_gbit = match s[4] & 0x0F {
        0 => 0.25,
        1 => 0.5,
        2 => 1.0,
        3 => 2.0,
        4 => 4.0,
        5 => 8.0,
        6 => 16.0,
        7 => 32.0,
        8 => 12.0,
        9 => 24.0,
        _ => 0.0,
    };
    let device_width = 4u32 << (s[12] & 0x07);
    let ranks = ((s[12] >> 3) & 0x07) as u32 + 1;
    let bus_width = 8u32 << (s[13] & 0x07);
    let ecc = (s[13] >> 3) & 0x03 == 1;
    let three_ds = s[6] & 0x03 == 2;
    let dies = if three_ds { ((s[6] >> 4) & 0x07) as u32 + 1 } else { 1 };
    let capacity = (density_gbit / 8.0 * (bus_width / device_width) as f64 * ranks as f64 * dies as f64
        * (1u64 << 30) as f64) as u64;

    let mut info = SpdInfo {
        dram_type: "DDR4".into(),
        module_type: ddr4_module_type(s[3]).into(),
        capacity_bytes: capacity,
        ranks,
        device_width,
        bus_width,
        ecc,
        jedec: Timings {
            name: "JEDEC".into(),
            speed_mts: mts(tck),
            tck_ps: tck as u32,
            cl: clocks(taa, tck),
            trcd: clocks(trcd, tck),
            trp: clocks(trp, tck),
            tras: clocks(tras, tck),
            trc: clocks(trc, tck),
            trfc1_ns: trfc1 as f64 / 1000.0,
            voltage_v: Some(1.2),
        },
        ..Default::default()
    };
    if s.len() >= 352 {
        info.manufacturer = jep106(s[320], s[321]);
        info.manufacture_date = if s[323] != 0 { format!("20{:02} KW {:02}", bcd(s[323]), bcd(s[324])) } else { String::new() };
        info.serial = s[325..329].iter().map(|b| format!("{b:02X}")).collect();
        info.part_number = ascii(&s[329..349]);
        info.dram_manufacturer = jep106(s[350], s[351]);
    }
    if s.len() >= 440 && s[384] == 0x0C && s[385] == 0x4A {
        info.xmp_present = true;
        for (n, base) in [(0usize, 393usize), (1, 440)] {
            if s[386] & (1 << n) != 0 && s.len() >= base + 39 {
                info.profiles.push(decode_xmp2_profile(s, base, n + 1));
            }
        }
        info.notes.push("XMP-2.0-Dekodierung ist experimentell".into());
    }
    Ok(info)
}

/// DDR4 XMP 2.0 profile, offsets relative to the profile start.
fn decode_xmp2_profile(s: &[u8], b: usize, num: usize) -> Timings {
    let mtb = |x: u8| x as i64 * 125;
    let ftb = |x: u8| x as i8 as i64;
    let v = s[b];
    let voltage = (v >> 7) as f64 + (v & 0x7F) as f64 / 100.0;
    let tck = mtb(s[b + 3]) + ftb(s[b + 38]);
    let taa = mtb(s[b + 8]) + ftb(s[b + 37]);
    let trcd = mtb(s[b + 9]) + ftb(s[b + 36]);
    let trp = mtb(s[b + 10]) + ftb(s[b + 35]);
    let tras = ((((s[b + 11] & 0x0F) as i64) << 8) | s[b + 12] as i64) * 125;
    let trc = ((((s[b + 11] >> 4) as i64) << 8) | s[b + 13] as i64) * 125 + ftb(s[b + 34]);
    let trfc1 = u16::from_le_bytes([s[b + 14], s[b + 15]]) as i64 * 125;
    Timings {
        name: format!("XMP Profil {num}"),
        speed_mts: mts(tck),
        tck_ps: tck as u32,
        cl: clocks(taa, tck),
        trcd: clocks(trcd, tck),
        trp: clocks(trp, tck),
        tras: clocks(tras, tck),
        trc: clocks(trc, tck),
        trfc1_ns: trfc1 as f64 / 1000.0,
        voltage_v: Some(voltage),
    }
}

fn ddr5_module_type(b: u8) -> &'static str {
    match b & 0x0F {
        0x01 => "RDIMM",
        0x02 => "UDIMM",
        0x03 => "SO-DIMM",
        0x04 => "LRDIMM",
        0x05 => "CUDIMM",
        0x06 => "CSODIMM",
        0x07 => "MRDIMM",
        0x08 => "CAMM2",
        0x0A => "DDIMM",
        0x0B => "Solder down",
        _ => "Sonstige",
    }
}

fn decode_ddr5(s: &[u8]) -> Result<SpdInfo, String> {
    if s.len() < 640 {
        return Err("DDR5-SPD braucht mindestens 640 Byte".into());
    }
    let w = |o: usize| u16::from_le_bytes([s[o], s[o + 1]]) as i64;
    let tck = w(20);
    let density_gbit = match s[4] & 0x1F {
        1 => 4.0,
        2 => 8.0,
        3 => 12.0,
        4 => 16.0,
        5 => 24.0,
        6 => 32.0,
        7 => 48.0,
        8 => 64.0,
        _ => 0.0,
    };
    let dies = match (s[4] >> 5) & 0x07 {
        0 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        _ => 1,
    } as f64;
    let device_width = 4u32 << ((s[6] >> 5) & 0x07);
    let ranks = ((s[234] >> 3) & 0x07) as u32 + 1;
    let bus_per_channel = 8u32 << (s[235] & 0x07);
    let channels = ((s[235] >> 5) & 0x03) as u32 + 1;
    let ecc = (s[235] >> 3) & 0x03 != 0;
    let capacity = (channels as f64 * (bus_per_channel / device_width) as f64 * dies * density_gbit / 8.0
        * ranks as f64
        * (1u64 << 30) as f64) as u64;
    let mut info = SpdInfo {
        dram_type: "DDR5".into(),
        module_type: ddr5_module_type(s[3]).into(),
        capacity_bytes: capacity,
        ranks,
        device_width,
        bus_width: bus_per_channel * channels,
        ecc,
        jedec: Timings {
            name: "JEDEC".into(),
            speed_mts: mts(tck),
            tck_ps: tck as u32,
            cl: clocks(w(30), tck),
            trcd: clocks(w(32), tck),
            trp: clocks(w(34), tck),
            tras: clocks(w(36), tck),
            trc: clocks(w(38), tck),
            trfc1_ns: w(42) as f64,
            voltage_v: Some(1.1),
        },
        ..Default::default()
    };
    // DDR5 CL must be even.
    if info.jedec.cl % 2 == 1 {
        info.jedec.cl += 1;
    }
    if s.len() >= 554 {
        info.manufacturer = jep106(s[512], s[513]);
        info.manufacture_date = if s[515] != 0 { format!("20{:02} KW {:02}", bcd(s[515]), bcd(s[516])) } else { String::new() };
        info.serial = s[517..521].iter().map(|b| format!("{b:02X}")).collect();
        info.part_number = ascii(&s[521..551]);
        info.dram_manufacturer = jep106(s[552], s[553]);
    }
    info.xmp_present = s.len() >= 642 && s[640] == 0x0C && s[641] == 0x4A;
    info.expo_present = s.len() >= 836 && &s[832..836] == b"EXPO";
    if info.xmp_present || info.expo_present {
        info.notes.push("XMP-3.0/EXPO-Profile erkannt; Profilwerte werden in einer späteren Version dekodiert".into());
    }
    Ok(info)
}

/// Reads all SPD EEPROMs the Linux kernel exposes (ee1004 = DDR4, spd5118 = DDR5).
/// Needs `modprobe ee1004` / `modprobe spd5118` on the live system.
pub fn read_linux_eeproms() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/bus/i2c/devices") else { return out };
    let mut entries: Vec<_> = dir.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = std::fs::read_to_string(p.join("name")).unwrap_or_default();
        let name = name.trim();
        if name == "ee1004" || name == "spd5118" {
            if let Ok(bytes) = std::fs::read(p.join("eeprom")) {
                out.push((e.file_name().to_string_lossy().to_string(), bytes));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic DDR4-3200 CL22 8 GB single-rank x8 UDIMM.
    fn ddr4_sample() -> Vec<u8> {
        let mut s = vec![0u8; 512];
        s[2] = 0x0C;
        s[3] = 0x02;
        s[4] = 0x85; // 8 Gbit, 4 bank groups
        s[12] = 0x01; // x8, 1 rank
        s[13] = 0x03; // 64 bit, no ECC
        s[18] = 5; // 625 ps
        s[125] = 0;
        s[24] = 110; // 13.75 ns -> CL22
        s[25] = 110;
        s[26] = 110;
        s[27] = 0x11; // tRAS 32 ns = 256 MTB
        s[28] = 0x00;
        s[29] = 0x6E; // tRC = 0x16E MTB = 45.75 ns
        s[30..32].copy_from_slice(&2800u16.to_le_bytes()); // 350 ns
        s[320] = 0x01; // bank 2 with parity... continuation count 1
        s[321] = 0x98; // Kingston
        s[329..336].copy_from_slice(b"TEST-01");
        // XMP 2.0 profile 1: DDR4-3600 (tCK 555 ps) 1.35 V
        s[384] = 0x0C;
        s[385] = 0x4A;
        s[386] = 0x01;
        s[393] = 0x80 | 35;
        s[393 + 3] = 5; // 625 ps
        s[393 + 38] = (-70i8) as u8; // -> 555 ps
        s[393 + 8] = 0x70; // 14 ns
        s
    }

    #[test]
    fn decodes_ddr4() {
        let i = decode(&ddr4_sample()).unwrap();
        assert_eq!(i.dram_type, "DDR4");
        assert_eq!(i.capacity_bytes, 8 << 30);
        assert_eq!(i.jedec.speed_mts, 3200);
        assert_eq!(i.jedec.cl, 22);
        assert_eq!(i.jedec.trfc1_ns, 350.0);
        assert_eq!(i.manufacturer, "Kingston");
        assert_eq!(i.part_number, "TEST-01");
        assert!(i.xmp_present);
        assert_eq!(i.profiles.len(), 1);
        assert_eq!(i.profiles[0].speed_mts, 3600);
        assert_eq!(i.profiles[0].voltage_v, Some(1.35));
    }

    #[test]
    fn decodes_ddr5_capacity() {
        let mut s = vec![0u8; 1024];
        s[2] = 0x12;
        s[3] = 0x02;
        s[4] = 0x04; // 16 Gbit, monolithic
        s[6] = 0x20; // x8
        s[20..22].copy_from_slice(&416u16.to_le_bytes()); // DDR5-4800
        s[30..32].copy_from_slice(&16_640u16.to_le_bytes()); // tAA 16.64 ns -> CL40
        s[234] = 0x00; // 1 rank
        s[235] = 0x22; // 2 channels x 32 bit
        let i = decode(&s).unwrap();
        assert_eq!(i.capacity_bytes, 16 << 30);
        assert_eq!(i.jedec.speed_mts, 4800);
        assert_eq!(i.jedec.cl, 40);
    }
}
