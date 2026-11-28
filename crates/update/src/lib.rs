//! Update check for RAMForge.exe, RAMForge-USB.exe and the boot ISO.
//!
//! Trust chain, in this order:
//! 1. Download `latest.json` and `latest.json.sig` over HTTPS only.
//! 2. Verify the detached Ed25519 signature over the exact bytes of
//!    `latest.json` with the public key compiled into the binary ([`key`]).
//!    The server alone is never trusted: an attacker controlling it could swap
//!    a file and its hash together, but cannot forge the signature.
//! 3. Reject anything that is not newer than the running version (no downgrades).
//! 4. Download the asset from its sources in order (GitHub Releases first,
//!    ramforge.org mirror second; HTTPS only, also for redirects) and compare
//!    its SHA-256 and size with the signed manifest before anything is
//!    replaced. A failing source is discarded and the next one tried.
//!
//! `latest.json` and its signature live only on ramforge.org, never on GitHub.

pub mod fetch;
pub mod key;
pub mod online;
pub mod selfreplace;
#[cfg(feature = "ui")]
pub mod ui;

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub use fetch::Downloaded;

/// Change here if the update location moves.
pub const MANIFEST_URL: &str = "https://ramforge.org/update/latest.json";
pub const SIGNATURE_URL: &str = "https://ramforge.org/update/latest.json.sig";
/// Second source for the signed manifest: the newest GitHub release carries
/// the same latest.json + .sig. The signature is checked either way, so the
/// source does not need to be trusted.
pub const MANIFEST_URL_GITHUB: &str =
    "https://github.com/airichde/ramforge/releases/latest/download/latest.json";
pub const SIGNATURE_URL_GITHUB: &str =
    "https://github.com/airichde/ramforge/releases/latest/download/latest.json.sig";
/// Shown when every download source failed.
pub const DOWNLOAD_PAGE: &str = "https://ramforge.org/download";

const MAX_MANIFEST: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    /// Sources in order of preference: GitHub Releases first, then the mirror.
    pub urls: Vec<String>,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    #[serde(default)]
    pub released: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub notes_url: String,
    #[serde(default)]
    pub min_supported: String,
    pub assets: HashMap<String, Asset>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    WindowsPortable,
    UsbTool,
    BootIso,
    /// Smaller boot image without GPU firmware/Vulkan/GPU module.
    BootIsoCompact,
    /// Smallest boot image: RAM test only, no benchmarks, no network.
    BootIsoRam,
    /// Optional RAMForge-GPU.exe (GPU benchmark, ~6 MB).
    GpuModule,
}

