//! RAMForge-USB – prepares a USB stick with the RAMForge boot edition and
//! updates it in place.
//!
//! STATUS (0.1):
//! * lists removable drives and rescans every 2 s (plugging in is noticed),
//! * recognises RAMForge sticks by their marker file and shows whether the
//!   boot edition on it is up to date,
//! * loads the signed manifest only when a stick is selected or a RAMForge
//!   stick is plugged in, never at program start; the same manifest also
//!   tells whether RAMForge-USB.exe itself has an update.
//!
//! * copies the PORTABLE edition (RAMForge.exe + RAMForge-USB.exe) into
//!   `X:\RAMForge\` after a confirmation. This only adds files: nothing is
//!   deleted or formatted. Updating a portable stick downloads the new EXEs
//!   through the same signed chain (manifest signature + SHA-256).
//!
//! Making a stick BOOTABLE (partitioning, erases the stick) is deliberately
//! not implemented yet: it ships with the boot edition (roadmap 0.3) and will
//! need a double confirmation.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bootsetup;

use eframe::egui::{self, Color32, RichText};
use ramforge_update::ui::{UiEvent, UpdateUi};
use ramforge_update::{Manifest, StickState};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DISCLAIMER_MD: &str = include_str!("../../../DISCLAIMER.md");
const DISCLAIMER_VERSION: u32 = 1;
const ACCENT: Color32 = Color32::from_rgb(0xF2, 0x6B, 0x2A);
const OK: Color32 = Color32::from_rgb(0x2E, 0xB8, 0x6A);
const WARN: Color32 = Color32::from_rgb(0xE8, 0x9A, 0x2C);
const BAD: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
/// Marker file on a prepared stick (on the FAT32 boot partition).
const STICK_MARKER: &str = r"RAMForge\stick.json";
const RESCAN: Duration = Duration::from_secs(2);

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    disclaimer_accepted: u32,
    check_updates: Option<bool>,
    skipped_update: String,
}

#[derive(Serialize, Deserialize)]
struct StickInfo {
    version: String,
    /// "portable" (EXE files only) or "boot" (bootable edition).
    #[serde(default = "default_edition")]
    edition: String,
    #[serde(default)]
    installed: String,
}

fn default_edition() -> String {
    "boot".into()
}

#[derive(Clone, Debug, PartialEq)]
struct Drive {
    root: String,
    label: String,
    fs: String,
    size: u64,
    free: u64,
    /// Version of RAMForge on the stick, if it is one.
    ramforge: Option<String>,
    edition: String,
}

impl Drive {
    fn is_ventoy(&self) -> bool {
        let l = self.label.to_lowercase();
        l == "ventoy" || l == "vtoyefi"
    }
}

#[cfg(windows)]
fn list_removable() -> Vec<Drive> {
    use windows_sys::Win32::Storage::FileSystem::*;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let text = |b: &[u16]| {
        String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])
    };
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let w = wide(&root);
        // DRIVE_REMOVABLE = 2. USB SSDs that report as fixed disks follow in 0.3.
        if unsafe { GetDriveTypeW(w.as_ptr()) } != 2 {
            continue;
        }
        let mut name = [0u16; 261];
        let mut fsname = [0u16; 261];
        let (mut serial, mut maxlen, mut flags) = (0u32, 0u32, 0u32);
        let ok = unsafe {
            GetVolumeInformationW(
                w.as_ptr(),
                name.as_mut_ptr(),
                261,
                &mut serial,
                &mut maxlen,
                &mut flags,
                fsname.as_mut_ptr(),
                261,
            )
        } != 0;
        if !ok {
            continue; // card reader without medium
        }
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) };
        // Read only. This tool never writes to a drive in this version.
        let info = std::fs::read_to_string(format!("{root}{STICK_MARKER}"))
            .ok()
            .and_then(|s| serde_json::from_str::<StickInfo>(s.trim_start_matches('\u{feff}')).ok());
        let (ramforge, edition) = match info {
            Some(i) => (Some(i.version), i.edition),
            None => (None, String::new()),
        };
        out.push(Drive {
            root,
            label: text(&name),
            fs: text(&fsname),
            size: total,
            free: avail,
            ramforge,
            edition,
        });
    }
    out
}

#[cfg(not(windows))]
fn list_removable() -> Vec<Drive> {
    Vec::new()
}

/// The RAMForge.exe shipped next to this tool.
fn main_exe() -> Result<PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = me.parent().ok_or("kein Programmordner")?;
    ["RAMForge.exe", "ramforge.exe"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!(
                "RAMForge.exe liegt nicht neben RAMForge-USB.exe ({})",
                dir.display()
            )
        })
}

/// RAMForge-GPU.exe next to this tool, if shipped.
fn gpu_exe() -> Option<PathBuf> {
    let p = std::env::current_exe()
        .ok()?
        .parent()?
        .join("RAMForge-GPU.exe");
    p.is_file().then_some(p)
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Copies via a temporary name and renames, so a cancelled copy never leaves
/// a half-written EXE behind.
fn copy_replace(src: &std::path::Path, dest: &std::path::Path) -> Result<u64, String> {
    if same_file(src, dest) {
        return Ok(0); // running from the stick itself: already there
    }
    let tmp = dest.with_extension("exe.part");
    let n = std::fs::copy(src, &tmp).map_err(|e| format!("{}: {e}", dest.display()))?;
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!(
            "{} ersetzen: {e} (läuft das Programm gerade vom Stick?)",
            dest.display()
        )
    })?;
    Ok(n)
}

/// The only folder this tool ever writes to on a stick. Everything else on
/// the stick (other folders such as key backups, BIOS files, …) is never
/// listed, changed or deleted. Formatting is not implemented at all.
const STICK_DIR: &str = "RAMForge";

