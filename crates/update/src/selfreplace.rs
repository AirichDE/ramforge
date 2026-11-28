//! Replacing the running EXE with a verified download.
//!
//! Windows lets you rename a running EXE but not overwrite it, so:
//! `RAMForge.exe` → `RAMForge.exe.old`, `RAMForge.exe.new` → `RAMForge.exe`,
//! start the new EXE, exit. The next start deletes `RAMForge.exe.old`.

use crate::UpdateError;
#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use std::path::{Path, PathBuf};

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

pub fn current_exe() -> Result<PathBuf, UpdateError> {
    std::env::current_exe().map_err(|e| UpdateError::Io(e.to_string()))
}

/// Where the download goes: next to the EXE, so the final rename stays on one volume.
pub fn staging_path() -> Result<PathBuf, UpdateError> {
    Ok(sibling(&current_exe()?, ".new"))
}

/// Call once at startup: removes the previous version left by an update.
/// Retries for a few seconds in the background, because the old process may
/// still be shutting down.
pub fn cleanup_old() {
    let Ok(exe) = current_exe() else { return };
    let old = sibling(&exe, ".old");
    let stale_new = sibling(&exe, ".new");
    let _ = std::fs::remove_file(&stale_new);
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..20 {
            if std::fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}

/// True if the folder of the EXE is writable (a precondition for self-update).
pub fn can_self_update() -> bool {
    let Ok(exe) = current_exe() else { return false };
    let probe = sibling(&exe, ".write-test");
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

/// Swaps in the verified file at `new_file` and starts it.
/// The caller must exit right after this returns Ok.
pub fn install_and_restart(new_file: &Path) -> Result<(), UpdateError> {
    let exe = current_exe()?;
    let old = sibling(&exe, ".old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| {
        UpdateError::Io(tf!(
            "alte Version umbenennen: {e}",
            "renaming old version: {e}"
        ))
    })?;
    if let Err(e) = std::fs::rename(new_file, &exe) {
        // Roll back, so the user keeps a working EXE.
        let _ = std::fs::rename(&old, &exe);
        return Err(UpdateError::Io(tf!(
            "neue Version einsetzen: {e}",
            "installing new version: {e}"
        )));
    }
    std::process::Command::new(&exe)
        .arg("--updated")
        .spawn()
        .map_err(|e| {
            UpdateError::Io(tf!(
                "neue Version starten: {e}",
                "starting new version: {e}"
            ))
        })?;
    Ok(())
}
