//! Hyprland : la fenêtre de mpv est créée au lancement (dans le fil du lecteur, sans retarder la
//! bibliothèque) sous le titre `bioplay-attente` ; une règle étroite (classe ET titre initial)
//! la met de côté sur le bureau spécial `special:bioplay`. Elle est ramenée, agrandie, à la
//! première image d'une vidéo, et remise de côté au retour : ni fenêtre à créer ni carte
//! graphique à initialiser au clic. Une fenêtre créée autrement (vidéo donnée au lancement)
//! n'est jamais visée par la règle. Hors Hyprland : rien (mpv crée et ferme sa fenêtre).
//! Commandes envoyées directement sur la prise de Hyprland (pas de `hyprctl` à lancer).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

const WINDOW: &str = "class:^bioplay-video$";
const ASIDE: &str = "special:bioplay";

fn socket() -> Option<PathBuf> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let run = std::env::var_os("XDG_RUNTIME_DIR")?;
    let p = PathBuf::from(run).join("hypr").join(sig).join(".socket.sock");
    p.exists().then_some(p)
}

/// Sous Hyprland (et pas désactivé par `BIOPLAY_NOKEEP`).
pub fn available() -> bool {
    std::env::var_os("BIOPLAY_NOKEEP").is_none() && socket().is_some()
}

fn request(msg: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket()?).ok()?;
    s.write_all(msg.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

fn dispatch(expr: &str) {
    let _ = request(&format!("dispatch {expr}"));
}

/// Numéro du bureau affiché sur l'écran actif.
fn active_workspace() -> Option<i64> {
    let v: serde_json::Value = serde_json::from_str(&request("j/activeworkspace")?).ok()?;
    v.get("id")?.as_i64()
}

/// Bureau où se trouve la fenêtre vidéo.
fn video_workspace() -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&request("j/clients")?).ok()?;
    v.as_array()?
        .iter()
        .find(|c| c.get("class").and_then(|x| x.as_str()) == Some("bioplay-video"))
        .and_then(|c| c.get("workspace")?.get("name")?.as_str().map(str::to_owned))
}

/// Ramène la fenêtre vidéo sur le bureau courant, agrandie et active ; vérifie qu'elle y est
/// (une seconde tentative sinon) : une vidéo ne doit jamais rester cachée.
pub fn bring_video() {
    for _ in 0..2 {
        let Some(ws) = active_workspace() else { return };
        dispatch(&format!("hl.dsp.window.move({{ workspace = \"{ws}\", follow = true, window = \"{WINDOW}\" }})"));
        dispatch(&format!("hl.dsp.window.fullscreen({{ mode = \"maximized\", action = \"set\", window = \"{WINDOW}\" }})"));
        dispatch(&format!("hl.dsp.focus({{ window = \"{WINDOW}\" }})"));
        if !video_workspace().is_some_and(|w| w.starts_with("special")) {
            return;
        }
    }
}

/// Met la fenêtre vidéo de côté (bureau spécial, invisible).
pub fn set_video_aside() {
    dispatch(&format!("hl.dsp.window.move({{ workspace = \"{ASIDE}\", follow = false, window = \"{WINDOW}\" }})"));
}
