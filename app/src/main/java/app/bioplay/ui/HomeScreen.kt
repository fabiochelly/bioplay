package app.bioplay.ui

import android.graphics.Bitmap
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.bioplay.Folder
import app.bioplay.Lang
import app.bioplay.Library
import app.bioplay.Video
import app.bioplay.clock
import app.bioplay.t
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private val Tile = RoundedCornerShape(14.dp)

/**
 * Accueil : une barre compacte (ouvrir, dossiers surveillés, langue) puis les vidéos des
 * dossiers surveillés, en miniatures ou en liste. Tourne avec le téléphone.
 */
@Composable
fun HomeScreen(
    library: Library,
    gridState: LazyGridState,
    listState: LazyListState,
    onOpen: () -> Unit,
    onAddFolder: () -> Unit,
    onPlay: (Video) -> Unit,
) {
    var folders by remember { mutableStateOf(false) }
    BackHandler(folders) { folders = false }
    Box(Modifier.fillMaxSize().background(Color(0xFF05070C))) {
        Backdrop()
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.displayCutout)) {
            NavBar(library, onOpen, onFolders = { folders = true })
            val videos = library.videos
            when {
                library.folders.isEmpty() -> Empty(
                    t("Choisissez les dossiers où sont vos vidéos.", "Choose the folders holding your videos."),
                    t("Ajouter un dossier", "Add a folder"), onAddFolder,
                )
                videos.isEmpty() -> Empty(
                    if (library.scanning) t("Recherche des vidéos…", "Looking for videos…")
                    else t("Aucune vidéo dans ces dossiers.", "No video in these folders."),
                    null, {},
                )
                library.grid -> LazyVerticalGrid(
                    GridCells.Adaptive(160.dp), Modifier.fillMaxSize(), gridState,
                    contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 24.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    items(videos, key = { it.uri }) { v -> GridTile(library, v) { onPlay(v) } }
                }
                else -> LazyColumn(
                    Modifier.fillMaxSize(), listState,
                    contentPadding = PaddingValues(start = 12.dp, end = 12.dp, top = 2.dp, bottom = 24.dp),
                ) {
                    items(videos, key = { it.uri }) { v -> ListRow(library, v) { onPlay(v) } }
                }
            }
        }
        if (folders) FoldersSheet(library, onAddFolder, onDismiss = { folders = false })
    }
}

/** Lumières de fond floues : c'est sur elles que le verre prend son relief. */
@Composable
private fun Backdrop() = Canvas(Modifier.fillMaxSize()) {
    fun glow(x: Float, y: Float, r: Float, c: Color) =
        drawCircle(Brush.radialGradient(listOf(c, Color.Transparent), Offset(size.width * x, size.height * y), size.maxDimension * r), size.maxDimension * r, Offset(size.width * x, size.height * y))
    glow(0.18f, 0.1f, 0.45f, Color(0x553A7BFF))
    glow(0.85f, 0.9f, 0.5f, Color(0x44A04BFF))
    glow(0.7f, 0.05f, 0.3f, Color(0x2828E0D0))
}

@Composable
private fun NavBar(library: Library, onOpen: () -> Unit, onFolders: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
            GlassButton(onOpen, size = 44.dp) { Icon(Icons.open, size = 22.dp) }
            Spacer(Modifier.width(10.dp))
            // Le seul bouton à libellé ; il rétrécit (points de suspension) sur un écran étroit.
            GlassButton(onFolders, Modifier.weight(1f, fill = false), shape = Capsule, size = 44.dp) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.folder, size = 19.dp)
                    BasicText(t("Dossiers", "Folders"), Modifier.padding(start = 8.dp).weight(1f, fill = false), maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(14, FontWeight.SemiBold))
                    if (library.folders.isNotEmpty()) BasicText("${library.folders.size}", style = text(13, FontWeight.SemiBold, Ink.accent), modifier = Modifier.padding(start = 8.dp))
                }
            }
        }
        Spacer(Modifier.width(10.dp))
        GlassButton(library::toggleView, size = 44.dp) { Icon(if (library.grid) Icons.rows else Icons.grid, size = 20.dp) }
        Spacer(Modifier.width(10.dp))
        FlagSwitch()
    }
}

