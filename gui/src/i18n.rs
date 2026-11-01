//! Small UI translation layer.
//!
//! English source strings are the lookup keys. Missing translations fall back
//! to English, so a partially translated language still works and new UI text
//! never breaks the app.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    #[serde(alias = "en")]
    English,
    #[serde(alias = "fr")]
    French,
    #[serde(alias = "de")]
    German,
    #[serde(alias = "es")]
    Spanish,
}

impl Language {
    pub const ALL: [Language; 4] = [
        Language::English,
        Language::French,
        Language::German,
        Language::Spanish,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::French => "Français",
            Language::German => "Deutsch",
            Language::Spanish => "Español",
        }
    }

    #[allow(dead_code)]
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::French => "fr",
            Language::German => "de",
            Language::Spanish => "es",
        }
    }
}

static ACTIVE: OnceLock<RwLock<Language>> = OnceLock::new();

fn active() -> &'static RwLock<Language> {
    ACTIVE.get_or_init(|| RwLock::new(Language::default()))
}

pub fn set_language(language: Language) {
    *active()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = language;
}

pub fn current() -> Language {
    *active()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn tr(text: &'static str) -> &'static str {
    let language = current();
    if language == Language::English {
        return text;
    }
    dictionary(language).get(text).copied().unwrap_or(text)
}

fn dictionary(language: Language) -> &'static HashMap<&'static str, &'static str> {
    static FRENCH: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static GERMAN: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static SPANISH: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    match language {
        Language::English => {
            static EMPTY: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
            EMPTY.get_or_init(HashMap::new)
        }
        Language::French => FRENCH.get_or_init(|| parse(include_str!("../../assets/i18n/fr.json"))),
        Language::German => GERMAN.get_or_init(|| parse(include_str!("../../assets/i18n/de.json"))),
        Language::Spanish => {
            SPANISH.get_or_init(|| parse(include_str!("../../assets/i18n/es.json")))
        }
    }
}

fn parse(text: &str) -> HashMap<&'static str, &'static str> {
    let entries: HashMap<String, String> = serde_json::from_str(text).unwrap_or_else(|error| {
        eprintln!("MusicWasp: invalid i18n catalogue: {error}");
        HashMap::new()
    });
    entries
        .into_iter()
        .map(|(key, value)| {
            (
                Box::leak(key.into_boxed_str()) as &'static str,
                Box::leak(value.into_boxed_str()) as &'static str,
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/i18n.rs"]
mod tests;
