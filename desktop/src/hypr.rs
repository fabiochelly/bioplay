//! Hyprland : garder la fenêtre de mpv d'une vidéo à l'autre. Créée normalement à la première
//! vidéo, elle est mise de côté au retour à la bibliothèque (bureau spécial `special:bioplay`,
//! invisible) au lieu d'être fermée, puis ramenée, agrandie, à la première image de la vidéo
//! suivante : ni nouvelle fenêtre ni initialisation de la carte graphique au clic.
//! Aucune règle de fenêtre n'envoie de fenêtre sur ce bureau : si une commande échoue, la
//! fenêtre reste simplement visible. Hors Hyprland : rien (mpv ferme sa fenêtre au retour).
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

/// Ramène la fenêtre vidéo sur le bureau courant, agrandie et active.
pub fn bring_video() {
    let Some(ws) = active_workspace() else { return };
    dispatch(&format!("hl.dsp.window.move({{ workspace = \"{ws}\", follow = true, window = \"{WINDOW}\" }})"));
    dispatch(&format!("hl.dsp.window.fullscreen({{ mode = \"maximized\", action = \"set\", window = \"{WINDOW}\" }})"));
    dispatch(&format!("hl.dsp.focus({{ window = \"{WINDOW}\" }})"));
}

/// Met la fenêtre vidéo de côté (bureau spécial, invisible).
pub fn set_video_aside() {
    dispatch(&format!("hl.dsp.window.move({{ workspace = \"{ASIDE}\", follow = false, window = \"{WINDOW}\" }})"));
}
