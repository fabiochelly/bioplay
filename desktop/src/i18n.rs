//! Interface en français ou en anglais. Langue du système par défaut, bascule d'un clic,
//! choix mémorisé dans `~/.local/state/bioplay/lang`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

pub fn en() -> bool {
    EN.load(Ordering::Relaxed)
}

/// Le texte dans la langue choisie.
pub fn tr(fr: &'static str, en: &'static str) -> &'static str {
    if self::en() { en } else { fr }
}

fn file() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    base.join("bioplay/lang")
}

pub fn init() {
    let saved = std::fs::read_to_string(file()).ok();
    let en = match saved.as_deref().map(str::trim) {
        Some("en") => true,
        Some("fr") => false,
        _ => {
            let lang = std::env::var("LC_ALL").ok().filter(|v| !v.is_empty()).or_else(|| std::env::var("LANG").ok()).unwrap_or_default();
            !lang.starts_with("fr")
        }
    };
    EN.store(en, Ordering::Relaxed);
}

pub fn toggle() {
    let en = !en();
    EN.store(en, Ordering::Relaxed);
    let f = file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, if en { "en" } else { "fr" });
}