/// Path of a file directly inside `<root>RAMForge\`. Refuses every name that
/// could point elsewhere (separators, drive prefixes, "." and "..").
fn stick_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let bad = name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']);
    if bad {
        return Err(format!("unzulässiger Dateiname auf dem Stick: {name:?}"));
    }
    Ok(root.join(STICK_DIR).join(name))
}

fn write_marker(root: &Path, version: &str) -> Result<(), String> {
    let info = StickInfo {
        version: version.into(),
        edition: "portable".into(),
        installed: chrono::Local::now().format("%Y-%m-%d").to_string(),
    };
    let path = stick_path(root, "stick.json")?;
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&info).unwrap_or_default(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

/// Installs the portable edition into `<root>RAMForge\`. Only adds or replaces
/// RAMForge.exe, RAMForge-USB.exe and stick.json in that folder.
fn install_portable(root: &str) -> Result<String, String> {
    let main = main_exe()?;
    let usb = std::env::current_exe().map_err(|e| e.to_string())?;
    let need = std::fs::metadata(&main).map(|m| m.len()).unwrap_or(0)
        + std::fs::metadata(&usb).map(|m| m.len()).unwrap_or(0);
    if let Some(d) = list_removable()
        .into_iter()
        .find(|d| d.root.eq_ignore_ascii_case(root))
    {
        if d.free > 0 && d.free < need + (1 << 20) {
            return Err(format!(
                "Zu wenig Platz auf {root}: {} nötig, {} frei",
                gb(need),
                gb(d.free)
            ));
        }
    }
    install_portable_from(Path::new(root), &main, &usb)
}

fn install_portable_from(root: &Path, main: &Path, usb: &Path) -> Result<String, String> {
    let dir = root.join(STICK_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let a = copy_replace(main, &stick_path(root, "RAMForge.exe")?)?;
    let b = copy_replace(usb, &stick_path(root, "RAMForge-USB.exe")?)?;
    if let Some(g) = gpu_exe() {
        copy_replace(&g, &stick_path(root, "RAMForge-GPU.exe")?)?;
    }
    write_marker(root, env!("CARGO_PKG_VERSION"))?;
    Ok(format!(
        "RAMForge {} portabel nach {} kopiert ({:.1} MB). Start: {}",
        env!("CARGO_PKG_VERSION"),
        dir.display(),
        (a + b) as f64 / 1e6,
        dir.join("RAMForge.exe").display()
    ))
}

/// Updates a portable stick from the verified manifest: every EXE is checked
/// against the SHA-256 from the signed manifest before it replaces the old one.
fn update_portable(root: &str, m: &Manifest, progress: &dyn Fn(String)) -> Result<String, String> {
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let download =
        |asset: &ramforge_update::Asset, dest: &Path, label: &str| -> Result<String, String> {
            // Tries GitHub first, then the mirror; nothing on the stick changes unless
            // one source delivered exactly the signed bytes.
            ramforge_update::download(
                asset,
                dest,
                &|done, total| {
                    progress(if total > 0 {
                        format!("{label}: {:.0} %", done as f64 * 100.0 / total as f64)
                    } else {
                        format!("{label}: {:.1} MB", done as f64 / 1e6)
                    })
                },
                &cancel,
            )
            .map(|got| got.source)
            .map_err(|e| e.to_string())
        };
    update_portable_with(Path::new(root), m, &download)
}

type Downloader<'a> = dyn Fn(&ramforge_update::Asset, &Path, &str) -> Result<String, String> + 'a;

fn update_portable_with(
    root: &Path,
    m: &Manifest,
    download: &Downloader,
) -> Result<String, String> {
    let mut sources = Vec::new();
    for (kind, name) in [
        (ramforge_update::AssetKind::WindowsPortable, "RAMForge.exe"),
        (ramforge_update::AssetKind::UsbTool, "RAMForge-USB.exe"),
    ] {
        let Some(asset) = m.assets.get(kind.key()) else {
            continue;
        };
        let tmp = stick_path(root, &format!("{name}.new"))?;
        let source = download(asset, &tmp, name).map_err(|e| format!("{name}: {e}"))?;
        sources.push(format!("{name} (Quelle: {source})"));
        std::fs::rename(&tmp, stick_path(root, name)?).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{name} ersetzen: {e}")
        })?;
    }
    write_marker(root, &m.version)?;
    Ok(format!(
        "Stick {} auf Version {} aktualisiert: {} – Signatur und SHA-256 geprüft",
        root.display(),
        m.version,
        sources.join(", ")
    ))
}

#[derive(Clone)]
enum Job {
    Idle,
    Running(String),
    Done(String),
    Failed(String),
}

fn data_dir() -> PathBuf {
    let portable = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("RAMForge-data")));
    let fallback = std::env::var("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("RAMForge"))
        .unwrap_or_else(|_| std::env::temp_dir());
    for d in portable.into_iter().chain(Some(fallback)) {
        if std::fs::create_dir_all(&d).is_ok()
            && std::fs::write(d.join(".write-test"), b"ok").is_ok()
        {
            let _ = std::fs::remove_file(d.join(".write-test"));
            return d;
        }
    }
    std::env::temp_dir()
}

fn gb(b: u64) -> String {
    if b == 0 {
        "–".into()
    } else {
        format!("{:.1} GB", b as f64 / 1e9)
    }
}

fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// The signed manifest, loaded on demand.
#[derive(Clone)]
enum Online {
    NotLoaded,
    Loading,
    Ready(Manifest),
    Failed(String),
}

struct UsbApp {
    settings_path: PathBuf,
    settings: Settings,
    drives: Vec<Drive>,
    last_scan: Instant,
    selected: Option<String>,
    accept_check: bool,
    online: Arc<Mutex<Online>>,
    tool_update_offered: bool,
    updater: UpdateUi,
    log: Vec<String>,
    job: Arc<Mutex<Job>>,
    confirm_install: Option<String>,
    /// Physical disk behind the selected drive (looked up in the background).
    disk_check: Arc<Mutex<Option<(String, Result<bootsetup::DiskInfo, String>)>>>,
    /// Erase dialog: disk and what the user typed.
    erase: Option<(bootsetup::DiskInfo, String, bool)>,
    boot_iso: Option<PathBuf>,
    /// Copy RAMForge-GPU.exe too (only offered if it lies next to this tool).
    include_gpu: bool,
    allow_virtual: bool,
    auto_shot: Option<(PathBuf, Instant, bool, Arc<std::sync::atomic::AtomicBool>)>,
}

