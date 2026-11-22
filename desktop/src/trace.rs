//! Comptage des causes (mesures) : chaque réveil de la boucle, chaque image dessinée, chaque appel
//! à mpv, avec sa raison. Sert à vérifier que rien ne se fait sans nécessité : dans une situation
//! où rien ne change (lecture commandes masquées, pause, accueil au repos), tout compteur non nul
//! est suspect. Inactif hors mesure : une lecture atomique par appel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);
static COUNTS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

pub fn enable() {
    ON.store(true, Ordering::Relaxed);
}

#[inline]
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Compte une occurrence de `what` (« réveil : … », « image : … », « mpv get … »).
#[inline]
pub fn count(what: &str) {
    if on() {
        add(what);
    }
}

/// Variante qui ne construit le libellé que si la mesure est active.
#[inline]
pub fn count_with(f: impl FnOnce() -> String) {
    if on() {
        add(&f());
    }
}

fn add(what: &str) {
    let mut g = COUNTS.lock().unwrap_or_else(|e| e.into_inner());
    *g.get_or_insert_with(HashMap::new).entry(what.to_owned()).or_default() += 1;
}

/// Compteurs depuis le dernier appel (remis à zéro).
pub fn take() -> HashMap<String, u64> {
    COUNTS.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default()
}
