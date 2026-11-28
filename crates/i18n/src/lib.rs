//! UI language of RAMForge: German or English.
//!
//! Both texts stand side by side where they are used, so a missing
//! translation is visible in the code itself:
//!
//! ```
//! use ramforge_i18n::{t, tf};
//! let label = t!("Test starten", "Start test");
//! let n = 3;
//! let msg = tf!("{n} Fehler", "{n} errors");
//! # let _ = (label, msg);
//! ```
//!
//! The language is chosen once at start from the saved setting ("auto",
//! "de", "en"); "auto" follows the Windows display language or `LANG`.
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if LANG.load(Ordering::Relaxed) == Lang::En as u8 {
        Lang::En
    } else {
        Lang::De
    }
}

pub fn is_en() -> bool {
    get() == Lang::En
}

/// "de" or "en" (news requests, reports, HTML `lang`).
pub fn code() -> &'static str {
    match get() {
        Lang::De => "de",
        Lang::En => "en",
    }
}

/// Saved preference ("auto", "de", "en") to a language.
pub fn resolve(pref: &str) -> Lang {
    match pref {
        "de" => Lang::De,
        "en" => Lang::En,
        _ => detect(),
    }
}

/// Sets the language from the saved preference and returns it.
pub fn init(pref: &str) -> Lang {
    let l = resolve(pref);
    set(l);
    l
}

/// System language: German if the Windows display language (or `LANG` /
/// `LC_ALL` / `LC_MESSAGES` on Linux) is German, English otherwise.
pub fn detect() -> Lang {
    #[cfg(windows)]
    {
        // Primary language id 0x07 = German (all regions).
        let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        return if id & 0x3ff == 0x07 {
            Lang::De
        } else {
            Lang::En
        };
    }
    #[allow(unreachable_code)]
    {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    return if v.starts_with("de") {
                        Lang::De
                    } else {
                        Lang::En
                    };
                }
            }
        }
        Lang::En
    }
}

/// Text in the current language: `t!("Deutsch", "English")`.
#[macro_export]
macro_rules! t {
    ($de:expr, $en:expr $(,)?) => {
        if $crate::is_en() {
            $en
        } else {
            $de
        }
    };
}

/// Formatted text in the current language: `tf!("{n} Fehler", "{n} errors")`.
/// Both format strings may use the same inline or positional arguments.
#[macro_export]
macro_rules! tf {
    // The rest is passed through as tokens, so named arguments (`u = x`)
    // work like in format!.
    ($de:literal, $en:literal $($rest:tt)*) => {
        if $crate::is_en() {
            format!($en $($rest)*)
        } else {
            format!($de $($rest)*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_and_macros() {
        set(Lang::En);
        assert_eq!(t!("Ja", "Yes"), "Yes");
        let n = 2;
        assert_eq!(tf!("{n} Fehler", "{n} errors"), "2 errors");
        assert_eq!(code(), "en");
        set(Lang::De);
        assert_eq!(t!("Ja", "Yes"), "Ja");
        assert_eq!(tf!("{} von {}", "{} of {}", 1, 3), "1 von 3");
        assert_eq!(resolve("en"), Lang::En);
    }
}