impl UsbApp {
    fn new() -> Self {
        let settings_path = data_dir().join("usb-settings.json");
        // Tolerates a UTF-8 BOM (Notepad, PowerShell).
        let settings: Settings = std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default();
        let mut app = Self {
            settings_path,
            settings,
            drives: list_removable(),
            last_scan: Instant::now(),
            selected: None,
            accept_check: false,
            online: Arc::new(Mutex::new(Online::NotLoaded)),
            tool_update_offered: false,
            updater: UpdateUi::new(
                "RAMForge-USB",
                ramforge_update::AssetKind::UsbTool,
                env!("CARGO_PKG_VERSION"),
            ),
            log: Vec::new(),
            job: Arc::new(Mutex::new(Job::Idle)),
            confirm_install: None,
            disk_check: Arc::new(Mutex::new(None)),
            erase: None,
            boot_iso: arg_value("--iso")
                .map(PathBuf::from)
                .or_else(bootsetup::find_iso),
            include_gpu: gpu_exe().is_some(),
            allow_virtual: std::env::args().any(|a| a == "--allow-virtual-disk"),
            auto_shot: arg_value("--screenshot").map(|p| {
                (
                    PathBuf::from(p),
                    Instant::now(),
                    false,
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                )
            }),
        };
        if let Some(root) = arg_value("--select") {
            app.selected = app
                .drives
                .iter()
                .find(|d| d.root.eq_ignore_ascii_case(&root))
                .map(|d| d.root.clone());
        }
        app
    }

    fn persist(&self) {
        if let Ok(s) = serde_json::to_string_pretty(&self.settings) {
            let _ = std::fs::write(&self.settings_path, s);
        }
    }

    fn online_enabled(&self) -> bool {
        self.settings.check_updates.unwrap_or(true)
    }

    /// Loads the signed manifest in the background. `retry` also retries after a failure.
    fn ensure_manifest(&mut self, retry: bool) {
        if !self.online_enabled() {
            return;
        }
        let mut st = self.online.lock().unwrap();
        let start = match &*st {
            Online::NotLoaded => true,
            Online::Failed(_) => retry,
            Online::Loading | Online::Ready(_) => false,
        };
        if !start {
            return;
        }
        *st = Online::Loading;
        drop(st);
        self.log
            .push("Lade aktuelle Versionsdaten von ramforge.org …".into());
        let online = self.online.clone();
        std::thread::spawn(move || {
            let res = ramforge_update::fetch_verified_manifest();
            *online.lock().unwrap() = match res {
                Ok(m) => Online::Ready(m),
                Err(e) => Online::Failed(e.to_string()),
            };
        });
    }

    fn rescan(&mut self) {
        let before: Vec<String> = self.drives.iter().map(|d| d.root.clone()).collect();
        self.drives = list_removable();
        for d in &self.drives {
            if !before.contains(&d.root) {
                self.log.push(format!(
                    "Laufwerk {} erkannt ({})",
                    d.root,
                    if d.label.is_empty() {
                        "ohne Namen"
                    } else {
                        &d.label
                    }
                ));
            }
        }
        if let Some(sel) = &self.selected {
            if !self.drives.iter().any(|d| &d.root == sel) {
                self.log.push(format!("Laufwerk {sel} wurde entfernt"));
                self.selected = None;
            }
        }
        self.last_scan = Instant::now();
    }

    fn job_running(&self) -> bool {
        matches!(&*self.job.lock().unwrap(), Job::Running(_))
    }

    fn start_install(&mut self, root: String) {
        let job = self.job.clone();
        *job.lock().unwrap() = Job::Running(format!("Kopiere nach {root}RAMForge …"));
        std::thread::spawn(move || {
            let r = install_portable(&root);
            *job.lock().unwrap() = match r {
                Ok(m) => Job::Done(m),
                Err(e) => Job::Failed(e),
            };
        });
    }

    fn start_update(&mut self, root: String, m: Manifest) {
        let job = self.job.clone();
        *job.lock().unwrap() = Job::Running("Lade Update …".into());
        std::thread::spawn(move || {
            let j2 = job.clone();
            let r = update_portable(&root, &m, &move |t| *j2.lock().unwrap() = Job::Running(t));
            *job.lock().unwrap() = match r {
                Ok(m) => Job::Done(m),
                Err(e) => Job::Failed(e),
            };
        });
    }

    /// Status text and colour for one drive.
    fn status(&self, d: &Drive) -> (String, Color32, String) {
        let Some(installed) = &d.ramforge else {
            return ("kein RAMForge-Stick".into(), Color32::GRAY, String::new());
        };
        let (text, color, tip) = self.version_status(installed);
        let prefix = if d.edition == "portable" {
            "portabel · "
        } else {
            "bootfähig · "
        };
        (format!("{prefix}{text}"), color, tip)
    }

