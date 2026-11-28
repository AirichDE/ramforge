//! The update signing PUBLIC keys. This is the only place they live.
//!
//! Two Ed25519 keys are trusted. A signature is valid if it verifies under
//! EITHER of them, and under no other key.
//!
//! * RELEASE_KEY signs every update (manifest and files).
//! * EMERGENCY_KEY is kept offline. It is only used if the release key is lost
//!   or stolen: an EXE with a new RELEASE_KEY is published, signed with the
//!   emergency key, and every installed copy accepts it.
//!
//! The private keys exist only on the maintainer's PC, encrypted with a
//! passphrase (`ramforge-sign keygen`). Never in git, never on the server.
//!
//! Key rotation: generate a new pair offline with
//! `ramforge-sign keygen --out <path outside the repo>`, paste the printed
//! array below and publish the new fingerprint on https://ramforge.org.
//! `cargo test -p ramforge-update -- --ignored` is the release gate.

#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use sha2::{Digest, Sha256};

/// Release key (2026-11-25), base64 `6X+e5Aj8uH8YW4i0GwYOw9E4DIAjyBiGww28XZBSloo=`
/// Fingerprint 79D1 AF24 BEAD 4207 50B6 D310 02F0 B850 677A D831 D426 A841 FBFD CF06 1868 87E1
pub const RELEASE_KEY: [u8; 32] = [
    0xE9, 0x7F, 0x9E, 0xE4, 0x08, 0xFC, 0xB8, 0x7F, 0x18, 0x5B, 0x88, 0xB4, 0x1B, 0x06, 0x0E, 0xC3,
    0xD1, 0x38, 0x0C, 0x80, 0x23, 0xC8, 0x18, 0x86, 0xC3, 0x0D, 0xBC, 0x5D, 0x90, 0x52, 0x96, 0x8A,
];
pub const RELEASE_KEY_IS_PLACEHOLDER: bool = false;

/// Emergency key (2026-11-25, offline), base64 `GMEBp0o15JJ/V1U1nNXytPCvvzcab+jGg78xJlTHV6w=`
/// Fingerprint 8568 B6C3 10CA 2122 F398 9AB6 E441 E77E B6AE 3BB3 7ED3 780F 021A 6308 E82E 288A
pub const EMERGENCY_KEY: [u8; 32] = [
    0x18, 0xC1, 0x01, 0xA7, 0x4A, 0x35, 0xE4, 0x92, 0x7F, 0x57, 0x55, 0x35, 0x9C, 0xD5, 0xF2, 0xB4,
    0xF0, 0xAF, 0xBF, 0x37, 0x1A, 0x6F, 0xE8, 0xC6, 0x83, 0xBF, 0x31, 0x26, 0x54, 0xC7, 0x57, 0xAC,
];
pub const EMERGENCY_KEY_IS_PLACEHOLDER: bool = false;

/// The keys a signature may verify under. All-zero entries are ignored.
pub fn trusted_keys() -> Vec<[u8; 32]> {
    [RELEASE_KEY, EMERGENCY_KEY]
        .into_iter()
        .filter(|k| *k != [0; 32])
        .collect()
}

/// At least one key is set; otherwise every online function stays off.
pub fn is_configured() -> bool {
    !trusted_keys().is_empty()
}

/// True as long as a placeholder or test key is compiled in.
pub fn is_placeholder() -> bool {
    RELEASE_KEY_IS_PLACEHOLDER || EMERGENCY_KEY_IS_PLACEHOLDER
}

/// Fingerprint, identical everywhere (app, `--version`, signing tool, website):
/// SHA-256 over the raw 32-byte public key, uppercase hex, groups of 4
/// separated by single spaces.
pub fn fingerprint_of(key: &[u8; 32]) -> String {
    let h = Sha256::digest(key);
    let hex: String = h.iter().map(|b| format!("{b:02X}")).collect();
    hex.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

fn fp_or_unset(key: &[u8; 32]) -> String {
    if *key == [0; 32] {
        t!("nicht hinterlegt", "not set").into()
    } else {
        fingerprint_of(key)
    }
}

pub fn release_fingerprint() -> String {
    fp_or_unset(&RELEASE_KEY)
}

pub fn emergency_fingerprint() -> String {
    fp_or_unset(&EMERGENCY_KEY)
}

/// Lines for About pages and `--version`: ("Release", fp), ("Notfall", fp).
pub fn fingerprints() -> [(&'static str, String); 2] {
    [
        ("Release", release_fingerprint()),
        (t!("Notfall", "Emergency"), emergency_fingerprint()),
    ]
}

pub fn to_b64(key: &[u8; 32]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(key)
}
