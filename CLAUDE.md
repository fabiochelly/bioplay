# bioplay

Lecteur vidéo minimal (Android + bureau) construit sur libmpv, interface « verre liquide ».

- `core/` : Rust, commun. Index de tous les sous-titres d'un fichier (MKV parcouru en sautant
  l'audio/vidéo ; SRT/ASS/VTT). `cargo test -p bioplay-core`, `cargo run --release --example dump -- f.mkv`.
  `keyframes()` : images clés lues dans les Cues (≈ 0,1 ms) ; le bureau saute dessus (≈ 30 ms au lieu
  de 200-700 ms précis) : 10 s → toujours l'image clé la plus proche ; réplique (≤ 10 s) → précis,
  image clé seulement dans les 500 ms qui précèdent. `--example keyframes|hits -- f.mkv`.
- `app/` : Android (Kotlin + Compose), libmpv via `dev.jdtech.mpv:libmpv`. `./dev.sh` compile,
  installe sur l'émulateur et ouvre `samples/test.mkv` (copié dans `files/` de l'app via run-as).
- `desktop/` : bureau (Rust). mpv ouvre et dessine SA fenêtre en gpu-next (même image et même coût
  que mpv, Dolby Vision compris) avec `~/.config/mpv/mpv.conf` et `input.conf`. L'interface egui est
  rendue hors écran (EGL surfaceless, `overlay.rs`) et posée par `overlay-add` ; le verre est le
  shader `shaders/glass.glsl` (inactif quand `glass_n=0`). Raccourcis propres via `define-section`
  + `script-binding main/…`. `./desktop/install.sh` installe dans `~/.local`. `--panel=list|tracks|…`
  ouvre un tiroir au démarrage ; toute autre option `--k=v` est passée à mpv.

- Accueil (Android et bureau) : barre compacte (ouvrir, « Dossiers », drapeaux ; Android a aussi
  la bascule liste/miniatures) puis les vidéos des dossiers surveillés, en miniatures sur le bureau,
  sur un fond uni #060912. Android : `Library.kt` (arbres SAF, miniatures WebP dans
  `no_backup/thumbs`). Bureau : `library.rs` (dossiers dans `~/.local/state/bioplay/folders`,
  miniatures PNG tirées par ffmpeg dans `~/.cache/bioplay/thumbs`). Une seule incrustation mpv
  (mpv renvoie TOUTES ses incrustations au GPU à chaque changement : jamais une par miniature) ;
  fond, miniatures (mises à l'échelle une fois, copiées par lignes) et interface y sont composés
  par bandes, une par cœur ; pendant un défilement, en demi-résolution agrandie par mpv (`dw/dh`).
  Accueil sans verre (boutons opaques, texte blanc net) ; le verre est pour la vidéo (mesuré :
  ≈ 0,5 ms de carte graphique par image, aucune image perdue, `BIOPLAY_GLASSTEST=1 bioplay film`).
  Barre de lecture sans fondu (apparition/disparition instantanées) ; partie lue en bleu électrique.
  Vidéo HDR (`video-params/gamma` pq/hlg) : mpv éclaircit et sature l'interface (le bleu virait au
  cyan) ; bioplay envoie alors des couleurs compensées (`PROGRESS_HDR`, `ACCENT_HDR`, `accent()`)
  trouvées par mesure : `BIOPLAY_SWATCHES=r0-r1/pas,g…,b… BIOPLAY_SHOT=f.png BIOPLAY_GLASSTEST=1
  bioplay film_hdr` (grille de cases rendue par mpv lui-même, `screenshot-to-file … window`). Tuiles en
  cache (`overlay::Sprite`, `ui::Rec`) : titre, durée, avancée, liseré, dossier dessinés une fois
  par aspect puis recopiés (tronçons non transparents) ; seule la zone qui change est refaite,
  le reste recopié de l'image précédente. Fond uni : miniatures (coins mélangés au fond) et tuiles opaques, simplement recopiées. Mesure en
  vrai du défilement : `BIOPLAY_SCROLLTEST=1 bioplay` (processeur par fil, position imposée à la
  grille, indépendante de la souris). Mesure sans fenêtre : `BIOPLAY_BENCH=grid [BIOPLAY_PLACE=sous/dossier]
  [BIOPLAY_SNAPSHOT=f.png] bioplay [fichier]` (durées par étape, ou capture de l'interface) ;
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
  `BIOPLAY_FLAGS=1 bioplay` affiche la galerie sur l'accueil.

Règles : la position de lecture est toujours mémorisée (clé nom+taille du fichier) et reprise.
Textes de l'interface en français.