    fn version_status(&self, installed: &str) -> (String, Color32, String) {
        match &*self.online.lock().unwrap() {
            Online::NotLoaded if !self.online_enabled() => (
                format!("Version {installed}"),
                Color32::GRAY,
                "Online-Prüfung ist ausgeschaltet".into(),
            ),
            Online::NotLoaded | Online::Loading => (
                format!("Version {installed} · prüfe …"),
                Color32::GRAY,
                String::new(),
            ),
            Online::Failed(e) => (
                format!("Version {installed} · Prüfung nicht möglich"),
                WARN,
                e.clone(),
            ),
            Online::Ready(m) => match ramforge_update::stick_state(m, installed) {
                StickState::UpToDate { installed } => {
                    (format!("aktuell ({installed})"), OK, String::new())
                }
                StickState::Outdated {
                    installed,
                    latest,
                    urgent,
                } => (
                    format!("veraltet: {installed} → {latest}"),
                    if urgent { BAD } else { WARN },
                    if urgent {
                        "Diese Version wird nicht mehr unterstützt".into()
                    } else {
                        String::new()
                    },
                ),
                StickState::Unknown { installed } => (
                    format!("Version „{installed}“ unbekannt"),
                    WARN,
                    String::new(),
                ),
            },
        }
    }

    fn md(ui: &mut egui::Ui, md: &str) {
        for line in md.lines() {
            let l = line.trim_end().replace("**", "");
            if let Some(h) = l.strip_prefix("## ") {
                ui.label(RichText::new(h).size(17.0).strong().color(ACCENT));
            } else if let Some(h) = l.strip_prefix("### ") {
                ui.label(RichText::new(h).strong());
            } else if let Some(h) = l.strip_prefix("# ") {
                ui.label(RichText::new(h).size(20.0).strong());
            } else if let Some(b) = l.strip_prefix("- ") {
                ui.label(format!("  •  {b}"));
            } else if l != "---" {
                ui.label(l.trim_start_matches("> "));
            }
        }
    }

    fn detail(&mut self, ui: &mut egui::Ui) {
        let Some(d) = self
            .selected
            .as_ref()
            .and_then(|s| self.drives.iter().find(|d| &d.root == s))
            .cloned()
        else {
            ui.label(RichText::new("Stick in der Liste auswählen. Erst dann werden die aktuellen Versionsdaten geladen.").weak());
            return;
        };
        egui::Frame::group(ui.style()).inner_margin(12.0).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(format!("{}  {}", d.root, d.label)).size(17.0).strong());
            ui.label(RichText::new(format!("{} · {} gesamt · {} frei", d.fs, gb(d.size), gb(d.free))).weak());
            ui.add_space(6.0);
            let (text, color, tip) = self.status(&d);
            ui.label(RichText::new(text).size(20.0).strong().color(color)).on_hover_text(tip);
            let online = self.online.lock().unwrap().clone();
            match (&d.ramforge, &online) {
                (None, _) => {
                    ui.label("Auf diesem Stick ist RAMForge noch nicht eingerichtet.");
                    if d.is_ventoy() {
                        ui.label(RichText::new(
                            "Ventoy-Stick erkannt: Die Datei RAMForge-Boot-….iso kann auch einfach auf diesen Stick kopiert werden, ohne ihn neu einzurichten (Ventoy zeigt sie dann im Startmenü).",
                        ).color(ACCENT));
                    }
                }
                (Some(_), Online::Ready(m)) => {
                    ui.label(format!(
                        "Neueste Boot-Version: {}{}",
                        m.version,
                        if m.released.is_empty() { String::new() } else { format!(" (veröffentlicht {})", m.released) }
                    ));
                    if !m.notes_url.is_empty() {
                        ui.hyperlink_to("Was ist neu?", &m.notes_url);
                    }
                }
                (Some(_), Online::Failed(e)) => {
                    ui.label(RichText::new(format!("Aktualität nicht prüfbar: {e}")).small());
                }
                _ => {}
            }
            ui.add_space(8.0);
            let busy = self.job_running();
            let outdated_manifest = match (&d.ramforge, &online) {
                (Some(v), Online::Ready(m)) if d.edition == "portable" => {
                    matches!(ramforge_update::stick_state(m, v), StickState::Outdated { .. }).then(|| m.clone())
                }
                _ => None,
            };
            ui.horizontal(|ui| {
                let copy_label = if d.ramforge.is_some() && d.edition == "portable" {
                    "⟳ Portable Version erneut kopieren"
                } else {
                    "▶ RAMForge portabel auf den Stick kopieren"
                };
                let copy = egui::Button::new(RichText::new(copy_label).strong()).fill(ACCENT);
                if ui
                    .add_enabled(!busy, copy)
                    .on_hover_text("Kopiert RAMForge.exe und RAMForge-USB.exe nach RAMForge\\ auf dem Stick. Nichts wird gelöscht oder formatiert.")
                    .clicked()
                {
                    self.confirm_install = Some(d.root.clone());
                }
                let upd = ui
                    .add_enabled(!busy && outdated_manifest.is_some(), egui::Button::new("Stick aktualisieren"))
                    .on_disabled_hover_text("Aktiv, wenn auf dem Stick eine ältere Version liegt und die Versionsdaten von ramforge.org geladen sind.");
                if upd.clicked() {
                    if let Some(m) = outdated_manifest.clone() {
                        self.start_update(d.root.clone(), m);
                    }
                }
                if ui.add_enabled(self.online_enabled(), egui::Button::new("⟳ Erneut prüfen")).clicked() {
                    *self.online.lock().unwrap() = Online::NotLoaded;
                    self.tool_update_offered = false;
                    self.ensure_manifest(true);
                }
            });
            ui.add_space(6.0);
            self.boot_section(ui, &d, busy);
            match self.job.lock().unwrap().clone() {
                Job::Running(t) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t);
                    });
                }
                Job::Done(t) => {
                    ui.label(RichText::new(t).color(OK));
                }
                Job::Failed(t) => {
                    ui.label(RichText::new(t).color(BAD));
                }
                Job::Idle => {}
            }
        });
    }
}