/** Langue de l'interface : deux drapeaux, l'actif net et cerclé, l'autre estompé. */
@Composable
private fun FlagSwitch() {
    val context = LocalContext.current
    Row(
        Modifier.height(44.dp).clip(Capsule).background(Color.White.copy(alpha = 0.06f)).border(1.dp, Color.White.copy(alpha = 0.18f), Capsule).padding(horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        for ((code, en) in listOf("fr" to false, "en" to true)) {
            val on = Lang.en == en
            Box(
                Modifier.fillMaxHeight().padding(vertical = 4.dp).clip(Capsule)
                    .background(if (on) Ink.accent.copy(alpha = 0.22f) else Color.Transparent)
                    .clickable { if (!on) Lang.toggle(context) }
                    .padding(horizontal = 9.dp),
                contentAlignment = Alignment.Center,
            ) { LangFlag(code, Modifier.width(24.dp).height(16.dp), if (on) 1f else 0.5f) }
        }
    }
}

@Composable
private fun Empty(message: String, action: String?, onAction: () -> Unit) {
    Column(Modifier.fillMaxSize().padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
        BasicText(message, style = text(16, color = Ink.secondary))
        if (action != null) {
            Spacer(Modifier.height(18.dp))
            GlassButton(onAction, shape = Capsule, size = 50.dp) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.add, size = 20.dp)
                    BasicText(action, style = text(15, FontWeight.SemiBold), modifier = Modifier.padding(start = 8.dp, end = 4.dp))
                }
            }
        }
    }
}

// --- Vignettes ---------------------------------------------------------------------------------

/** Miniature lue du cache disque hors du fil principal ; rechargée quand elle vient d'être créée. */
@Composable
private fun thumb(library: Library, v: Video): Bitmap? {
    val made = library.thumbsMade
    val b by produceState<Bitmap?>(null, v.thumb, made) {
        if (value == null) value = withContext(Dispatchers.IO) { library.loadThumb(v) }
    }
    return b
}

