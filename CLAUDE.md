# bioplay

Lecteur vidéo minimal (Android + bureau) construit sur libmpv, interface « verre liquide ».

- `core/` : Rust, commun. Index de tous les sous-titres d'un fichier (MKV parcouru en sautant
  l'audio/vidéo ; SRT/ASS/VTT). `cargo test -p bioplay-core`, `cargo run --release --example dump -- f.mkv`.
- `app/` : Android (Kotlin + Compose), libmpv via `dev.jdtech.mpv:libmpv`. `./dev.sh` compile,
  installe sur l'émulateur et ouvre `samples/test.mkv` (copié dans `files/` de l'app via run-as).
- `desktop/` : bureau (Rust). mpv ouvre et dessine SA fenêtre en gpu-next (même image et même coût
  que mpv, Dolby Vision compris) avec `~/.config/mpv/mpv.conf` et `input.conf`. L'interface egui est
  rendue hors écran (EGL surfaceless, `overlay.rs`) et posée par `overlay-add` ; le verre est le
  shader `shaders/glass.glsl` (inactif quand `glass_n=0`). Raccourcis propres via `define-section`
  + `script-binding main/…`. `./desktop/install.sh` installe dans `~/.local`. `--panel=list|tracks|…`
  ouvre un tiroir au démarrage ; toute autre option `--k=v` est passée à mpv.

Règles : la position de lecture est toujours mémorisée (clé nom+taille du fichier) et reprise.
Textes de l'interface en français.