impl UsbApp {
    /// "Bootfähiger Stick": ISO status, disk checks, set-up and in-place update.
    fn boot_section(&mut self, ui: &mut egui::Ui, d: &Drive, busy: bool) {
        ui.separator();
        ui.label(
            RichText::new("Bootfähiger Stick (startet ohne Windows, auch mit Secure Boot)")
                .strong(),
        );
        let isos = bootsetup::find_isos();
        if isos.len() > 1 {
            ui.label("Variante:");
            for (p, v) in &isos {
                let mb = std::fs::metadata(p)
                    .map(|m| m.len() / 1_000_000)
                    .unwrap_or(0);
                let label = match v {
                    bootsetup::Variant::Voll => format!("Voll ({mb} MB): alles, zusätzlich Grafiktreiber und GPU-Test"),
                    bootsetup::Variant::Kompakt => format!("Kompakt ({mb} MB): RAM-Test, Sensoren, CPU/RAM-Benchmarks"),
                    bootsetup::Variant::Ram => format!("Nur RAM-Test ({mb} MB): RAM-Test, Sensoren, SPD – ohne Benchmarks und Netzwerk"),
                };
                if ui.radio(self.boot_iso.as_ref() == Some(p), label).clicked() {
                    self.boot_iso = Some(p.clone());
                }
            }
        }
        let ram_only =
            self.boot_iso.as_deref().map(bootsetup::variant_of) == Some(bootsetup::Variant::Ram);
        if gpu_exe().is_some() && !ram_only {
            ui.checkbox(
                &mut self.include_gpu,
                "GPU-Modul (RAMForge-GPU.exe, ≈ 6 MB) für Windows mitkopieren",
            );
        }
        let iso = self.boot_iso.clone();
        match &iso {
            Some(p) => {
                ui.label(
                    RichText::new(format!(
                        "Boot-Abbild: {} (Version {})",
                        p.file_name().unwrap_or_default().to_string_lossy(),
                        bootsetup::iso_version(p)
                    ))
                    .small(),
                );
            }
            None => {
                ui.label(
                    RichText::new("RAMForge-Boot-….iso liegt nicht neben RAMForge-USB.exe.")
                        .color(WARN),
                );
                ui.hyperlink_to(
                    "Boot-Abbild auf ramforge.org herunterladen",
                    "https://ramforge.org/download",
                );
            }
        }
        // Disk behind this drive letter, looked up once per selection.
        let current = self.disk_check.lock().unwrap().clone();
        let info = match current {
            Some((root, r)) if root == d.root => Some(r),
            _ => {
                let slot = self.disk_check.clone();
                let root = d.root.clone();
                *slot.lock().unwrap() = Some((root.clone(), Err("wird geprüft …".into())));
                std::thread::spawn(move || {
                    let r = bootsetup::disk_for_letter(&root);
                    *slot.lock().unwrap() = Some((root, r));
                });
                None
            }
        };
        let Some(info) = info else { return };
        let disk = match info {
            Ok(disk) => disk,
            Err(e) => {
                ui.label(RichText::new(format!("Datenträger: {e}")).small().weak());
                return;
            }
        };
        ui.label(
            RichText::new(format!(
                "Datenträger {}: {} · {} · {:.1} GB · Laufwerke {}",
                disk.number,
                disk.friendly_name,
                disk.bus_type,
                disk.size as f64 / 1e9,
                disk.letters
                    .iter()
                    .map(|l| format!("{l}:"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ))
            .small(),
        );
        let refusal = bootsetup::refusal(&disk, self.allow_virtual);
        if let Some(why) = &refusal {
            ui.label(RichText::new(format!("🔒 {why}")).color(BAD).strong());
        }
        ui.horizontal(|ui| {
            let can = !busy && iso.is_some() && refusal.is_none();
            let b = egui::Button::new(RichText::new("⚠ Bootfähig einrichten – LÖSCHT ALLE DATEN").strong().color(Color32::WHITE)).fill(BAD);
            if ui.add_enabled(can, b).on_disabled_hover_text("Nur für USB-Sticks ohne Schlüssel-Ordner, mit vorhandener Boot-ISO.").clicked() {
                self.erase = Some((disk.clone(), String::new(), false));
            }
            if d.edition == "boot" {
                let up = egui::Button::new("Boot-Dateien aktualisieren (ohne Formatieren)");
                if ui.add_enabled(!busy && iso.is_some(), up).clicked() {
                    if let Some(iso) = iso.clone() {
                        let job = self.job.clone();
                        let root = d.root.clone();
                        *job.lock().unwrap() = Job::Running("Boot-Dateien werden aktualisiert …".into());
                        std::thread::spawn(move || {
                            let j2 = job.clone();
                            let r = bootsetup::update_in_place(Path::new(&root), &iso, &move |f, t| {
                                *j2.lock().unwrap() = Job::Running(format!("{:.0} % · {t}", f * 100.0))
                            });
                            *job.lock().unwrap() = match r {
                                Ok(()) => Job::Done(format!("Boot-Dateien auf {root} aktualisiert, RAMForge-data blieb erhalten")),
                                Err(e) => Job::Failed(e),
                            };
                        });
                    }
                }
            }
        });
    }

    fn erase_dialog(&mut self, ctx: &egui::Context) {
        let Some((disk, typed, ack)) = self.erase.as_mut() else {
            return;
        };
        let mut close = false;
        let mut start = false;
        egui::Modal::new(egui::Id::unique("erase-confirm")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            ui.label(RichText::new("Stick endgültig löschen?").size(19.0).strong().color(BAD));
            ui.label(format!(
                "Datenträger {} · {} · {:.1} GB",
                disk.number,
                disk.friendly_name,
                disk.size as f64 / 1e9
            ));
            ui.label(RichText::new(format!(
                "Alle Partitionen und Daten auf {} werden unwiderruflich gelöscht. Danach enthält der Stick RAMForge (bootfähig, Secure Boot) und die portable Windows-Version.",
                disk.letters.iter().map(|l| format!("{l}:")).collect::<Vec<_>>().join(", ")
            )));
            ui.add_space(6.0);
            ui.checkbox(ack, "Ich habe geprüft, dass das der richtige Stick ist, und brauche keine Daten mehr darauf.");
            ui.horizontal(|ui| {
                ui.label("Zur Bestätigung LÖSCHEN eintippen:");
                ui.text_edit_singleline(typed);
            });
            ui.horizontal(|ui| {
                let ok = *ack && typed.trim() == "LÖSCHEN";
                if ui.add_enabled(ok, egui::Button::new(RichText::new("Löschen und einrichten").strong().color(Color32::WHITE)).fill(BAD)).clicked() {
                    start = true;
                }
                if ui.button("Abbrechen").clicked() {
                    close = true;
                }
            });
        });
        if start {
            let disk = disk.clone();
            self.erase = None;
            self.start_setup(disk);
        } else if close {
            self.erase = None;
        }
    }

    fn start_setup(&mut self, disk: bootsetup::DiskInfo) {
        let (Some(iso), Ok(main), Ok(usb)) =
            (self.boot_iso.clone(), main_exe(), std::env::current_exe())
        else {
            *self.job.lock().unwrap() =
                Job::Failed("Boot-ISO oder RAMForge.exe nicht gefunden".into());
            return;
        };
        let job = self.job.clone();
        let allow_virtual = self.allow_virtual;
        let ram_only = bootsetup::variant_of(&iso) == bootsetup::Variant::Ram;
        let gpu = if self.include_gpu && !ram_only {
            gpu_exe()
        } else {
            None
        };
        *job.lock().unwrap() = Job::Running("Wird vorbereitet …".into());
        std::thread::spawn(move || {
            let j2 = job.clone();
            let r = bootsetup::setup(
                &bootsetup::SetupInput {
                    disk: disk.number,
                    expect_serial: disk.serial_number.clone(),
                    expect_size: disk.size,
                    iso: &iso,
                    main_exe: &main,
                    usb_exe: &usb,
                    gpu_exe: gpu.as_deref(),
                    allow_virtual,
                },
                &move |f, t| {
                    *j2.lock().unwrap() = Job::Running(format!("{:.0} % · {t}", f * 100.0))
                },
            );
            *job.lock().unwrap() = match r {
                Ok(root) => Job::Done(format!(
                    "Fertig: {root} ist bootfähig. Im Bootmenü des PCs (oft F8, F11 oder F12) den Stick wählen – Secure Boot kann an bleiben."
                )),
                Err(e) => Job::Failed(e),
            };
        });
    }
}

impl eframe::App for UsbApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let accepted = self.settings.disclaimer_accepted >= DISCLAIMER_VERSION;
        let finished = match &*self.job.lock().unwrap() {
            Job::Done(t) | Job::Failed(t) => Some(t.clone()),
            _ => None,
        };
        if let Some(t) = finished {
            if self.log.last() != Some(&t) {
                self.log.push(t);
                self.rescan();
            }
        }
        if self.last_scan.elapsed() >= RESCAN {
            self.rescan();
        }
        if accepted {
            // A plugged-in RAMForge stick or a selected stick triggers the online check.
            if self.selected.is_some() || self.drives.iter().any(|d| d.ramforge.is_some()) {
                self.ensure_manifest(false);
            }
            let ready = match &*self.online.lock().unwrap() {
                Online::Ready(m) if !self.tool_update_offered => Some(m.clone()),
                _ => None,
            };
            if let Some(m) = ready {
                self.tool_update_offered = true;
                self.updater
                    .offer_from_manifest(m, &self.settings.skipped_update);
            }
            let failed = match &*self.online.lock().unwrap() {
                Online::Failed(e) => Some(e.clone()),
                _ => None,
            };
            if let Some(e) = failed {
                let line = format!("Versionsdaten nicht abrufbar: {e}");
                if self.log.last() != Some(&line) {
                    self.log.push(line);
                }
            }
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("RAM").size(24.0).strong().color(ACCENT));
                ui.add_space(-8.0);
                ui.label(RichText::new("Forge USB").size(24.0).strong());
                ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).weak());
            });
            ui.label("Richtet einen USB-Stick mit der bootfähigen RAMForge-Variante ein und hält ihn aktuell, ohne ihn neu zu formatieren.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("USB-Sticks").strong());
                ui.label(RichText::new("(wird alle 2 Sekunden aktualisiert)").small().weak());
            });
            if self.drives.is_empty() {
                ui.label(RichText::new("Kein USB-Stick gefunden. Stick einstecken, er erscheint automatisch.").weak());
            }
            let mut clicked = None;
            egui::Grid::new("drives").num_columns(5).striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
                for h in ["", "Laufwerk", "Größe / frei", "Dateisystem", "RAMForge"] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                for d in &self.drives {
                    let sel = self.selected.as_deref() == Some(&d.root);
                    if ui.radio(sel, "").clicked() {
                        clicked = Some(d.root.clone());
                    }
                    if ui.selectable_label(sel, format!("{} {}", d.root, d.label)).clicked() {
                        clicked = Some(d.root.clone());
                    }
                    ui.label(format!("{} / {}", gb(d.size), gb(d.free)));
                    ui.label(&d.fs);
                    let (text, color, tip) = self.status(d);
                    let r = ui.label(RichText::new(text).color(color));
                    if !tip.is_empty() {
                        r.on_hover_text(tip);
                    }
                    ui.end_row();
                }
            });
            if let Some(root) = clicked {
                self.selected = Some(root);
                if accepted {
                    self.ensure_manifest(true);
                }
            }
            ui.add_space(10.0);
            self.detail(ui);
            ui.add_space(10.0);
            ui.separator();
            let mut on = self.online_enabled();
            if ui.checkbox(&mut on, "Beim Auswählen eines Sticks online nach Updates suchen").changed() {
                self.settings.check_updates = Some(on);
                self.persist();
            }
            ui.label(RichText::new("Fingerabdrücke der Update-Schlüssel (SHA-256):").small());
            for (name, fp) in ramforge_update::key::fingerprints() {
                ui.label(RichText::new(format!("{name:<8} {fp}")).small().monospace());
            }
            if ramforge_update::key::is_placeholder() {
                ui.label(RichText::new("Entwicklungsversion: Test- bzw. Platzhalterschlüssel").small().color(WARN));
            }
            self.log.extend(self.updater.take_log().into_iter().map(|(_, m)| m));
            for l in self.log.iter().rev().take(6) {
                ui.label(RichText::new(l).small().weak());
            }
        });

        self.erase_dialog(&ctx);
        if let Some(root) = self.confirm_install.clone() {
            egui::Modal::new(egui::Id::unique("usb-confirm")).show(&ctx, |ui| {
                ui.set_max_width(520.0);
                ui.label(RichText::new(format!("RAMForge auf {root} kopieren?")).size(17.0).strong());
                ui.label(format!(
                    "Kopiert RAMForge.exe und RAMForge-USB.exe nach {root}RAMForge\\ und legt dort die Kennung stick.json an. \
                     Vorhandene Dateien auf dem Stick bleiben unangetastet, nichts wird formatiert."
                ));
                ui.label(RichText::new("Danach RAMForge an jedem Windows-PC direkt vom Stick starten. Berichte und Logs landen in RAMForge\\RAMForge-data.").small().weak());
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new("Kopieren").fill(ACCENT)).clicked() {
                        self.confirm_install = None;
                        self.start_install(root.clone());
                    }
                    if ui.button("Abbrechen").clicked() {
                        self.confirm_install = None;
                    }
                });
            });
        }
        if !accepted {
            egui::Modal::new(egui::Id::unique("usb-disclaimer")).show(&ctx, |ui| {
                ui.set_max_width(720.0);
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| Self::md(ui, DISCLAIMER_MD));
                ui.separator();
                ui.checkbox(&mut self.accept_check, "Ich habe den Haftungsausschluss gelesen und akzeptiere ihn. / I accept the disclaimer.");
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.accept_check, egui::Button::new("Akzeptieren / Accept")).clicked() {
                        self.settings.disclaimer_accepted = DISCLAIMER_VERSION;
                        self.persist();
                    }
                    if ui.button("Beenden / Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        } else {
            match self.updater.show(&ctx, false) {
                UiEvent::Skip(v) => {
                    self.settings.skipped_update = v;
                    self.persist();
                }
                UiEvent::Restart => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                UiEvent::None => {}
            }
        }
        ctx.request_repaint_after(Duration::from_millis(500));

        // `--screenshot <file>`: wait until the online check settled, save, exit.
        let settled = !matches!(&*self.online.lock().unwrap(), Online::Loading);
        if let Some((path, start, requested, done)) = &mut self.auto_shot {
            ctx.request_repaint_after(Duration::from_millis(100));
            if done.load(std::sync::atomic::Ordering::Acquire) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if !*requested && settled && start.elapsed() > Duration::from_secs(4) {
                *requested = true;
                let (path, done) = (path.clone(), done.clone());
                ctx.request_screenshot(move |img| {
                    let bytes: Vec<u8> = img
                        .pixels
                        .iter()
                        .flat_map(|c| c.to_srgba_unmultiplied())
                        .collect();
                    let _ = save_png(&path, img.size[0] as u32, img.size[1] as u32, bytes);
                    done.store(true, std::sync::atomic::Ordering::Release);
                });
            }
        }
    }
}

