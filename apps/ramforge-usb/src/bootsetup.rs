//! Makes a USB stick bootable with the RAMForge boot edition (Windows).
//!
//! Layout (UEFI, works with Secure Boot ON):
//!   MBR, one primary FAT32 partition "RAMFORGE" (at most 32 GB, rest unused)
//!   ├─ EFI/boot/bootx64.efi   Microsoft-signed shim from Debian
//!   ├─ EFI/boot/grubx64.efi   signed GRUB, boot/grub/…  live/…  (1:1 from the ISO)
//!   └─ RAMForge/              portable Windows EXEs, stick.json, RAMForge-data/
//!
//! SAFETY RULES (checked when listing AND again right before erasing):
//! * only disks on the USB bus (or, with --allow-virtual-disk, VHD files for tests)
//! * never the system or boot disk, never the disk this program runs from
//! * never a disk that has a folder "RAMForge-Keys" on any of its volumes
//!   (offline signing key backup) – such a stick is refused, full stop
//! * the user has to type LÖSCHEN; the disk number is re-verified by serial

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const KEY_FOLDER: &str = "RAMForge-Keys";
pub const VOLUME_LABEL: &str = "RAMFORGE";
const MAX_FAT32_MB: u64 = 32_000;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DiskInfo {
    pub number: u32,
    #[serde(default)]
    pub friendly_name: String,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub bus_type: String,
    pub size: u64,
    #[serde(default)]
    pub is_system: bool,
    #[serde(default)]
    pub is_boot: bool,
    /// Drive letters of all volumes on this disk, e.g. ["I"].
    #[serde(default)]
    pub letters: Vec<String>,
}

fn powershell(script: &str) -> Result<String, String> {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|e| format!("PowerShell: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The physical disk that holds drive `letter` ("I" or "I:\").
pub fn disk_for_letter(letter: &str) -> Result<DiskInfo, String> {
    let l = letter.trim_end_matches(['\\', ':']);
    if l.len() != 1 || !l.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(format!("ungültiger Laufwerksbuchstabe {letter}"));
    }
    let script = format!(
        "$d = Get-Partition -DriveLetter {l} -ErrorAction Stop | Get-Disk; \
         $letters = @(Get-Partition -DiskNumber $d.Number | Where-Object DriveLetter | ForEach-Object {{ [string]$_.DriveLetter }}); \
         [pscustomobject]@{{ Number=$d.Number; FriendlyName=$d.FriendlyName; SerialNumber=$d.SerialNumber; BusType=[string]$d.BusType; \
           Size=[uint64]$d.Size; IsSystem=[bool]$d.IsSystem; IsBoot=[bool]$d.IsBoot; Letters=$letters }} | ConvertTo-Json -Compress"
    );
    let json = powershell(&script)?;
    serde_json::from_str(&json).map_err(|e| format!("Datenträgerinfo: {e}: {json}"))
}

pub fn disk_by_number(n: u32) -> Result<DiskInfo, String> {
    let script = format!(
        "$d = Get-Disk -Number {n} -ErrorAction Stop; \
         $letters = @(Get-Partition -DiskNumber $d.Number -ErrorAction SilentlyContinue | Where-Object DriveLetter | ForEach-Object {{ [string]$_.DriveLetter }}); \
         [pscustomobject]@{{ Number=$d.Number; FriendlyName=$d.FriendlyName; SerialNumber=$d.SerialNumber; BusType=[string]$d.BusType; \
           Size=[uint64]$d.Size; IsSystem=[bool]$d.IsSystem; IsBoot=[bool]$d.IsBoot; Letters=$letters }} | ConvertTo-Json -Compress"
    );
    let json = powershell(&script)?;
    serde_json::from_str(&json).map_err(|e| format!("Datenträgerinfo: {e}: {json}"))
}

/// Why this disk must not be erased, or None if it may be.
pub fn refusal(d: &DiskInfo, allow_virtual: bool) -> Option<String> {
    if d.is_system || d.is_boot {
        return Some("System- oder Startdatenträger von Windows – wird nie gelöscht.".into());
    }
    let bus = d.bus_type.to_ascii_lowercase();
    let virtual_ok = allow_virtual && (bus.contains("virtual") || bus == "file backed virtual");
    if bus != "usb" && !virtual_ok {
        return Some(format!(
            "Kein USB-Datenträger (Bus: {}). Nur USB-Sticks werden eingerichtet.",
            d.bus_type
        ));
    }
    for l in &d.letters {
        // Existence check only: the folder's content is never listed or read.
        if Path::new(&format!("{l}:\\{KEY_FOLDER}")).exists() {
            return Some(format!(
                "Auf {l}: liegt der Ordner {KEY_FOLDER} (Schlüssel-Sicherung). Dieser Datenträger wird nie gelöscht."
            ));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let drive = exe
            .to_string_lossy()
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase().to_string());
        if let Some(drive) = drive {
            if d.letters.iter().any(|l| l.eq_ignore_ascii_case(&drive)) {
                return Some("RAMForge-USB läuft von diesem Datenträger. Bitte von einem anderen Laufwerk starten.".into());
            }
        }
    }
    if d.size < 1_500_000_000 {
        return Some("Zu klein: mindestens 2 GB nötig.".into());
    }
    None
}

/// Boot image variants, largest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Variant {
    /// GPU firmware, Vulkan, GPU benchmark.
    Voll,
    /// RAM test, sensors, CPU/RAM benchmarks, software graphics.
    Kompakt,
    /// Only the RAM test (plus sensors and SPD), no network: smallest.
    Ram,
}

