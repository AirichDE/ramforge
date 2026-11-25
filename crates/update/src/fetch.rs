//! Downloading release files from an ordered list of sources
//! (GitHub Releases first, the ramforge.org mirror second).
//!
//! Rules:
//! * Every URL and every redirect target must be `https://`. Redirects are
//!   followed by hand (at most [`MAX_REDIRECTS`]), so a downgrade to http is
//!   impossible. No host allowlist: integrity comes from the signed manifest
//!   and the SHA-256 check, not from where the bytes come from.
//! * A source that fails (network error, timeout, HTTP error, more bytes than
//!   the manifest size, wrong SHA-256) is discarded, its partial file deleted,
//!   and the next source is tried.
//! * Nothing is installed unless one source delivered exactly the signed bytes.

use crate::{verify_digest, Asset, UpdateError};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const MAX_REDIRECTS: usize = 5;
const MAX_ASSET: u64 = 4 << 30;

/// One HTTP step, without following redirects.
pub enum Hop {
    Redirect(String),
    Body {
        reader: Box<dyn Read + Send>,
        len: Option<u64>,
    },
    Status(u16),
}

/// Network access behind a trait, so the fallback logic is testable offline.
pub trait Fetcher {
    fn fetch(&self, url: &str) -> Result<Hop, String>;
}

/// The real fetcher: HTTPS only, Windows certificate store, no automatic redirects.
pub struct HttpsFetcher {
    agent: ureq::Agent,
}

impl HttpsFetcher {
    pub fn new() -> Self {
        use ureq::tls::{RootCerts, TlsConfig};
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_body(Some(Duration::from_secs(60)))
            .user_agent(format!("RAMForge/{}", env!("CARGO_PKG_VERSION")))
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        Self { agent }
    }
}

impl Default for HttpsFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher for HttpsFetcher {
    fn fetch(&self, url: &str) -> Result<Hop, String> {
        let resp = self.agent.get(url).call().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        if (300..400).contains(&status) {
            let loc = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("Weiterleitung {status} ohne Ziel"))?;
            return Ok(Hop::Redirect(loc.to_string()));
        }
        if !(200..300).contains(&status) {
            return Ok(Hop::Status(status));
        }
        let len = resp.body().content_length();
        let reader = resp
            .into_body()
            .into_with_config()
            .limit(MAX_ASSET)
            .reader();
        Ok(Hop::Body {
            reader: Box::new(reader),
            len,
        })
    }
}

/// Resolves a Location header against the current URL. Only https results.
fn resolve(base: &str, loc: &str) -> Result<String, String> {
    let loc = loc.trim();
    let abs = if loc.starts_with("https://") {
        loc.to_string()
    } else if let Some(rest) = loc.strip_prefix("//") {
        format!("https://{rest}")
    } else if loc.starts_with('/') {
        let after = base
            .strip_prefix("https://")
            .ok_or("Basis ist nicht HTTPS")?;
        let host = after.split('/').next().unwrap_or(after);
        format!("https://{host}{loc}")
    } else if loc.contains("://") {
        return Err(format!(
            "Weiterleitung auf unsichere Adresse abgelehnt: {loc}"
        ));
    } else {
        return Err(format!("relative Weiterleitung nicht unterstützt: {loc}"));
    };
    Ok(abs)
}

fn host(url: &str) -> &str {
    let after = url.split_once("://").map(|x| x.1).unwrap_or(url);
    after.split(['/', '?', '#']).next().unwrap_or(after)
}

/// Human label for the update log: "GitHub" or "Spiegel ramforge.org".
pub fn source_label(url: &str) -> String {
    let h = host(url).to_ascii_lowercase();
    if h == "github.com" || h.ends_with(".github.com") || h.ends_with("githubusercontent.com") {
        "GitHub".into()
    } else if h == "ramforge.org" || h.ends_with(".ramforge.org") {
        "Spiegel ramforge.org".into()
    } else {
        h
    }
}

/// Follows up to MAX_REDIRECTS https-only redirects and returns the body.
fn open(f: &dyn Fetcher, url: &str) -> Result<(Box<dyn Read + Send>, Option<u64>), String> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        if !current.starts_with("https://") {
            return Err(format!("nur HTTPS erlaubt: {current}"));
        }
        match f.fetch(&current)? {
            Hop::Body { reader, len } => return Ok((reader, len)),
            Hop::Status(s) => return Err(format!("HTTP {s}")),
            Hop::Redirect(loc) => current = resolve(&current, &loc)?,
        }
    }
    Err(format!("mehr als {MAX_REDIRECTS} Weiterleitungen"))
}

/// Result of a successful download.
#[derive(Clone, Debug, PartialEq)]
pub struct Downloaded {
    pub url: String,
    /// "GitHub", "Spiegel ramforge.org", or the host name.
    pub source: String,
}

