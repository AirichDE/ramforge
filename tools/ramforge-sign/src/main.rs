//! ramforge-sign – offline signing tool for RAMForge releases.
//!
//!   ramforge-sign keygen --out <file.pem>     new key pair, passphrase-encrypted
//!   ramforge-sign pubkey <file.pem>           public key + fingerprint
//!   ramforge-sign sign   <file.pem> <file>    writes <file>.sig
//!   ramforge-sign verify <pubkey-b64|embedded> <file>   checks <file>.sig
//!   ramforge-sign hash   <file>               sha256 + size for the manifest
//!
//! Security rules:
//! * Private keys are stored only as ENCRYPTED PKCS#8 PEM (scrypt + AES-256-CBC).
//! * The passphrase is read from the terminal without echo. It is never taken
//!   from a command-line argument or an environment variable and never logged.
//! * The decrypted key exists only in memory (zeroized on drop), never on disk.
//! * `keygen` refuses to write inside a git working tree and never overwrites.

use base64::Engine;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use zeroize::Zeroizing;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const MIN_PASSPHRASE: usize = 12;

fn usage() -> ExitCode {
    eprintln!(
        "ramforge-sign {}\n\
         \n  keygen --out <datei.pem>              neues Schlüsselpaar, mit Passphrase verschlüsselt\
         \n  pubkey <datei.pem>                    öffentlichen Schlüssel und Fingerabdruck anzeigen\
         \n  sign   <datei.pem> <datei>            <datei>.sig erzeugen (z. B. latest.json oder eine EXE)\
         \n  verify <pubkey-base64|embedded> <datei>   <datei>.sig prüfen\
         \n  hash   <datei>                        SHA-256 und Größe für das Manifest\
         \n\nDie Passphrase wird nur interaktiv abgefragt.",
        env!("CARGO_PKG_VERSION")
    );
    ExitCode::from(2)
}

/// Returns the work tree root if `p` lies inside a git working tree.
fn git_work_tree(p: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(p).ok()?;
    abs.ancestors().skip(1).find(|d| d.join(".git").exists()).map(Path::to_path_buf)
}

fn fingerprint(pk: &[u8; 32]) -> String {
    ramforge_update::key::fingerprint_of(pk)
}

fn print_public(pk: &[u8; 32]) {
    println!("Öffentlicher Schlüssel (base64): {}", B64.encode(pk));
    println!("Fingerabdruck (SHA-256):         {}", fingerprint(pk));
    let bytes: Vec<String> = pk.iter().map(|b| format!("0x{b:02X}")).collect();
    println!("\nFür crates/update/src/key.rs (RELEASE_KEY oder EMERGENCY_KEY):\n");
    println!("[\n    {},\n    {},\n]", bytes[..16].join(", "), bytes[16..].join(", "));
}

// ------------------------------------------------------------------ core
// Everything below takes the passphrase as a parameter so it can be tested;
// only `main` reads it, and only from the terminal.

/// Creates a key pair and writes the private key, encrypted, to `out`.
fn keygen_to(out: &Path, passphrase: &str) -> Result<[u8; 32], String> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(format!("Passphrase zu kurz (mindestens {MIN_PASSPHRASE} Zeichen)"));
    }
    if let Some(repo) = git_work_tree(out) {
        return Err(format!(
            "Ziel liegt in einem Git-Arbeitsverzeichnis ({}). Bitte einen Ort außerhalb wählen, z. B. %USERPROFILE%\\.ramforge\\",
            repo.display()
        ));
    }
    if out.exists() {
        return Err(format!("{} existiert bereits – wird nicht überschrieben", out.display()));
    }
    // 32 bytes from the operating system's CSPRNG.
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut()).map_err(|e| format!("Zufallsquelle: {e}"))?;
    let sk = SigningKey::from_bytes(&seed);
    let pem = sk
        .to_pkcs8_encrypted_pem(passphrase.as_bytes(), pkcs8::LineEnding::LF)
        .map_err(|e| format!("Verschlüsseln: {e}"))?;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // create_new: fails instead of overwriting, even under a race.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    f.write_all(pem.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    Ok(sk.verifying_key().to_bytes())
}

/// Loads and decrypts a private key. Refuses unencrypted key files.
fn load_key(path: &Path, passphrase: &str) -> Result<SigningKey, String> {
    let pem = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?);
    if !pem.contains("BEGIN ENCRYPTED PRIVATE KEY") {
        return Err("Die Schlüsseldatei ist nicht verschlüsselt. Bitte mit `keygen` einen neuen, passwortgeschützten Schlüssel erzeugen.".into());
    }
    SigningKey::from_pkcs8_encrypted_pem(&pem, passphrase.as_bytes())
        .map_err(|_| "Entschlüsseln fehlgeschlagen: falsche Passphrase oder beschädigte Datei".to_string())
}