@Composable
private fun Thumb(library: Library, v: Video, modifier: Modifier) {
    val b = thumb(library, v)
    Box(modifier.clip(Tile).background(Color.White.copy(alpha = 0.06f)).border(1.dp, Color.White.copy(alpha = 0.12f), Tile)) {
        if (b != null) Image(b.asImageBitmap(), null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        else Icon(Icons.play, Modifier.align(Alignment.Center), size = 26.dp, tint = Ink.faint)
        Progress(library, v)
        if (v.duration > 0) BasicText(
            clock(v.duration), style = text(11, FontWeight.SemiBold),
            modifier = Modifier.align(Alignment.TopEnd).padding(6.dp).clip(Capsule).background(Color.Black.copy(alpha = 0.55f)).padding(horizontal = 6.dp, vertical = 1.dp),
        )
    }
}

/** Avancée de la lecture sur chaque miniature : un rail discret, rempli jusqu'à la position. */
@Composable
private fun BoxScope.Progress(library: Library, v: Video) {
    if (v.duration <= 0) return
    val f = (library.position(v) / v.duration).toFloat().coerceIn(0f, 1f)
    Canvas(Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(horizontal = 8.dp, vertical = 7.dp).height(4.dp)) {
        val r = CornerRadius(size.height / 2)
        drawRoundRect(Color.Black.copy(alpha = 0.45f), cornerRadius = r)
        drawRoundRect(Color.White.copy(alpha = 0.22f), cornerRadius = r)
        if (f > 0f) drawRoundRect(Ink.accent, size = Size(maxOf(size.height, size.width * f), size.height), cornerRadius = r)
    }
}

private fun Video.title() = name.substringBeforeLast('.').replace('.', ' ').replace('_', ' ')

@Composable
private fun GridTile(library: Library, v: Video, onClick: () -> Unit) {
    Column(Modifier.clip(Tile).clickable(onClick = onClick)) {
        Thumb(library, v, Modifier.fillMaxWidth().aspectRatio(16f / 9f))
        BasicText(
            v.title(), maxLines = 2, overflow = TextOverflow.Ellipsis, style = text(13, FontWeight.SemiBold),
            modifier = Modifier.padding(start = 2.dp, end = 2.dp, top = 7.dp),
        )
    }
}

@Composable
private fun ListRow(library: Library, v: Video, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clip(Tile).clickable(onClick = onClick).padding(horizontal = 6.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Thumb(library, v, Modifier.width(112.dp).aspectRatio(16f / 9f))
        Column(Modifier.weight(1f).padding(start = 14.dp)) {
            BasicText(v.title(), maxLines = 2, overflow = TextOverflow.Ellipsis, style = text(15, FontWeight.SemiBold))
            val pos = library.position(v)
            val detail = when {
                v.duration <= 0 -> ""
                pos > 0 -> "${t("Reste", "Left")} ${clock(v.duration - pos)}"
                else -> clock(v.duration)
            }
            if (detail.isNotEmpty()) BasicText(detail, style = text(12, color = Ink.secondary), modifier = Modifier.padding(top = 3.dp))
        }
    }
}

// --- Dossiers surveillés -------------------------------------------------------------------------

@Composable
private fun FoldersSheet(library: Library, onAdd: () -> Unit, onDismiss: () -> Unit) {
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.35f)).clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss)) {
        Column(
            Modifier.align(Alignment.Center).padding(16.dp).widthIn(max = 520.dp).fillMaxWidth()
                .glass(RoundedCornerShape(28.dp), tint = 0.7f)
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .padding(horizontal = 18.dp, vertical = 16.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                BasicText(t("Dossiers surveillés", "Watched folders"), style = text(17, FontWeight.SemiBold), modifier = Modifier.weight(1f))
                GlassButton(onDismiss, plain = true, size = 36.dp) { Icon(Icons.close, size = 18.dp, tint = Ink.secondary) }
            }
            BasicText(
                t("Leurs vidéos, sous-dossiers compris, s'affichent sur l'accueil.", "Their videos, subfolders included, are shown on the home screen."),
                style = text(12, color = Ink.secondary), modifier = Modifier.padding(top = 2.dp, bottom = 10.dp),
            )
            LazyColumn(Modifier.heightIn(max = 320.dp)) {
                items(library.folders, key = { it.uri }) { f -> FolderRow(f, library.videos.count { it.folder == f.uri }) { library.removeFolder(f) } }
            }
            Spacer(Modifier.height(10.dp))
            GlassButton(onAdd, shape = Capsule, size = 46.dp, modifier = Modifier.align(Alignment.CenterHorizontally)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.add, size = 19.dp)
                    BasicText(t("Ajouter un dossier", "Add a folder"), style = text(14, FontWeight.SemiBold), modifier = Modifier.padding(start = 8.dp, end = 4.dp))
                }
            }
        }
    }
}

@Composable
private fun FolderRow(f: Folder, count: Int, onRemove: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(Icons.folder, size = 20.dp, tint = Ink.accent)
        Column(Modifier.weight(1f).padding(start = 12.dp)) {
            BasicText(f.name, maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(15, FontWeight.SemiBold))
            BasicText("${f.place} · $count ${t("vidéos", "videos")}", maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(12, color = Ink.secondary))
        }
        GlassButton(onRemove, plain = true, size = 38.dp) { Icon(Icons.close, size = 16.dp, tint = Ink.faint) }
    }
}