impl Variant {
    /// Value written to RAMForge\stick.json.
    pub fn key(self) -> &'static str {
        match self {
            Variant::Voll => "voll",
            Variant::Kompakt => "kompakt",
            Variant::Ram => "ram",
        }
    }
}

/// Variant from the file name: RAMForge-Boot-[Kompakt-|RAM-]<version>.iso.
pub fn variant_of(iso: &Path) -> Variant {
    let n = iso
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if n.contains("kompakt") {
        Variant::Kompakt
    } else if n.starts_with("ramforge-boot-ram-") {
        Variant::Ram
    } else {
        Variant::Voll
    }
}

/// Boot images shipped next to this program: one per variant (newest version),
/// largest variant first.
pub fn find_isos() -> Vec<(PathBuf, Variant)> {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    else {
        return vec![];
    };
    let mut isos: Vec<(PathBuf, Variant)> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let n = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    n.starts_with("ramforge-boot-") && n.ends_with(".iso")
                })
                .map(|p| {
                    let v = variant_of(&p);
                    (p, v)
                })
                .collect()
        })
        .unwrap_or_default();
    // Newest version first within each variant, largest variant first.
    isos.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
    isos.dedup_by(|a, b| a.1 == b.1);
    isos
}

pub fn find_iso() -> Option<PathBuf> {
    find_isos().into_iter().next().map(|x| x.0)
}

pub fn iso_version(iso: &Path) -> String {
    let n = iso
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    n.trim_start_matches("RAMForge-Boot-")
        .trim_start_matches("ramforge-boot-")
        .trim_start_matches("Kompakt-")
        .trim_start_matches("kompakt-")
        .trim_start_matches("RAM-")
        .trim_start_matches("ram-")
        .to_string()
}

