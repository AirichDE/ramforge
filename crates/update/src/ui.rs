//! The update window shared by RAMForge.exe and RAMForge-USB.exe.

use crate::{selfreplace, AssetKind, CheckOutcome, Update, UpdateError};
use egui::{Align2, RichText, Vec2};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

enum State {
    Idle,
    Checking,
    Available(Box<Update>),
    Downloading { done: u64, total: u64 },
    Ready(Box<Update>, PathBuf),
    Failed(String),
}

struct Shared {
    state: State,
    open: bool,
    log: Vec<(bool, String)>,
}

fn apply(shared: &Mutex<Shared>, res: Result<CheckOutcome, UpdateError>, skipped: &str) {
    let mut s = shared.lock().unwrap();
    match res {
        Ok(CheckOutcome::Available(u)) if u.manifest.version == skipped && !u.urgent => {
            s.log.push((
                false,
                format!("Update {} verfügbar (übersprungen)", u.manifest.version),
            ));
            s.state = State::Idle;
        }
        Ok(CheckOutcome::Available(u)) => {
            s.log.push((
                false,
                format!("Update {} verfügbar, Signatur gültig", u.manifest.version),
            ));
            s.state = State::Available(u);
            s.open = true;
        }
        Ok(CheckOutcome::UpToDate { latest }) => {
            s.log.push((
                false,
                format!("Keine neuere Version (aktuell veröffentlicht: {latest})"),
            ));
            s.state = State::Idle;
        }
        Err(e @ UpdateError::BadSignature)
        | Err(e @ UpdateError::Insecure(_))
        | Err(e @ UpdateError::BadManifest(_)) => {
            s.log.push((true, e.to_string()));
            s.state = State::Failed(e.to_string());
            s.open = true;
        }
        Err(e) => {
            s.log.push((false, e.to_string()));
            s.state = State::Idle;
        }
    }
}

/// What the host application has to do after `show()`.
pub enum UiEvent {
    None,
    /// Remember this version as skipped in the settings.
    Skip(String),
    /// The new EXE was started: close this one now.
    Restart,
}

pub struct UpdateUi {
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    kind: AssetKind,
    current: String,
    product: String,
}

