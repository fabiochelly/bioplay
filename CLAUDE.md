# bioplay

Lecteur vidéo minimal (Android + bureau) construit sur libmpv, interface « verre liquide ».

- `core/` : Rust, commun. Index de tous les sous-titres d'un fichier (MKV parcouru en sautant
  l'audio/vidéo ; SRT/ASS/VTT). `cargo test -p bioplay-core`, `cargo run --release --example dump -- f.mkv`.
  `keyframes()` : images clés lues dans les Cues (≈ 0,1 ms) ; le bureau saute dessus (≈ 30 ms au lieu
  de 200-700 ms précis) : 10 s → toujours l'image clé la plus proche ; réplique (≤ 10 s) → précis,
  image clé seulement dans les 500 ms qui précèdent. `--example keyframes|hits -- f.mkv`.
- `app/` : Android (Kotlin + Compose), libmpv via `dev.jdtech.mpv:libmpv`. `./dev.sh` compile,
  installe sur l'émulateur et ouvre `samples/test.mkv` (copié dans `files/` de l'app via run-as).
- `desktop/` : bureau (Rust). Deux fenêtres :
  - la bibliothèque (`libwin.rs`, `ui/library.slint`) : Slint, winit + Skia, fenêtre transparente
    (vitre sombre, Hyprland floute le fond d'écran). Grille par rangées (seules les rangées visibles
    existent, miniatures lues à ce moment). Capture sans fenêtre : `BIOPLAY_LIBSNAP=f.png
    [BIOPLAY_LIBPLACE=sous/dossier] [BIOPLAY_LIBVIEW=list] [BIOPLAY_LIBSHEET=1] bioplay`.
    Mesures en vrai : `BIOPLAY_AUTOPLAY=film.mkv BIOPLAY_DEBUG_OPEN=1` (heures du clic et de la
    première image), `BIOPLAY_SCROLLTEST=1` (processeur pendant 4 s de défilement).
  - la lecture : mpv (préparé dès le lancement dans le fil « lecteur », sans fenêtre tant qu'il n'y
    a pas de vidéo) ouvre SA fenêtre en gpu-next (même image et même coût que mpv, Dolby Vision
    compris) avec `~/.config/mpv/mpv.conf` et `input.conf`. Les commandes sont egui, rendues sur le
    CPU (`overlay.rs`) et posées par `overlay-add` ; le verre est `shaders/glass.glsl`. Bouton
    « Bibliothèque » ou fermeture de la fenêtre : retour à la bibliothèque (ou fin si la vidéo
    venait de l'extérieur). Raccourcis via `define-section` + `script-binding main/…`.
  - Hyprland (`~/.config/hypr/hyprland.lua`) : classes `bioplay` (bibliothèque) et `bioplay-video`
    (mpv) sans animation ; la vidéo s'ouvre agrandie par-dessus la bibliothèque, restée en place.
  `./desktop/install.sh` installe dans `~/.local`. `--panel=list|tracks|…` ouvre un tiroir ; toute
  autre option `--k=v` est passée à mpv.

- Accueil (Android et bureau) : barre compacte (ouvrir, « Dossiers », liste/miniatures, drapeaux)
  puis les vidéos des dossiers surveillés. Android : `Library.kt` (arbres SAF, miniatures WebP dans
  `no_backup/thumbs`). Bureau : `library.rs` (dossiers dans `~/.local/state/bioplay/folders`,
  miniatures PNG tirées par ffmpeg dans `~/.cache/bioplay/thumbs`). Capture des commandes de lecture : `BIOPLAY_BENCH=snapshot BIOPLAY_SNAPSHOT=f.png
  [BIOPLAY_SEEK=±1] bioplay film.mkv` ;
  `BIOPLAY_BENCH=play bioplay --hwdec=auto-copy film.mkv` : vraie boucle, réveils, images, envois
  et CPU (processus, fils) commandes visibles / masquées / sauts. Redessin seulement de ce qui
  change, forme par forme (`shape_marks`), et seulement quand l'affichage change (seconde, pixel). Relecture en arrière-plan à chaque lancement, rien
  pendant la lecture.

- Partages réseau (Android, `Smb.kt`, smbj) : adresses `smb://hôte[:port]/partage/chemin`, compte par
  hôte dans les préférences. Le fichier est remis à mpv / l'index / les miniatures comme un
  descripteur ordinaire (`openProxyFileDescriptor`, lecture anticipée adaptative). L'index MKV passe
  par les Cues quand elles couvrent les sous-titres (sinon parcours complet). Essai sans interface
  (debug) : `am start -n app.bioplay/.MainActivity --es smbtest "hôte:port|user|pass|smb://…"`.
- TV (Google TV) : lanceur leanback + bannière, tout se pilote aux flèches (`focusRing`), OK =
  pause, ←/→ = 10 s, Retour masque les commandes ; sans sélecteur de fichiers, « Partage réseau ».

- `shared/flags.txt` : drapeaux des langues (formes simples + codes + mots de titre), lus par
  `desktop/src/flags.rs` et `app/.../ui/Flags.kt`. Ajouter un drapeau = l'ajouter ici seulement.

Règles : la position de lecture est toujours mémorisée (clé nom+taille du fichier) et reprise.
Textes de l'interface en français.
