// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Tiny two-language switch (German / English).
//!
//! Strings live next to the code that uses them as `tr("Deutsch", "English")`
//! pairs. That keeps every label readable in place and needs no key tables.
//! The language follows the system locale unless the user picks one.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

static CURRENT: AtomicU8 = AtomicU8::new(1);

/// `setting` is the value stored in the config: "de", "en" or "auto".
pub fn init(setting: &str) {
    let lang = match setting {
        "de" => Lang::De,
        "en" => Lang::En,
        _ => detect(),
    };
    set(lang);
}

pub fn detect() -> Lang {
    let locale = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    if locale.starts_with("de") {
        Lang::De
    } else {
        Lang::En
    }
}

pub fn set(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::De => 0,
            Lang::En => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 0 {
        Lang::De
    } else {
        Lang::En
    }
}

pub fn is_de() -> bool {
    current() == Lang::De
}

/// Pick the German or the English text.
#[inline]
pub fn tr(de: &'static str, en: &'static str) -> &'static str {
    if is_de() {
        de
    } else {
        en
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_language() {
        set(Lang::De);
        assert_eq!(tr("Geräte", "Devices"), "Geräte");
        set(Lang::En);
        assert_eq!(tr("Geräte", "Devices"), "Devices");
    }
}
