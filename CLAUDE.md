# bioplay

Lecteur vidéo minimal (Android + bureau) construit sur libmpv, interface « verre liquide ».

- `core/` : Rust, commun. Index de tous les sous-titres d'un fichier (MKV parcouru en sautant
  l'audio/vidéo ; SRT/ASS/VTT). `cargo test -p bioplay-core`, `cargo run --release --example dump -- f.mkv`.
- `app/` : Android (Kotlin + Compose), libmpv via `dev.jdtech.mpv:libmpv`. `./dev.sh` compile,
  installe sur l'émulateur et ouvre `samples/test.mkv` (copié dans `files/` de l'app via run-as).
- `desktop/` : bureau (Rust : winit + glutin + egui), libmpv système rendu dans une texture GL ;
  le verre (flou dual Kawase + réfraction) est dans `gfx.rs`. mpv charge `~/.config/mpv/mpv.conf`
  et `input.conf` ; les touches lui sont transmises par `keypress`. `./desktop/install.sh` installe
  dans `~/.local`. `--panel=list|tracks|speed|zoom` ouvre un tiroir au démarrage (captures).

Règles : la position de lecture est toujours mémorisée (clé nom+taille du fichier) et reprise.
Textes de l'interface en français.