impl AssetKind {
    pub fn key(self) -> &'static str {
        match self {
            AssetKind::WindowsPortable => "windows_portable",
            AssetKind::UsbTool => "usb_tool",
            AssetKind::BootIso => "boot_iso",
            AssetKind::BootIsoCompact => "boot_iso_compact",
            AssetKind::BootIsoRam => "boot_iso_ram",
            AssetKind::GpuModule => "gpu_module",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Update {
    pub manifest: Manifest,
    pub asset: Asset,
    pub kind: AssetKind,
    /// The running version is older than `min_supported`: update strongly advised.
    pub urgent: bool,
}

#[derive(Clone, Debug)]
pub enum CheckOutcome {
    UpToDate { latest: String },
    Available(Box<Update>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateError {
    /// No public key compiled in (development build). Nothing was contacted.
    NoKey,
    Network(String),
    /// The server answered, but has no update information yet (HTTP 404).
    NotPublished,
    BadSignature,
    BadManifest(String),
    Insecure(String),
    HashMismatch {
        expected: String,
        actual: String,
    },
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    Io(String),
    Cancelled,
    /// Every source of an asset failed; one reason per source.
    AllSourcesFailed(Vec<String>),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            UpdateError::NoKey => t!(
                "Update-Prüfung deaktiviert: kein Update-Schlüssel hinterlegt (Entwicklungsversion)",
                "Update check disabled: no update key built in (development build)"
            )
            .to_string(),
            UpdateError::Network(e) => tf!(
                "Keine Verbindung zum Update-Server: {e}",
                "No connection to the update server: {e}"
            ),
            UpdateError::NotPublished => {
                t!("Noch keine Version veröffentlicht", "No version published yet").to_string()
            }
            UpdateError::BadSignature => t!(
                "Update-Signatur ungültig – es wurde nichts installiert",
                "Invalid update signature – nothing was installed"
            )
            .to_string(),
            UpdateError::BadManifest(e) => tf!(
                "Update-Information fehlerhaft: {e}",
                "Faulty update information: {e}"
            ),
            UpdateError::Insecure(u) => tf!(
                "Unsichere Update-Adresse abgelehnt (nur HTTPS): {u}",
                "Insecure update address refused (HTTPS only): {u}"
            ),
            UpdateError::HashMismatch { .. } => t!(
                "Prüfsumme des Downloads stimmt nicht – es wurde nichts installiert",
                "Download checksum does not match – nothing was installed"
            )
            .to_string(),
            UpdateError::SizeMismatch { expected, actual } => tf!(
                "Download hat die falsche Größe ({actual} statt {expected} Byte) – es wurde nichts installiert",
                "Download has the wrong size ({actual} instead of {expected} bytes) – nothing was installed"
            ),
            UpdateError::Io(e) => tf!(
                "Update konnte nicht geschrieben werden: {e}",
                "Update could not be written: {e}"
            ),
            UpdateError::Cancelled => t!("Update abgebrochen", "Update cancelled").to_string(),
            UpdateError::AllSourcesFailed(reasons) => tf!(
                "Download fehlgeschlagen – bitte später erneut versuchen. Es wurde nichts geändert. ({})",
                "Download failed – please try again later. Nothing was changed. ({})",
                reasons.join("; ")
            ),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for UpdateError {}

/// Semantic version "x.y.z" (a leading "v" and a "-suffix" are ignored).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('v');
        let core = s.split(['-', '+']).next()?;
        let mut it = core.split('.').map(|p| p.parse::<u64>());
        let v = Version(
            it.next()?.ok()?,
            it.next().unwrap_or(Ok(0)).ok()?,
            it.next().unwrap_or(Ok(0)).ok()?,
        );
        it.next().is_none().then_some(v)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

fn require_https(url: &str) -> Result<(), UpdateError> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err(UpdateError::Insecure(url.to_string()))
    }
}

/// Verifies a detached base64 Ed25519 signature (surrounding whitespace is
/// ignored). Valid if it verifies under ANY of `keys`, and only then.
/// Returns the index of the key that matched.
pub fn verify_detached(
    data: &[u8],
    sig_b64: &[u8],
    keys: &[[u8; 32]],
) -> Result<usize, UpdateError> {
    let sig_text = std::str::from_utf8(sig_b64).map_err(|_| UpdateError::BadSignature)?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(sig_text.trim())
        .map_err(|_| UpdateError::BadSignature)?;
    let sig = Signature::from_slice(&raw).map_err(|_| UpdateError::BadSignature)?;
    keys.iter()
        .position(|k| {
            *k != [0; 32]
                && VerifyingKey::from_bytes(k).is_ok_and(|vk| vk.verify_strict(data, &sig).is_ok())
        })
        .ok_or(UpdateError::BadSignature)
}

/// Step 2: verify the signature against the trusted keys, then parse.
/// Nothing is parsed before the signature is valid.
pub fn verify_manifest(
    json: &[u8],
    sig_b64: &[u8],
    keys: &[[u8; 32]],
) -> Result<Manifest, UpdateError> {
    verify_detached(json, sig_b64, keys)?;
    let m: Manifest =
        serde_json::from_slice(json).map_err(|e| UpdateError::BadManifest(e.to_string()))?;
    if m.schema != 1 {
        return Err(UpdateError::BadManifest(tf!(
            "unbekanntes Schema {}",
            "unknown schema {}",
            m.schema
        )));
    }
    Ok(m)
}

/// Parses a manifest WITHOUT checking a signature. Only for the signing tool,
/// which checks a manifest before it signs it. Clients use [`verify_manifest`].
pub fn parse_unverified(json: &[u8]) -> Result<Manifest, String> {
    let m: Manifest = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    if m.schema != 1 {
        return Err(tf!("unbekanntes Schema {}", "unknown schema {}", m.schema));
    }
    Version::parse(&m.version)
        .ok_or_else(|| tf!("Version {} ungültig", "version {} invalid", m.version))?;
    Ok(m)
}

/// Step 3: decide whether the signed manifest offers a real upgrade.
pub fn evaluate(m: Manifest, current: &str, kind: AssetKind) -> Result<CheckOutcome, UpdateError> {
    let cur = Version::parse(current).ok_or_else(|| {
        UpdateError::BadManifest(tf!(
            "eigene Version {current} ungültig",
            "own version {current} invalid"
        ))
    })?;
    let latest = Version::parse(&m.version).ok_or_else(|| {
        UpdateError::BadManifest(tf!("Version {} ungültig", "version {} invalid", m.version))
    })?;
    if latest <= cur {
        return Ok(CheckOutcome::UpToDate {
            latest: m.version.clone(),
        });
    }
    let asset = m.assets.get(kind.key()).cloned().ok_or_else(|| {
        UpdateError::BadManifest(tf!("kein Paket „{}“", "no package “{}”", kind.key()))
    })?;
    check_asset(&asset)?;
    if asset.sha256.len() != 64 || !asset.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(UpdateError::BadManifest(
            t!(
                "SHA-256 fehlt oder ist ungültig",
                "SHA-256 missing or invalid"
            )
            .into(),
        ));
    }
    let urgent = Version::parse(&m.min_supported).is_some_and(|min| cur < min);
    Ok(CheckOutcome::Available(Box::new(Update {
        manifest: m,
        asset,
        kind,
        urgent,
    })))
}

/// Every asset needs at least one source, and every source must be HTTPS.
pub fn check_asset(asset: &Asset) -> Result<(), UpdateError> {
    if asset.urls.is_empty() {
        return Err(UpdateError::BadManifest(
            t!(
                "Paket ohne Download-Adresse",
                "package without download address"
            )
            .into(),
        ));
    }
    for u in &asset.urls {
        require_https(u)?;
    }
    Ok(())
}

pub(crate) fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(format!("RAMForge/{}", env!("CARGO_PKG_VERSION")))
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

pub(crate) fn fetch_small(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, UpdateError> {
    require_https(url)?;
    let mut resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(404) => UpdateError::NotPublished,
        e => UpdateError::Network(e.to_string()),
    })?;
    resp.body_mut()
        .with_config()
        .limit(MAX_MANIFEST)
        .read_to_vec()
        .map_err(|e| UpdateError::Network(e.to_string()))
}

/// Steps 1–3. Contacts the server only if a public key is compiled in.
pub fn check(current: &str, kind: AssetKind) -> Result<CheckOutcome, UpdateError> {
    evaluate(fetch_verified_manifest()?, current, kind)
}

pub fn check_from(
    current: &str,
    kind: AssetKind,
    manifest_url: &str,
    sig_url: &str,
) -> Result<CheckOutcome, UpdateError> {
    evaluate(
        fetch_verified_manifest_from(manifest_url, sig_url)?,
        current,
        kind,
    )
}

/// Steps 1–2 only: the manifest, after its signature was verified.
/// Used where one manifest answers several questions (RAMForge-USB checks
/// itself and the stick with the same download).
/// The signed manifest from ramforge.org, or from the latest GitHub release
/// if ramforge.org is unreachable or has none. A bad signature is never
/// retried elsewhere.
pub fn fetch_verified_manifest() -> Result<Manifest, UpdateError> {
    let primary = match fetch_verified_manifest_from(MANIFEST_URL, SIGNATURE_URL) {
        Ok(m) => return Ok(m),
        Err(e @ (UpdateError::NoKey | UpdateError::BadSignature | UpdateError::BadManifest(_))) => {
            return Err(e)
        }
        Err(e) => e,
    };
    match fetch_verified_manifest_from(MANIFEST_URL_GITHUB, SIGNATURE_URL_GITHUB) {
        Ok(m) => Ok(m),
        Err(UpdateError::BadSignature) => Err(UpdateError::BadSignature),
        // Report what ramforge.org said: that is the primary source.
        Err(_) => Err(primary),
    }
}

pub fn fetch_verified_manifest_from(
    manifest_url: &str,
    sig_url: &str,
) -> Result<Manifest, UpdateError> {
    if !key::is_configured() {
        return Err(UpdateError::NoKey);
    }
    let agent = agent();
    let json = fetch_small(&agent, manifest_url)?;
    let sig = fetch_small(&agent, sig_url)?;
    verify_manifest(&json, &sig, &key::trusted_keys())
}

/// State of a RAMForge boot stick compared with the signed manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StickState {
    UpToDate {
        installed: String,
    },
    Outdated {
        installed: String,
        latest: String,
        urgent: bool,
    },
    /// The version on the stick can not be read or compared.
    Unknown {
        installed: String,
    },
}