fn sig_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(".sig");
    PathBuf::from(s)
}

/// Signs `file`, writes `<file>.sig` (base64 + newline) and self-verifies it.
fn sign_file(sk: &SigningKey, file: &Path) -> Result<PathBuf, String> {
    let data = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if file.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
        // Refuse to sign a manifest the app would reject anyway.
        let m = ramforge_update::parse_unverified(&data).map_err(|e| format!("Manifest ungültig: {e}"))?;
        for (name, a) in &m.assets {
            ramforge_update::check_asset(a).map_err(|e| format!("Paket {name}: {e}"))?;
            if !a.urls[0].starts_with("https://github.com/") {
                eprintln!("Hinweis: Paket {name}: erste Quelle ist nicht GitHub Releases ({})", a.urls[0]);
            }
            if a.sha256.len() != 64 || !a.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("Paket {name}: sha256 fehlt oder ist ungültig"));
            }
        }
    }
    let sig = B64.encode(sk.sign(&data).to_bytes());
    let out = sig_path(file);
    std::fs::write(&out, format!("{sig}\n")).map_err(|e| format!("{}: {e}", out.display()))?;
    ramforge_update::verify_detached(&data, sig.as_bytes(), &[sk.verifying_key().to_bytes()])
        .map_err(|e| format!("Selbstprüfung fehlgeschlagen: {e}"))?;
    Ok(out)
}

/// Checks `<file>.sig` under the given keys; returns the index of the matching key.
fn verify_file(keys: &[[u8; 32]], file: &Path) -> Result<usize, String> {
    let data = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let sp = sig_path(file);
    let sig = std::fs::read(&sp).map_err(|e| format!("{}: {e}", sp.display()))?;
    ramforge_update::verify_detached(&data, &sig, keys).map_err(|e| e.to_string())
}

fn hash(path: &Path) -> Result<(), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        size += n as u64;
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    println!("{{\"sha256\": \"{hex}\", \"size\": {size}}}");
    Ok(())
}

// ------------------------------------------------------------- terminal

fn ask(prompt: &str) -> Result<Zeroizing<String>, String> {
    rpassword::prompt_password(prompt).map(Zeroizing::new).map_err(|e| format!("Passphrase lesen: {e}"))
}

fn cmd_keygen(out: &Path) -> Result<(), String> {
    // Check the target before asking, so nobody types a passphrase for nothing.
    if out.exists() {
        return Err(format!("{} existiert bereits – wird nicht überschrieben", out.display()));
    }
    if let Some(repo) = git_work_tree(out) {
        return Err(format!("Ziel liegt in einem Git-Arbeitsverzeichnis ({}). Bitte einen Ort außerhalb wählen.", repo.display()));
    }
    println!("Neue Passphrase (mindestens {MIN_PASSPHRASE} Zeichen, wird nicht angezeigt).");
    println!("Ohne Passphrase ist der Schlüssel unbrauchbar. Sie lässt sich nicht wiederherstellen.");
    let p1 = ask("Passphrase: ")?;
    if p1.chars().count() < MIN_PASSPHRASE {
        return Err(format!("Passphrase zu kurz (mindestens {MIN_PASSPHRASE} Zeichen)"));
    }
    let p2 = ask("Passphrase wiederholen: ")?;
    if *p1 != *p2 {
        return Err("Die Eingaben stimmen nicht überein".into());
    }
    let pk = keygen_to(out, &p1)?;
    println!("\nPrivater Schlüssel (verschlüsselt): {}", out.display());
    println!("Sichern: Datei UND Passphrase getrennt aufbewahren (z. B. Passwortmanager + zwei USB-Sticks offline).\n");
    print_public(&pk);
    Ok(())
}

fn cmd_pubkey(file: &Path) -> Result<(), String> {
    let p = ask("Passphrase: ")?;
    let sk = load_key(file, &p)?;
    print_public(&sk.verifying_key().to_bytes());
    Ok(())
}

fn cmd_sign(key: &Path, file: &Path) -> Result<(), String> {
    let p = ask("Passphrase: ")?;
    let sk = load_key(key, &p)?;
    drop(p);
    let pk = sk.verifying_key().to_bytes();
    let out = sign_file(&sk, file)?;
    println!("Signiert:  {}\nSignatur:  {}\nSchlüssel: {}", file.display(), out.display(), fingerprint(&pk));
    let trusted = ramforge_update::key::trusted_keys();
    if !trusted.contains(&pk) {
        println!("ACHTUNG: Dieser Schlüssel ist NICHT in dieser Programmversion hinterlegt. Die App würde die Signatur ablehnen.");
    }
    Ok(())
}