fn try_one(
    f: &dyn Fetcher,
    url: &str,
    asset: &Asset,
    dest: &Path,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), UpdateError> {
    let (mut reader, len) = open(f, url).map_err(UpdateError::Network)?;
    if asset.size > 0 {
        if let Some(l) = len.filter(|&l| l > asset.size) {
            return Err(UpdateError::SizeMismatch {
                expected: asset.size,
                actual: l,
            });
        }
    }
    let total = if asset.size > 0 {
        asset.size
    } else {
        len.unwrap_or(0)
    };
    let limit = if asset.size > 0 {
        asset.size
    } else {
        MAX_ASSET
    };
    let mut file = std::fs::File::create(dest).map_err(|e| UpdateError::Io(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(UpdateError::Cancelled);
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| UpdateError::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > limit {
            // More bytes than the signed manifest allows: stop reading at once.
            return Err(UpdateError::SizeMismatch {
                expected: limit,
                actual: done,
            });
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| UpdateError::Io(e.to_string()))?;
        progress(done, total);
    }
    file.sync_all()
        .map_err(|e| UpdateError::Io(e.to_string()))?;
    drop(file);
    verify_digest(&asset.sha256, asset.size, &hasher.finalize(), done)
}

/// Downloads `asset` to `dest`, trying its URLs in order. On success `dest`
/// holds exactly the bytes whose SHA-256 the signed manifest names. On failure
/// `dest` does not exist.
pub fn download_asset(
    f: &dyn Fetcher,
    asset: &Asset,
    dest: &Path,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> Result<Downloaded, UpdateError> {
    let mut errors = Vec::new();
    for url in &asset.urls {
        let label = source_label(url);
        if !url.starts_with("https://") {
            errors.push(format!("{label}: keine HTTPS-Adresse"));
            continue;
        }
        match try_one(f, url, asset, dest, progress, cancel) {
            Ok(()) => {
                return Ok(Downloaded {
                    url: url.clone(),
                    source: label,
                })
            }
            Err(UpdateError::Cancelled) => {
                let _ = std::fs::remove_file(dest);
                return Err(UpdateError::Cancelled);
            }
            Err(e) => {
                let _ = std::fs::remove_file(dest);
                errors.push(format!("{label}: {e}"));
            }
        }
    }
    let _ = std::fs::remove_file(dest);
    Err(UpdateError::AllSourcesFailed(errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    enum Fake {
        Body(Vec<u8>),
        BodyNoLen(Vec<u8>),
        Redirect(&'static str),
        Status(u16),
        NetErr,
    }

    #[derive(Default)]
    struct FakeFetcher {
        routes: HashMap<String, Fake>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeFetcher {
        fn route(mut self, url: &str, r: Fake) -> Self {
            self.routes.insert(url.into(), r);
            self
        }
    }

    impl Fetcher for FakeFetcher {
        fn fetch(&self, url: &str) -> Result<Hop, String> {
            self.calls.lock().unwrap().push(url.to_string());
            match self.routes.get(url) {
                Some(Fake::Body(b)) => Ok(Hop::Body {
                    reader: Box::new(std::io::Cursor::new(b.clone())),
                    len: Some(b.len() as u64),
                }),
                Some(Fake::BodyNoLen(b)) => Ok(Hop::Body {
                    reader: Box::new(std::io::Cursor::new(b.clone())),
                    len: None,
                }),
                Some(Fake::Redirect(l)) => Ok(Hop::Redirect(l.to_string())),
                Some(Fake::Status(s)) => Ok(Hop::Status(*s)),
                Some(Fake::NetErr) | None => Err("Verbindung fehlgeschlagen".into()),
            }
        }
    }

    const GH: &str =
        "https://github.com/airichde/ramforge/releases/download/v0.2.0/RAMForge-0.2.0.exe";
    const GH_CDN: &str =
        "https://release-assets.githubusercontent.com/abc/RAMForge-0.2.0.exe?sig=1";
    const MIRROR: &str = "https://ramforge.org/dl/RAMForge-0.2.0.exe";
    const GOOD: &[u8] = b"RAMForge 0.2.0 binary";

    fn asset(size: u64) -> Asset {
        let h: String = Sha256::digest(GOOD)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Asset {
            urls: vec![GH.into(), MIRROR.into()],
            sha256: h,
            size,
        }
    }

    fn dest(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ramforge-fetch-{}-{name}.new", std::process::id()))
    }

    fn run(
        f: &FakeFetcher,
        a: &Asset,
        name: &str,
    ) -> (Result<Downloaded, UpdateError>, std::path::PathBuf) {
        let d = dest(name);
        let r = download_asset(f, a, &d, &|_, _| {}, &AtomicBool::new(false));
        (r, d)
    }

    #[test]
    fn github_via_redirect_is_used_first() {
        let f = FakeFetcher::default()
            .route(GH, Fake::Redirect(GH_CDN))
            .route(GH_CDN, Fake::Body(GOOD.to_vec()));
        let (r, d) = run(&f, &asset(GOOD.len() as u64), "gh");
        assert_eq!(r.unwrap().source, "GitHub");
        assert_eq!(std::fs::read(&d).unwrap(), GOOD);
        assert!(
            !f.calls.lock().unwrap().contains(&MIRROR.to_string()),
            "Spiegel darf nicht nötig sein"
        );
        std::fs::remove_file(d).ok();
    }

    #[test]
    fn first_url_failing_falls_back_to_mirror() {
        for first in [Fake::NetErr, Fake::Status(404), Fake::Status(503)] {
            let f = FakeFetcher::default()
                .route(GH, first)
                .route(MIRROR, Fake::Body(GOOD.to_vec()));
            let (r, d) = run(&f, &asset(0), "fallback");
            let got = r.unwrap();
            assert_eq!(got.source, "Spiegel ramforge.org");
            assert_eq!(got.url, MIRROR);
            assert_eq!(std::fs::read(&d).unwrap(), GOOD);
            std::fs::remove_file(d).ok();
        }
    }

    #[test]
    fn wrong_hash_is_rejected_and_next_source_used() {
        let f = FakeFetcher::default()
            .route(GH, Fake::Body(b"manipuliert".to_vec()))
            .route(MIRROR, Fake::Body(GOOD.to_vec()));
        let (r, d) = run(&f, &asset(0), "hash1");
        assert_eq!(r.unwrap().source, "Spiegel ramforge.org");
        std::fs::remove_file(d).ok();
    }

    #[test]
    fn mirror_with_wrong_hash_is_rejected() {
        let f = FakeFetcher::default()
            .route(GH, Fake::NetErr)
            .route(MIRROR, Fake::Body(b"boese Datei".to_vec()));
        let (r, d) = run(&f, &asset(0), "hash2");
        match r {
            Err(UpdateError::AllSourcesFailed(e)) => {
                assert_eq!(e.len(), 2);
                assert!(e[1].contains("Prüfsumme"), "{e:?}");
            }
            o => panic!("{o:?}"),
        }
        assert!(
            !d.exists(),
            "fehlgeschlagener Download darf nicht liegen bleiben"
        );
    }

    #[test]
    fn http_redirect_is_rejected() {
        let f = FakeFetcher::default()
            .route(GH, Fake::Redirect("http://evil.example/RAMForge.exe"))
            .route(MIRROR, Fake::NetErr);
        let (r, d) = run(&f, &asset(0), "http");
        match r {
            Err(UpdateError::AllSourcesFailed(e)) => assert!(
                e[0].contains("unsichere") || e[0].contains("HTTPS"),
                "{e:?}"
            ),
            o => panic!("{o:?}"),
        }
        assert!(
            !f.calls
                .lock()
                .unwrap()
                .iter()
                .any(|u| u.starts_with("http://")),
            "http-Ziel darf nie abgerufen werden"
        );
        assert!(!d.exists());
    }

    #[test]
    fn too_many_redirects_and_oversize_are_rejected() {
        let f = FakeFetcher::default()
            .route(GH, Fake::Redirect("https://a.example/1"))
            .route("https://a.example/1", Fake::Redirect("/2"))
            .route("https://a.example/2", Fake::Redirect("/3"))
            .route("https://a.example/3", Fake::Redirect("/4"))
            .route("https://a.example/4", Fake::Redirect("/5"))
            .route("https://a.example/5", Fake::Redirect("/6"))
            .route("https://a.example/6", Fake::Body(GOOD.to_vec()))
            // Mirror sends more bytes than the manifest allows (no length header).
            .route(MIRROR, Fake::BodyNoLen([GOOD, b"extra"].concat()));
        let (r, d) = run(&f, &asset(GOOD.len() as u64), "limits");
        match r {
            Err(UpdateError::AllSourcesFailed(e)) => {
                assert!(e[0].contains("Weiterleitungen"), "{e:?}");
                assert!(e[1].contains("Größe"), "{e:?}");
            }
            o => panic!("{o:?}"),
        }
        assert!(!d.exists());
    }

    #[test]
    fn cancel_stops_without_fallback() {
        let f = FakeFetcher::default()
            .route(GH, Fake::Body(GOOD.to_vec()))
            .route(MIRROR, Fake::Body(GOOD.to_vec()));
        let d = dest("cancel");
        let r = download_asset(&f, &asset(0), &d, &|_, _| {}, &AtomicBool::new(true));
        assert_eq!(r, Err(UpdateError::Cancelled));
        assert_eq!(f.calls.lock().unwrap().len(), 1);
        assert!(!d.exists());
    }

    #[test]
    fn labels_and_resolution() {
        assert_eq!(source_label(GH), "GitHub");
        assert_eq!(source_label(GH_CDN), "GitHub");
        assert_eq!(source_label(MIRROR), "Spiegel ramforge.org");
        assert_eq!(
            resolve("https://github.com/a/b", "/c").unwrap(),
            "https://github.com/c"
        );
        assert_eq!(
            resolve("https://github.com/a", "//cdn.example/x").unwrap(),
            "https://cdn.example/x"
        );
        assert!(resolve("https://github.com/a", "http://x/y").is_err());
        assert!(resolve("https://github.com/a", "ftp://x/y").is_err());
    }
}