impl UpdateUi {
    pub fn new(product: &str, kind: AssetKind, current: &str) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                state: State::Idle,
                open: false,
                log: Vec::new(),
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            kind,
            current: current.to_string(),
            product: product.to_string(),
        }
    }

    /// Starts the check on a background thread. Offline or server errors are
    /// only logged and never block the program.
    pub fn check_in_background(&self, skipped: &str) {
        let shared = self.shared.clone();
        let (current, kind, skipped) = (self.current.clone(), self.kind, skipped.to_string());
        shared.lock().unwrap().state = State::Checking;
        std::thread::spawn(move || apply(&shared, crate::check(&current, kind), &skipped));
    }

    /// Offers an update from a manifest that was already fetched and verified
    /// with [`crate::fetch_verified_manifest`] (no second download).
    pub fn offer_from_manifest(&self, m: crate::Manifest, skipped: &str) {
        apply(
            &self.shared,
            crate::evaluate(m, &self.current, self.kind),
            skipped,
        );
    }

    pub fn take_log(&self) -> Vec<(bool, String)> {
        std::mem::take(&mut self.shared.lock().unwrap().log)
    }

    pub fn is_checking(&self) -> bool {
        matches!(self.shared.lock().unwrap().state, State::Checking)
    }

    fn start_download(&self, u: Box<Update>) {
        let shared = self.shared.clone();
        let cancel = self.cancel.clone();
        cancel.store(false, Ordering::Relaxed);
        let total = u.asset.size;
        shared.lock().unwrap().state = State::Downloading { done: 0, total };
        std::thread::spawn(move || {
            let res = selfreplace::staging_path().and_then(|dest| {
                let sh = shared.clone();
                crate::download(
                    &u.asset,
                    &dest,
                    &move |done, total| {
                        sh.lock().unwrap().state = State::Downloading { done, total }
                    },
                    &cancel,
                )
                .map(|got| (dest, got))
            });
            let mut s = shared.lock().unwrap();
            match res {
                Ok((dest, got)) => {
                    s.log.push((
                        false,
                        format!(
                            "Update {} geladen (Quelle: {}), SHA-256 geprüft",
                            u.manifest.version, got.source
                        ),
                    ));
                    s.state = State::Ready(u, dest);
                }
                Err(e) => {
                    s.log.push((true, e.to_string()));
                    s.state = State::Failed(e.to_string());
                }
            }
        });
    }

    /// Draws the window if it is open. `busy` = a RAM test or write operation runs.
    pub fn show(&mut self, ctx: &egui::Context, busy: bool) -> UiEvent {
        let mut event = UiEvent::None;
        let mut action: Option<Box<dyn FnOnce(&mut Self) -> UiEvent>> = None;
        {
            let mut s = self.shared.lock().unwrap();
            if !s.open {
                return UiEvent::None;
            }
            let mut close = false;
            let title = match &s.state {
                State::Failed(_) => "Update",
                _ => "Update verfügbar",
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_min_width(420.0);
                    match &s.state {
                        State::Available(u) => {
                            let m = &u.manifest;
                            ui.label(RichText::new(format!("{} {} ist verfügbar", self.product, m.version)).size(17.0).strong());
                            ui.label(format!("Installiert: {} · veröffentlicht: {}", self.current, m.released));
                            if u.urgent {
                                ui.label(RichText::new("Diese Version wird nicht mehr unterstützt. Ein Update wird dringend empfohlen.").color(egui::Color32::from_rgb(0xE8, 0x9A, 0x2C)));
                            }
                            if !m.notes_url.is_empty() {
                                ui.hyperlink_to("Was ist neu?", &m.notes_url);
                            }
                            ui.label(RichText::new("Signatur geprüft. Der Download wird vor dem Einsetzen per SHA-256 kontrolliert.").small().weak());
                            if busy {
                                ui.label(RichText::new("Während ein Test läuft, wird nicht aktualisiert.").small());
                            }
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!busy, egui::Button::new("Jetzt aktualisieren")).clicked() {
                                    let u = u.clone();
                                    action = Some(Box::new(move |me: &mut Self| {
                                        if !selfreplace::can_self_update() {
                                            me.fail("Der Ordner der EXE ist schreibgeschützt. Bitte die neue Version manuell von ramforge.org laden.");
                                        } else {
                                            me.start_download(u);
                                        }
                                        UiEvent::None
                                    }));
                                }
                                if ui.button("Später").clicked() {
                                    close = true;
                                }
                                if ui.button("Diese Version überspringen").clicked() {
                                    event = UiEvent::Skip(m.version.clone());
                                    close = true;
                                }
                            });
                        }
                        State::Downloading { done, total } => {
                            ui.label("Update wird geladen…");
                            let f = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                            ui.add(egui::ProgressBar::new(f).text(format!("{:.1} MB", *done as f64 / 1e6)));
                            if ui.button("Abbrechen").clicked() {
                                self.cancel.store(true, Ordering::Relaxed);
                            }
                        }
                        State::Ready(u, path) => {
                            ui.label(format!("{} {} ist geladen und geprüft.", self.product, u.manifest.version));
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!busy, egui::Button::new("Neu starten und installieren")).clicked() {
                                    let path = path.clone();
                                    action = Some(Box::new(move |me: &mut Self| match selfreplace::install_and_restart(&path) {
                                        Ok(()) => UiEvent::Restart,
                                        Err(e) => {
                                            me.fail(&e.to_string());
                                            UiEvent::None
                                        }
                                    }));
                                }
                                if ui.button("Später").clicked() {
                                    close = true;
                                }
                            });
                        }
                        State::Failed(msg) => {
                            ui.label(RichText::new(msg).color(egui::Color32::from_rgb(0xE5, 0x48, 0x4D)));
                            if msg.starts_with("Download fehlgeschlagen") {
                                ui.hyperlink_to("Neue Version manuell von der Website laden", crate::DOWNLOAD_PAGE);
                            }
                            if ui.button("Schließen").clicked() {
                                close = true;
                            }
                        }
                        State::Idle | State::Checking => {
                            close = true;
                        }
                    }
                });
            if close {
                s.open = false;
            }
        }
        if let Some(a) = action {
            event = a(self);
        }
        event
    }

    fn fail(&self, msg: &str) {
        let mut s = self.shared.lock().unwrap();
        s.log.push((true, msg.to_string()));
        s.state = State::Failed(msg.to_string());
    }
}

/// Adds Windows' "Segoe UI Symbol" as a fallback font so symbols like ▶ ⏹ 📷 💾
/// render instead of empty boxes. Does nothing if the font is not there
/// (the boot edition bundles its own symbol font).
pub fn install_symbol_font(ctx: &egui::Context) {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let candidates = [
        std::path::Path::new(&windir)
            .join("Fonts")
            .join("seguisym.ttf"),
        // Boot edition (Debian live): fonts-symbola, then DejaVu as a fallback.
        "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf".into(),
        "/usr/share/fonts/truetype/symbola/Symbola.ttf".into(),
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(),
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut added = false;
    for (i, path) in candidates.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("symbols{i}");
            fonts
                .font_data
                .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push(name.clone());
            }
            added = true;
        }
    }
    if added {
        ctx.set_fonts(fonts);
    }
}