fn cmd_verify(key_arg: &str, file: &Path) -> Result<(), String> {
    let keys: Vec<[u8; 32]> = if key_arg == "embedded" {
        ramforge_update::key::trusted_keys()
    } else {
        let raw = B64.decode(key_arg.trim()).map_err(|e| format!("öffentlicher Schlüssel: {e}"))?;
        vec![raw.try_into().map_err(|_| "Schlüssel muss 32 Byte lang sein".to_string())?]
    };
    if keys.is_empty() {
        return Err("Kein Schlüssel hinterlegt".into());
    }
    let i = verify_file(&keys, file)?;
    println!("Signatur gültig. Schlüssel {}", fingerprint(&keys[i]));
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let res = match a.as_slice() {
        ["keygen", "--out", out] => cmd_keygen(Path::new(out)),
        ["pubkey", file] => cmd_pubkey(Path::new(file)),
        ["sign", key, file] => cmd_sign(Path::new(key), Path::new(file)),
        ["verify", key, file] => cmd_verify(key, Path::new(file)),
        ["hash", file] => hash(Path::new(file)),
        _ => return usage(),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory in the system temp folder, outside any repository.
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ramforge-sign-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert!(git_work_tree(&d.join("x")).is_none(), "Testordner liegt in einem Git-Repo: {}", d.display());
        d
    }

    const PASS: &str = "test-passphrase-123";

    #[test]
    fn keygen_sign_verify_round_trip() {
        let d = tmp("roundtrip");
        let key = d.join("release.pem");
        let pk = keygen_to(&key, PASS).unwrap();

        let pem = std::fs::read_to_string(&key).unwrap();
        assert!(pem.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----"), "nicht verschlüsselt");
        assert!(!pem.contains("BEGIN PRIVATE KEY"));

        // Wrong passphrase fails, right one restores the same key.
        assert!(load_key(&key, "wrong-passphrase-xx").is_err());
        let sk = load_key(&key, PASS).unwrap();
        assert_eq!(sk.verifying_key().to_bytes(), pk);

        let manifest = d.join("latest.json");
        std::fs::write(
            &manifest,
            r#"{"schema":1,"version":"0.2.0","assets":{"windows_portable":{"urls":["https://github.com/airichde/ramforge/releases/download/v0.2.0/RAMForge-0.2.0.exe","https://ramforge.org/dl/RAMForge-0.2.0.exe"],"sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","size":0}}}"#,
        )
        .unwrap();
        let sig = sign_file(&sk, &manifest).unwrap();
        assert!(sig.ends_with("latest.json.sig"));
        assert_eq!(verify_file(&[pk], &manifest), Ok(0));
        // The app's own check accepts it with this key as release or emergency key.
        let json = std::fs::read(&manifest).unwrap();
        let sigb = std::fs::read(&sig).unwrap();
        assert!(ramforge_update::verify_manifest(&json, &sigb, &[[0; 32], pk]).is_ok());

        // Tampering after signing is detected.
        std::fs::write(&manifest, std::fs::read_to_string(&manifest).unwrap().replace("0.2.0", "9.9.9")).unwrap();
        assert!(verify_file(&[pk], &manifest).is_err());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn keygen_refuses_short_passphrase_overwrite_and_git() {
        let d = tmp("refuse");
        assert!(keygen_to(&d.join("a.pem"), "short").is_err());
        let key = d.join("b.pem");
        keygen_to(&key, PASS).unwrap();
        let before = std::fs::read(&key).unwrap();
        assert!(keygen_to(&key, PASS).is_err(), "darf nicht überschreiben");
        assert_eq!(std::fs::read(&key).unwrap(), before);
        // Inside this repository (crate dir is in the ramforge git work tree).
        let in_repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("never-written.pem");
        if git_work_tree(&in_repo).is_some() {
            assert!(keygen_to(&in_repo, PASS).is_err());
            assert!(!in_repo.exists());
        }
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn unencrypted_key_files_are_refused() {
        let d = tmp("plain");
        let sk = SigningKey::from_bytes(&[5u8; 32]);
        let plain = sk.to_pkcs8_pem(pkcs8::LineEnding::LF).unwrap();
        let p = d.join("plain.pem");
        std::fs::write(&p, plain.as_bytes()).unwrap();
        assert!(load_key(&p, PASS).unwrap_err().contains("nicht verschlüsselt"));
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn manifest_with_http_asset_is_not_signed() {
        let d = tmp("http");
        let sk = SigningKey::from_bytes(&[6u8; 32]);
        let m = d.join("latest.json");
        std::fs::write(
            &m,
            r#"{"schema":1,"version":"0.2.0","assets":{"windows_portable":{"urls":["https://github.com/airichde/ramforge/releases/download/v0.2.0/x.exe","http://ramforge.org/x.exe"],"sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"}}}"#,
        )
        .unwrap();
        assert!(sign_file(&sk, &m).is_err());
        assert!(!sig_path(&m).exists());
        std::fs::remove_dir_all(d).ok();
    }
}
