//! News and opt-in usage statistics (ramforge.org API v1).
//!
//! What is sent, and nothing else (matches https://ramforge.org/privacy):
//! * News: program version and UI language (query string).
//! * Statistics, ONLY with explicit consent, at most once a day: a random
//!   UUIDv4 created locally (not derived from hardware), program version,
//!   edition and a coarse OS name like "Windows 11".
//! No hardware data, serial numbers, test results or logs ever leave the PC.

use crate::{agent, fetch_small, key, UpdateError};
#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use serde::{Deserialize, Serialize};

pub const NEWS_URL: &str = "https://ramforge.org/api/v1/news";
pub const PING_URL: &str = "https://ramforge.org/api/v1/ping";
pub const FORGET_URL: &str = "https://ramforge.org/api/v1/forget";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewsItem {
    pub id: i64,
    #[serde(default)]
    pub created: String,
    /// "info", "warning" or "critical"
    #[serde(default)]
    pub level: String,
    pub title: String,
    /// Plain text with \n line breaks. Never render as HTML.
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Deserialize)]
struct NewsResponse {
    items: Vec<NewsItem>,
}

fn online_enabled() -> Result<(), UpdateError> {
    if key::is_configured() {
        Ok(())
    } else {
        Err(UpdateError::NoKey)
    }
}

fn clean_version(v: &str) -> String {
    v.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        .take(32)
        .collect()
}

/// Fetches the news for this version. Untrusted input: links must be HTTPS,
/// texts are truncated, at most 20 items.
pub fn fetch_news(lang: &str, version: &str) -> Result<Vec<NewsItem>, UpdateError> {
    online_enabled()?;
    let lang = if lang == "en" { "en" } else { "de" };
    let url = format!("{NEWS_URL}?lang={lang}&version={}", clean_version(version));
    let body = fetch_small(&agent(), &url)?;
    let resp: NewsResponse =
        serde_json::from_slice(&body).map_err(|e| UpdateError::BadManifest(e.to_string()))?;
    Ok(resp
        .items
        .into_iter()
        .take(20)
        .map(|mut n| {
            if !n.url.starts_with("https://") {
                n.url.clear();
            }
            n.title = n.title.chars().take(200).collect();
            n.body = n.body.chars().take(4000).collect();
            n
        })
        .collect())
}

/// A new random install id (UUID version 4). Not derived from hardware.
pub fn new_install_id() -> String {
    let mut b = [0u8; 16];
    if getrandom::fill(&mut b).is_err() {
        // Extremely unlikely; fall back to time-based noise rather than a fixed id.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        b = t.to_le_bytes();
    }
    b[6] = (b[6] & 0x0F) | 0x40; // version 4
    b[8] = (b[8] & 0x3F) | 0x80; // RFC 4122 variant
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

pub fn is_uuid_v4(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 36
        && s.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
        && b[14] == b'4'
        && matches!(b[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b')
}

/// Coarse OS label for the statistics: only the product line, e.g. "Windows 11".
pub fn coarse_os(full: &str) -> String {
    for name in ["Windows 11", "Windows 10", "Windows Server"] {
        if full.contains(name) {
            return name.to_string();
        }
    }
    let s: String = full
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .take(40)
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "unknown".into()
    } else {
        s
    }
}

fn post_json(url: &str, body: String) -> Result<(), UpdateError> {
    online_enabled()?;
    let resp = agent()
        .post(url)
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(UpdateError::Network(format!("HTTP {}", resp.status())))
    }
}

/// Daily ping. The caller makes sure consent was given and limits it to once a day.
pub fn ping(install_id: &str, version: &str, edition: &str, os: &str) -> Result<(), UpdateError> {
    if !is_uuid_v4(install_id) {
        return Err(UpdateError::BadManifest(
            t!("ungültige Installations-ID", "invalid installation ID").into(),
        ));
    }
    let body = serde_json::json!({
        "install_id": install_id,
        "version": clean_version(version),
        "edition": edition,
        "os": coarse_os(os),
    });
    post_json(PING_URL, body.to_string())
}

/// Consent withdrawn: asks the server to delete everything stored for this id.
pub fn forget(install_id: &str) -> Result<(), UpdateError> {
    if !is_uuid_v4(install_id) {
        return Ok(());
    }
    post_json(
        FORGET_URL,
        serde_json::json!({ "install_id": install_id }).to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_ids_are_random_v4() {
        let a = new_install_id();
        let b = new_install_id();
        assert!(is_uuid_v4(&a), "{a}");
        assert_ne!(a, b);
        assert!(!is_uuid_v4("not-a-uuid"));
        assert!(is_uuid_v4("3F2504E0-4F89-41D3-9A0C-0305E82C3301"));
    }

    #[test]
    fn os_is_coarse() {
        assert_eq!(coarse_os("Windows 11 Pro 25H2 (Build 26200)"), "Windows 11");
        assert_eq!(coarse_os("Ubuntu 24.04 LTS"), "Ubuntu 24.04 LTS");
        assert_eq!(coarse_os("Évil<script>"), "vilscript");
    }
}