pub fn stick_state(m: &Manifest, installed: &str) -> StickState {
    let (Some(cur), Some(latest)) = (Version::parse(installed), Version::parse(&m.version)) else {
        return StickState::Unknown {
            installed: installed.to_string(),
        };
    };
    if cur >= latest {
        StickState::UpToDate {
            installed: installed.to_string(),
        }
    } else {
        let urgent = Version::parse(&m.min_supported).is_some_and(|min| cur < min);
        StickState::Outdated {
            installed: installed.to_string(),
            latest: m.version.clone(),
            urgent,
        }
    }
}

/// Checks the hash and size of data that was already written.
pub fn verify_digest(
    expected_hex: &str,
    expected_size: u64,
    actual: &[u8],
    actual_size: u64,
) -> Result<(), UpdateError> {
    if expected_size > 0 && expected_size != actual_size {
        return Err(UpdateError::SizeMismatch {
            expected: expected_size,
            actual: actual_size,
        });
    }
    let hex: String = actual.iter().map(|b| format!("{b:02x}")).collect();
    if !hex.eq_ignore_ascii_case(expected_hex) {
        return Err(UpdateError::HashMismatch {
            expected: expected_hex.to_lowercase(),
            actual: hex,
        });
    }
    Ok(())
}

/// Step 4: downloads the asset to `dest`, trying its sources in order
/// (see [`fetch`]). On success `dest` holds exactly the signed bytes and the
/// result names the source used; on failure `dest` does not exist.
pub fn download(
    asset: &Asset,
    dest: &Path,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> Result<Downloaded, UpdateError> {
    check_asset(asset)?;
    fetch::download_asset(&fetch::HttpsFetcher::new(), asset, dest, progress, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    /// Test keys from fixed seeds: release, emergency, unknown attacker.
    fn sk(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }
    fn pk(seed: u8) -> [u8; 32] {
        sk(seed).verifying_key().to_bytes()
    }
    const REL: u8 = 7;
    const EMG: u8 = 8;
    const EVIL: u8 = 9;
    fn trusted() -> [[u8; 32]; 2] {
        [pk(REL), pk(EMG)]
    }
    fn sign_with(seed: u8, data: &str) -> Vec<u8> {
        let sig = sk(seed).sign(data.as_bytes());
        // Trailing newline like the server writes it: must be tolerated.
        format!(
            "{}\n",
            base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
        )
        .into_bytes()
    }

    const JSON: &str = r#"{"schema":1,"version":"0.2.0","released":"2026-12-01","channel":"stable",
        "notes_url":"https://ramforge.org/changelog#0.2.0","min_supported":"0.1.5",
        "assets":{"windows_portable":{"urls":["https://github.com/airichde/ramforge/releases/download/v0.2.0/RAMForge-0.2.0.exe","https://ramforge.org/dl/RAMForge-0.2.0.exe"],
        "sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","size":0}}}"#;

    #[test]
    fn release_key_signature_is_accepted() {
        let m = verify_manifest(JSON.as_bytes(), &sign_with(REL, JSON), &trusted()).unwrap();
        assert_eq!(
            verify_detached(JSON.as_bytes(), &sign_with(REL, JSON), &trusted()),
            Ok(0)
        );
        match evaluate(m, "0.1.0", AssetKind::WindowsPortable).unwrap() {
            CheckOutcome::Available(u) => {
                assert_eq!(u.manifest.version, "0.2.0");
                assert!(u.urgent);
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn emergency_key_signature_is_accepted() {
        assert!(verify_manifest(JSON.as_bytes(), &sign_with(EMG, JSON), &trusted()).is_ok());
        assert_eq!(
            verify_detached(JSON.as_bytes(), &sign_with(EMG, JSON), &trusted()),
            Ok(1)
        );
    }

    #[test]
    fn unknown_key_is_rejected() {
        let err = verify_manifest(JSON.as_bytes(), &sign_with(EVIL, JSON), &trusted()).unwrap_err();
        assert_eq!(err, UpdateError::BadSignature);
        // An all-zero (unset) key never matches anything.
        assert!(verify_manifest(JSON.as_bytes(), &sign_with(REL, JSON), &[[0; 32]]).is_err());
        assert!(verify_manifest(JSON.as_bytes(), &sign_with(REL, JSON), &[]).is_err());
    }

    #[test]
    fn tampered_manifest_is_rejected() {
        let sig = sign_with(REL, JSON);
        let evil = JSON.replace("RAMForge-0.2.0.exe", "evil.exe");
        assert_eq!(
            verify_manifest(evil.as_bytes(), &sig, &trusted()).unwrap_err(),
            UpdateError::BadSignature
        );
        let one_byte = JSON.replacen("0.2.0", "0.2.1", 1);
        assert_eq!(
            verify_manifest(one_byte.as_bytes(), &sig, &trusted()).unwrap_err(),
            UpdateError::BadSignature
        );
        assert_eq!(
            verify_manifest(JSON.as_bytes(), b"not base64!", &trusted()).unwrap_err(),
            UpdateError::BadSignature
        );
    }

    #[test]
    fn downgrade_is_rejected_and_https_only() {
        let m = verify_manifest(JSON.as_bytes(), &sign_with(REL, JSON), &trusted()).unwrap();
        // Same or newer running version: nothing is offered, never an older file.
        assert!(matches!(
            evaluate(m.clone(), "0.2.0", AssetKind::WindowsPortable).unwrap(),
            CheckOutcome::UpToDate { .. }
        ));
        assert!(matches!(
            evaluate(m.clone(), "0.3.1", AssetKind::WindowsPortable).unwrap(),
            CheckOutcome::UpToDate { .. }
        ));
        assert!(evaluate(m.clone(), "0.1.0", AssetKind::UsbTool).is_err());
        // One insecure source in the list is enough to reject the asset.
        let http = JSON.replace("https://ramforge.org/dl", "http://ramforge.org/dl");
        let m = verify_manifest(http.as_bytes(), &sign_with(REL, &http), &trusted()).unwrap();
        assert!(matches!(
            evaluate(m, "0.1.0", AssetKind::WindowsPortable),
            Err(UpdateError::Insecure(_))
        ));
    }

    #[test]
    fn asset_needs_sources() {
        let empty = JSON.replace(
            r#""urls":["https://github.com/airichde/ramforge/releases/download/v0.2.0/RAMForge-0.2.0.exe","https://ramforge.org/dl/RAMForge-0.2.0.exe"]"#,
            r#""urls":[]"#,
        );
        assert_ne!(empty, JSON);
        let m = verify_manifest(empty.as_bytes(), &sign_with(REL, &empty), &trusted()).unwrap();
        assert!(matches!(
            evaluate(m, "0.1.0", AssetKind::WindowsPortable),
            Err(UpdateError::BadManifest(_))
        ));
        let m = verify_manifest(JSON.as_bytes(), &sign_with(REL, JSON), &trusted()).unwrap();
        assert_eq!(m.assets["windows_portable"].urls.len(), 2);
        assert!(m.assets["windows_portable"].urls[0].starts_with("https://github.com/"));
    }

    #[test]
    fn stick_states() {
        let m = verify_manifest(JSON.as_bytes(), &sign_with(REL, JSON), &trusted()).unwrap();
        assert_eq!(
            stick_state(&m, "0.2.0"),
            StickState::UpToDate {
                installed: "0.2.0".into()
            }
        );
        assert_eq!(
            stick_state(&m, "0.1.0"),
            StickState::Outdated {
                installed: "0.1.0".into(),
                latest: "0.2.0".into(),
                urgent: true
            }
        );
        assert!(matches!(
            stick_state(&m, "0.1.9"),
            StickState::Outdated { urgent: false, .. }
        ));
        assert!(matches!(
            stick_state(&m, "kaputt"),
            StickState::Unknown { .. }
        ));
    }

    #[test]
    fn versions() {
        assert_eq!(Version::parse("v1.2.3-beta"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("0.10"), Some(Version(0, 10, 0)));
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("abc"), None);
    }

    #[test]
    fn digest_check() {
        let empty = Sha256::digest(b"");
        let ok = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_digest(ok, 0, &empty, 0).is_ok());
        assert!(verify_digest(&ok.to_uppercase(), 0, &empty, 0).is_ok());
        assert!(matches!(
            verify_digest(ok, 5, &empty, 0),
            Err(UpdateError::SizeMismatch { .. })
        ));
        let bad = Sha256::digest(b"x");
        assert!(matches!(
            verify_digest(ok, 0, &bad, 1),
            Err(UpdateError::HashMismatch { .. })
        ));
    }

    #[test]
    fn fingerprint_format() {
        // SHA-256 of the raw key, uppercase hex, 16 groups of 4, single spaces.
        // Must match exactly what https://ramforge.org publishes.
        assert_eq!(
            key::release_fingerprint(),
            "79D1 AF24 BEAD 4207 50B6 D310 02F0 B850 677A D831 D426 A841 FBFD CF06 1868 87E1"
        );
        assert_eq!(
            key::emergency_fingerprint(),
            "8568 B6C3 10CA 2122 F398 9AB6 E441 E77E B6AE 3BB3 7ED3 780F 021A 6308 E82E 288A"
        );
        assert_eq!(
            key::to_b64(&key::RELEASE_KEY),
            "6X+e5Aj8uH8YW4i0GwYOw9E4DIAjyBiGww28XZBSloo="
        );
        assert_eq!(
            key::to_b64(&key::EMERGENCY_KEY),
            "GMEBp0o15JJ/V1U1nNXytPCvvzcab+jGg78xJlTHV6w="
        );
        assert_eq!(key::fingerprint_of(&[0; 32]).split(' ').count(), 16);
        if !key::is_configured() {
            assert_eq!(
                check("0.1.0", AssetKind::WindowsPortable).unwrap_err(),
                UpdateError::NoKey
            );
        }
    }

    /// Release gate: `cargo test -p ramforge-update -- --ignored` must pass
    /// before a public release. Fails while test/placeholder keys are compiled in.
    #[test]
    #[ignore]
    fn release_keys_are_real() {
        assert!(
            !key::is_placeholder(),
            "RELEASE_KEY/EMERGENCY_KEY sind noch Platzhalter (crates/update/src/key.rs)"
        );
        assert!(key::RELEASE_KEY != [0; 32] && key::EMERGENCY_KEY != [0; 32]);
        assert_ne!(key::RELEASE_KEY, key::EMERGENCY_KEY);
        // Exactly these two keys are trusted; the old website test key is gone.
        let old_test_key: [u8; 32] = [
            0xC8, 0xA3, 0xB4, 0xF3, 0x86, 0x61, 0x54, 0xF5, 0x86, 0x77, 0x61, 0x28, 0x9D, 0x7F,
            0xC5, 0x3E, 0xE0, 0xB8, 0x12, 0xCC, 0x17, 0xC0, 0xAA, 0x67, 0x65, 0xCF, 0xC5, 0x53,
            0xC0, 0x3C, 0xA7, 0xD6,
        ];
        assert_eq!(
            key::trusted_keys(),
            vec![key::RELEASE_KEY, key::EMERGENCY_KEY]
        );
        assert!(!key::trusted_keys().contains(&old_test_key));
    }
}