pub fn sha256_file(p: &Path, progress: &dyn Fn(u64)) -> Result<String, String> {
    let mut f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    let mut done = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        done += n as u64;
        progress(done);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Checks the ISO against its ".sha256" file if one lies next to it.
pub fn verify_iso(iso: &Path, progress: &dyn Fn(u64)) -> Result<bool, String> {
    let side = PathBuf::from(format!("{}.sha256", iso.display()));
    let Ok(text) = std::fs::read_to_string(&side) else {
        return Ok(false);
    };
    let want = text.split_whitespace().next().unwrap_or("").to_lowercase();
    let got = sha256_file(iso, progress)?;
    if got != want {
        return Err(format!(
            "Prüfsumme der ISO stimmt nicht ({}). Datei neu herunterladen.",
            side.display()
        ));
    }
    Ok(true)
}

/// Mounts the ISO read-only and returns its drive root ("X:\").
fn mount_iso(iso: &Path) -> Result<String, String> {
    let p = iso.display().to_string().replace('\'', "''");
    let l = powershell(&format!(
        "$i = Mount-DiskImage -ImagePath '{p}' -Access ReadOnly -StorageType ISO -PassThru -ErrorAction Stop; \
         Start-Sleep -Milliseconds 500; ($i | Get-Volume).DriveLetter"
    ))?;
    let l = l.trim();
    if l.len() != 1 {
        return Err(format!("ISO konnte nicht eingebunden werden ({l})"));
    }
    Ok(format!("{l}:\\"))
}

fn unmount_iso(iso: &Path) {
    let p = iso.display().to_string().replace('\'', "''");
    let _ = powershell(&format!("Dismount-DiskImage -ImagePath '{p}' | Out-Null"));
}

/// Erases disk `n` and creates one FAT32 partition. Returns its root ("X:\").
fn partition_and_format(n: u32, size: u64) -> Result<String, String> {
    let mb = size / (1024 * 1024);
    let create = if mb > MAX_FAT32_MB + 16 {
        format!("create partition primary size={MAX_FAT32_MB}")
    } else {
        "create partition primary".into()
    };
    let script = format!(
        "select disk {n}\nattributes disk clear readonly noerr\nclean\nconvert mbr\n{create}\nformat fs=fat32 quick label={VOLUME_LABEL}\nactive\nassign\nexit\n"
    );
    let path = std::env::temp_dir().join(format!("ramforge-diskpart-{}.txt", std::process::id()));
    std::fs::write(&path, script).map_err(|e| e.to_string())?;
    let out = Command::new("diskpart")
        .arg("/s")
        .arg(&path)
        .output()
        .map_err(|e| format!("diskpart: {e}"))?;
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        return Err(format!(
            "diskpart fehlgeschlagen (Administratorrechte?):\n{text}"
        ));
    }
    // Windows needs a moment to mount the new volume.
    for _ in 0..30 {
        if let Ok(d) = disk_by_number(n) {
            if let Some(l) = d.letters.first() {
                let root = format!("{l}:\\");
                if Path::new(&root).exists() {
                    return Ok(root);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err("Die neue Partition hat keinen Laufwerksbuchstaben bekommen".into())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

/// Copies every file of the ISO to the stick (overwriting), returns bytes copied.
fn copy_tree(
    src_root: &Path,
    dst_root: &Path,
    progress: &dyn Fn(u64, u64, &str),
) -> Result<u64, String> {
    let mut files = Vec::new();
    walk(src_root, &mut files).map_err(|e| format!("ISO lesen: {e}"))?;
    let total: u64 = files
        .iter()
        .filter_map(|f| f.metadata().ok())
        .map(|m| m.len())
        .sum();
    let mut done = 0u64;
    for f in &files {
        let rel = f.strip_prefix(src_root).map_err(|e| e.to_string())?;
        let dst = dst_root.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        if dst.exists() {
            // ISO files are read-only; their copies on the stick must be replaceable later.
            let mut perm = std::fs::metadata(&dst)
                .map_err(|e| e.to_string())?
                .permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(&dst, perm);
        }
        let n = std::fs::copy(f, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        let mut perm = std::fs::metadata(&dst)
            .map_err(|e| e.to_string())?
            .permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = std::fs::set_permissions(&dst, perm);
        done += n;
        progress(done, total, &rel.display().to_string());
    }
    Ok(done)
}

/// Compares size and SHA-256 of the big system image after copying.
fn verify_copy(src_root: &Path, dst_root: &Path) -> Result<(), String> {
    let rel = Path::new("live").join("filesystem.squashfs");
    let a = sha256_file(&src_root.join(&rel), &|_| {})?;
    let b = sha256_file(&dst_root.join(&rel), &|_| {})?;
    if a != b {
        return Err("Prüfung nach dem Kopieren fehlgeschlagen: live/filesystem.squashfs weicht ab (Stick defekt?)".into());
    }
    if !dst_root
        .join("EFI")
        .join("boot")
        .join("bootx64.efi")
        .exists()
    {
        return Err("EFI/boot/bootx64.efi fehlt nach dem Kopieren".into());
    }
    Ok(())
}

pub struct SetupInput<'a> {
    pub disk: u32,
    /// Serial and size seen when the user confirmed; must still match.
    pub expect_serial: Option<String>,
    pub expect_size: u64,
    pub iso: &'a Path,
    pub main_exe: &'a Path,
    pub usb_exe: &'a Path,
    /// Optional RAMForge-GPU.exe to put next to the Windows programs.
    pub gpu_exe: Option<&'a Path>,
    pub allow_virtual: bool,
}

/// Erases the disk and installs the boot edition. Returns the new root.
pub fn setup(i: &SetupInput, progress: &dyn Fn(f32, &str)) -> Result<String, String> {
    // Fresh checks right before erasing: the stick could have been swapped.
    let d = disk_by_number(i.disk)?;
    if d.serial_number != i.expect_serial || d.size != i.expect_size {
        return Err(
            "Der Datenträger hat sich seit der Bestätigung geändert. Abgebrochen, nichts gelöscht."
                .into(),
        );
    }
    if let Some(why) = refusal(&d, i.allow_virtual) {
        return Err(format!("Abgebrochen, nichts gelöscht: {why}"));
    }
    progress(0.02, "ISO wird geprüft …");
    let iso_len = std::fs::metadata(i.iso)
        .map(|m| m.len())
        .unwrap_or(1)
        .max(1);
    verify_iso(i.iso, &|done| {
        progress(
            0.02 + 0.08 * done as f32 / iso_len as f32,
            "ISO wird geprüft …",
        )
    })?;
    progress(0.10, "Stick wird gelöscht und mit FAT32 formatiert …");
    let root = partition_and_format(i.disk, d.size)?;
    progress(0.15, "ISO wird eingebunden …");
    let iso_root = mount_iso(i.iso)?;
    let res = (|| {
        copy_tree(
            Path::new(&iso_root),
            Path::new(&root),
            &|done, total, name| {
                progress(
                    0.15 + 0.7 * done as f32 / total.max(1) as f32,
                    &format!("Kopiere {name}"),
                )
            },
        )?;
        progress(0.86, "Kopie wird geprüft …");
        verify_copy(Path::new(&iso_root), Path::new(&root))
    })();
    unmount_iso(i.iso);
    res?;
    progress(0.95, "Windows-Programme und Kennung …");
    let rf = Path::new(&root).join("RAMForge");
    std::fs::create_dir_all(rf.join("RAMForge-data")).map_err(|e| e.to_string())?;
    std::fs::copy(i.main_exe, rf.join("RAMForge.exe")).map_err(|e| format!("RAMForge.exe: {e}"))?;
    std::fs::copy(i.usb_exe, rf.join("RAMForge-USB.exe"))
        .map_err(|e| format!("RAMForge-USB.exe: {e}"))?;
    if let Some(g) = i.gpu_exe {
        std::fs::copy(g, rf.join("RAMForge-GPU.exe"))
            .map_err(|e| format!("RAMForge-GPU.exe: {e}"))?;
    }
    let marker = serde_json::json!({
        "version": iso_version(i.iso),
        "edition": "boot",
        "variant": variant_of(i.iso).key(),
        "installed": chrono::Local::now().format("%Y-%m-%d").to_string(),
    });
    std::fs::write(
        rf.join("stick.json"),
        serde_json::to_string_pretty(&marker).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    progress(1.0, "Fertig");
    Ok(root)
}

/// Updates the boot files of an existing RAMForge boot stick in place: no
/// formatting; RAMForge-data and every foreign file outside the boot folders
/// stay untouched.
pub fn update_in_place(
    root: &Path,
    iso: &Path,
    progress: &dyn Fn(f32, &str),
) -> Result<(), String> {
    let iso_root = mount_iso(iso)?;
    let res = (|| {
        // Stale files only inside the folders that belong to the boot system.
        for dir in ["live", "boot", "EFI", "isolinux", ".disk"] {
            let src = Path::new(&iso_root).join(dir);
            let dst = root.join(dir);
            if !src.exists() || !dst.exists() {
                continue;
            }
            let mut old = Vec::new();
            walk(&dst, &mut old).map_err(|e| e.to_string())?;
            for f in old {
                let rel = f.strip_prefix(root).map_err(|e| e.to_string())?;
                if !Path::new(&iso_root).join(rel).exists() {
                    let _ = std::fs::remove_file(&f);
                }
            }
        }
        copy_tree(Path::new(&iso_root), root, &|done, total, name| {
            progress(
                done as f32 / total.max(1) as f32 * 0.9,
                &format!("Kopiere {name}"),
            )
        })?;
        verify_copy(Path::new(&iso_root), root)
    })();
    unmount_iso(iso);
    res?;
    let marker = root.join("RAMForge").join("stick.json");
    let m = serde_json::json!({"version": iso_version(iso), "edition": "boot", "installed": chrono::Local::now().format("%Y-%m-%d").to_string()});
    std::fs::write(marker, serde_json::to_string_pretty(&m).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    progress(1.0, "Fertig");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(bus: &str, letters: &[&str]) -> DiskInfo {
        DiskInfo {
            number: 9,
            friendly_name: "Test".into(),
            serial_number: Some("X".into()),
            bus_type: bus.into(),
            size: 16_000_000_000,
            is_system: false,
            is_boot: false,
            letters: letters.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn variant_and_version_from_file_name() {
        for (name, v, ver) in [
            ("RAMForge-Boot-0.3.0.iso", Variant::Voll, "0.3.0"),
            ("RAMForge-Boot-Kompakt-0.3.0.iso", Variant::Kompakt, "0.3.0"),
            ("RAMForge-Boot-RAM-0.3.0.iso", Variant::Ram, "0.3.0"),
        ] {
            assert_eq!(variant_of(Path::new(name)), v, "{name}");
            assert_eq!(iso_version(Path::new(name)), ver, "{name}");
        }
    }

    #[test]
    fn refuses_non_usb_system_and_small_disks() {
        assert!(refusal(&disk("NVMe", &[]), false).is_some());
        assert!(
            refusal(&disk("SATA", &[]), true).is_some(),
            "virtuell erlaubt heißt nicht SATA erlaubt"
        );
        assert!(refusal(&disk("File Backed Virtual", &[]), false).is_some());
        assert!(refusal(&disk("File Backed Virtual", &[]), true).is_none());
        let mut d = disk("USB", &[]);
        d.is_system = true;
        assert!(refusal(&d, false).is_some());
        let mut d = disk("USB", &[]);
        d.size = 1_000_000_000;
        assert!(refusal(&d, false).is_some());
        assert!(refusal(&disk("USB", &[]), false).is_none());
    }

    #[test]
    fn refuses_disk_with_key_folder() {
        // A temp dir stands in for a drive root: substitute a drive letter.
        let base = std::env::temp_dir().join(format!("rf-keytest-{}", std::process::id()));
        std::fs::create_dir_all(base.join(KEY_FOLDER)).unwrap();
        let out = Command::new("cmd")
            .args(["/C", "subst", "R:", &base.display().to_string()])
            .output();
        if out.map(|o| o.status.success()).unwrap_or(false) {
            let why = refusal(&disk("USB", &["R"]), false);
            let _ = Command::new("cmd")
                .args(["/C", "subst", "R:", "/D"])
                .output();
            assert!(why.unwrap().contains(KEY_FOLDER));
        }
        std::fs::remove_dir_all(base).ok();
    }
}