fn save_png(path: &std::path::Path, w: u32, h: u32, rgba: Vec<u8>) -> Option<()> {
    image::RgbaImage::from_raw(w, h, rgba)?.save(path).ok()
}

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
        println!("RAMForge-USB {}", env!("CARGO_PKG_VERSION"));
        println!("Fingerabdrücke der Update-Schlüssel (SHA-256):");
        for (name, fp) in ramforge_update::key::fingerprints() {
            println!("  {name:<8} {fp}");
        }
        return Ok(());
    }
    if let Some(n) = arg_value("--setup-disk") {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
        // Destructive: the disk number has to be repeated with --confirm-erase.
        let ok = arg_value("--confirm-erase").as_deref() == Some(n.as_str());
        let iso = arg_value("--iso")
            .map(PathBuf::from)
            .or_else(bootsetup::find_iso);
        let res = (|| -> Result<String, String> {
            if !ok {
                return Err("--confirm-erase <gleiche Nummer> fehlt".into());
            }
            let num: u32 = n
                .parse()
                .map_err(|_| "ungültige Datenträgernummer".to_string())?;
            let disk = bootsetup::disk_by_number(num)?;
            let iso = iso.ok_or("keine Boot-ISO gefunden (--iso)")?;
            let usb = std::env::current_exe().map_err(|e| e.to_string())?;
            bootsetup::setup(
                &bootsetup::SetupInput {
                    disk: num,
                    expect_serial: disk.serial_number.clone(),
                    expect_size: disk.size,
                    iso: &iso,
                    main_exe: &main_exe()?,
                    usb_exe: &usb,
                    gpu_exe: gpu_exe().as_deref(),
                    allow_virtual: std::env::args().any(|a| a == "--allow-virtual-disk"),
                },
                &|f, t| println!("{:>3.0} % {t}", f * 100.0),
            )
        })();
        match res {
            Ok(root) => println!("Fertig: {root}"),
            Err(e) => {
                eprintln!("Fehler: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if let Some(root) = arg_value("--install-portable") {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
        let root = if root.ends_with('\\') {
            root
        } else {
            format!("{root}\\")
        };
        match install_portable(&root) {
            Ok(m) => println!("{m}"),
            Err(e) => {
                eprintln!("Fehler: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    ramforge_update::selfreplace::cleanup_old();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("RAMForge-USB {}", env!("CARGO_PKG_VERSION")))
            .with_inner_size([860.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "RAMForge-USB",
        options,
        Box::new(|cc| {
            ramforge_update::ui::install_symbol_font(&cc.egui_ctx);
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            for theme in [egui::Theme::Dark, egui::Theme::Light] {
                cc.egui_ctx.style_mut_of(theme, |s| {
                    s.visuals.selection.bg_fill = ACCENT;
                    s.visuals.hyperlink_color = ACCENT;
                });
            }
            Ok(Box::new(UsbApp::new()))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Every file on the fake stick OUTSIDE the RAMForge folder: path -> bytes.
    fn outside_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(root).unwrap().to_path_buf();
                if rel
                    .components()
                    .next()
                    .map(|c| c.as_os_str() == STICK_DIR)
                    .unwrap_or(false)
                {
                    continue;
                }
                if p.is_dir() {
                    out.insert(rel.join("<dir>"), Vec::new());
                    walk(&p, root, out);
                } else {
                    out.insert(rel, std::fs::read(&p).unwrap());
                }
            }
        }
        let mut m = BTreeMap::new();
        walk(root, root, &mut m);
        m
    }

    /// A stick like the user's: emergency key folder, BIOS files, own reports.
    fn fake_stick(name: &str) -> (PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("ramforge-usb-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("stick");
        std::fs::create_dir_all(root.join("RAMForge-Keys")).unwrap();
        std::fs::write(
            root.join("RAMForge-Keys").join("emergency.pem"),
            b"-----BEGIN ENCRYPTED PRIVATE KEY-----\ntest\n",
        )
        .unwrap();
        std::fs::write(root.join("RAMForge-Keys").join("README.txt"), b"offline").unwrap();
        std::fs::write(root.join("E7B79AMS.AMA"), b"bios image").unwrap();
        std::fs::create_dir_all(root.join("Andere Daten").join("tief")).unwrap();
        std::fs::write(
            root.join("Andere Daten").join("tief").join("foto.jpg"),
            b"jpg",
        )
        .unwrap();
        std::fs::create_dir_all(root.join(STICK_DIR).join("RAMForge-data").join("reports"))
            .unwrap();
        std::fs::write(
            root.join(STICK_DIR)
                .join("RAMForge-data")
                .join("reports")
                .join("alt.html"),
            b"report",
        )
        .unwrap();
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("ramforge.exe"), b"main v1").unwrap();
        std::fs::write(src.join("RAMForge-USB.exe"), b"usb v1").unwrap();
        (root, src)
    }

    fn manifest(version: &str) -> Manifest {
        let a = |n: &str| ramforge_update::Asset {
            urls: vec![format!(
                "https://github.com/airichde/ramforge/releases/download/v{version}/{n}"
            )],
            sha256: "0".repeat(64),
            size: 0,
        };
        Manifest {
            schema: 1,
            version: version.into(),
            released: String::new(),
            channel: "stable".into(),
            notes_url: String::new(),
            min_supported: String::new(),
            assets: [
                ("windows_portable".to_string(), a("RAMForge.exe")),
                ("usb_tool".to_string(), a("RAMForge-USB.exe")),
            ]
            .into(),
        }
    }

    #[test]
    fn install_and_update_never_touch_other_folders() {
        let (root, src) = fake_stick("keep");
        let before = outside_snapshot(&root);
        assert!(before.keys().any(|p| p.starts_with("RAMForge-Keys")));

        // Install twice (second time replaces the EXEs).
        install_portable_from(
            &root,
            &src.join("ramforge.exe"),
            &src.join("RAMForge-USB.exe"),
        )
        .unwrap();
        install_portable_from(
            &root,
            &src.join("ramforge.exe"),
            &src.join("RAMForge-USB.exe"),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(root.join(STICK_DIR).join("RAMForge.exe")).unwrap(),
            b"main v1"
        );

        // In-place update with a fake downloader (the real one checks signature + SHA-256).
        let fake =
            |_a: &ramforge_update::Asset, dest: &Path, name: &str| -> Result<String, String> {
                assert!(
                    dest.starts_with(root.join(STICK_DIR)),
                    "Download außerhalb des RAMForge-Ordners: {}",
                    dest.display()
                );
                std::fs::write(dest, format!("{name} v2")).unwrap();
                Ok("GitHub".into())
            };
        let msg = update_portable_with(&root, &manifest("0.2.0"), &fake).unwrap();
        assert!(msg.contains("Quelle: GitHub"), "{msg}");
        assert_eq!(
            std::fs::read(root.join(STICK_DIR).join("RAMForge.exe")).unwrap(),
            b"RAMForge.exe v2"
        );
        let marker: StickInfo = serde_json::from_str(
            &std::fs::read_to_string(root.join(STICK_DIR).join("stick.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(marker.version, "0.2.0");

        // A failed download changes nothing either.
        let failing = |_a: &ramforge_update::Asset,
                       _d: &Path,
                       _n: &str|
         -> Result<String, String> { Err("offline".into()) };
        assert!(update_portable_with(&root, &manifest("0.3.0"), &failing).is_err());

        // Everything outside RAMForge\ is byte-identical, nothing added or removed,
        // and the user's own reports inside RAMForge\ survive.
        assert_eq!(outside_snapshot(&root), before);
        assert!(root
            .join(STICK_DIR)
            .join("RAMForge-data")
            .join("reports")
            .join("alt.html")
            .exists());
        assert!(!root.join(STICK_DIR).join("RAMForge.exe.new").exists());
        std::fs::remove_dir_all(root.parent().unwrap()).ok();
    }

    #[test]
    fn stick_paths_stay_inside_the_ramforge_folder() {
        let root = Path::new(r"I:\");
        assert_eq!(
            stick_path(root, "RAMForge.exe").unwrap(),
            PathBuf::from(r"I:\RAMForge\RAMForge.exe")
        );
        for bad in [
            "..",
            ".",
            "",
            r"..\RAMForge-Keys\x.pem",
            "../x",
            r"RAMForge-Keys\x",
            "C:x",
            "a/b",
        ] {
            assert!(
                stick_path(root, bad).is_err(),
                "{bad:?} muss abgelehnt werden"
            );
        }
    }
}
