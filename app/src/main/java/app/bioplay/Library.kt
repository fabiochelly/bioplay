package app.bioplay

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.DocumentsContract
import android.util.LruCache
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.Executors

/** Une vidéo d'un dossier surveillé. `key` est la clé de reprise de [Session] (nom + taille). */
data class Video(
    val uri: String, val name: String, val size: Long, val modified: Long, val folder: String,
    val duration: Double = 0.0,
    /** Sous-dossier, relatif au dossier surveillé (« Série/Saison 1 », vide à la racine). */
    val dir: String = "",
) {
    val key get() = "$name|$size"
    /** Nom du fichier de miniature : change si le fichier est remplacé. */
    val thumb: String get() = hash("$name|$size|$modified")
}

/** Endroit de la bibliothèque : un dossier surveillé et un sous-dossier ; racine si `root` est nul. */
data class Place(val root: String? = null, val path: String = "")

/** Un dossier à l'écran : son nom, où il mène, ce qu'il contient (vidéos comprises en dessous). */
class Dir(val name: String, val place: Place, val videos: Int, val dirs: Int)

/** Contenu d'un endroit : ses dossiers (jamais vides), puis ses vidéos, par ordre naturel. */
class Shelf(val place: Place, val title: String?, val dirs: List<Dir>, val videos: List<Video>)

/** Ordre « naturel » : « Épisode 2 » avant « Épisode 10 ». */
private val chunks = Regex("\\d+|\\D+")
val natural = Comparator<String> { a, b ->
    val re = chunks
    val x = re.findAll(a.lowercase()).map { it.value }.toList()
    val y = re.findAll(b.lowercase()).map { it.value }.toList()
    for (i in 0 until minOf(x.size, y.size)) {
        val (p, q) = x[i] to y[i]
        val c = if (p[0].isDigit() && q[0].isDigit()) (p.toBigInteger().compareTo(q.toBigInteger())).takeIf { it != 0 } ?: p.length.compareTo(q.length)
        else p.compareTo(q)
        if (c != 0) return@Comparator c
    }
    x.size.compareTo(y.size)
}

/** Un dossier surveillé (arbre SAF dont l'accès est conservé). */
class Folder(val uri: String) {
    val network = Smb.isSmb(uri)
    private val parsed = Uri.parse(uri)
    private val id = if (network) "" else runCatching { DocumentsContract.getTreeDocumentId(parsed) }.getOrDefault("")
    val path: String = if (network) parsed.pathSegments.joinToString("/") else id.substringAfter(':', "")
    val name: String get() = path.substringAfterLast('/').ifEmpty {
        when {
            network -> parsed.encodedAuthority.orEmpty()
            id.startsWith("primary") -> t("Stockage interne", "Internal storage")
            else -> id.substringBefore(':')
        }
    }
    val place: String get() = when {
        network -> "${t("Réseau", "Network")} · ${parsed.encodedAuthority}/$path"
        else -> (if (id.startsWith("primary")) t("Stockage interne", "Internal storage") else t("Carte SD", "SD card")) +
            if (path.isEmpty()) "" else "/" + path
    }
}

/**
 * Bibliothèque : les vidéos des dossiers surveillés et leurs miniatures.
 *
 * La liste est gardée dans `library.json` (affichée tout de suite au lancement), puis les
 * dossiers sont relus en arrière-plan et la vue mise à jour seulement si quelque chose a changé.
 * Les miniatures (WebP, 320 px) vont dans `no_backup/thumbs`, nommées d'après le fichier :
 * créées une fois, gardées tant que la vidéo existe, effacées quand elle disparaît.
 * Rien ne tourne pendant la lecture : tout le téléphone reste au film.
 */
class Library(private val context: Context) {
    private val prefs = context.getSharedPreferences("bioplay", Context.MODE_PRIVATE)
    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor { r ->
        Thread({ android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_BACKGROUND); r.run() }, "library")
    }
    private val listFile = File(context.filesDir, "library.json")
    private val thumbDir = File(context.noBackupFilesDir, "thumbs").apply { mkdirs() }

    var folders by mutableStateOf(loadFolders()); private set
    var videos by mutableStateOf(emptyList<Video>()); private set
    var scanning by mutableStateOf(false); private set
    /** Augmente à chaque miniature créée : les vignettes concernées se rechargent. */
    var thumbsMade by mutableIntStateOf(0); private set
    var grid by mutableStateOf(prefs.getBoolean("grid", true)); private set

    /** Vrai pendant la lecture : la relecture et les miniatures attendent. */
    @Volatile var hold = false

    /** Miniatures décodées, gardées en mémoire dans la limite de 12 Mo. */
    private val memory = object : LruCache<String, Bitmap>(12 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.allocationByteCount
    }

    /** Ce qu'on voit à cet endroit. Un seul dossier surveillé : on est directement dedans. */
    fun shelf(place: Place): Shelf {
        val roots = folders.filter { f -> videos.any { it.folder == f.uri } }
        val root = place.root ?: roots.singleOrNull()?.uri ?: folders.singleOrNull()?.uri
        if (root == null) {
            val dirs = roots.map { f ->
                val mine = videos.filter { it.folder == f.uri }
                Dir(f.name, Place(f.uri, ""), mine.size, mine.map { it.dir.substringBefore('/') }.filter { it.isNotEmpty() }.distinct().size)
            }.sortedWith(compareBy(natural) { it.name })
            return Shelf(Place(), null, dirs, emptyList())
        }
        val path = place.path
        val inside = videos.filter { it.folder == root && (path.isEmpty() || it.dir == path || it.dir.startsWith("$path/")) }
        val here = inside.filter { it.dir == path }.sortedWith(compareBy(natural) { it.name })
        val dirs = inside.filter { it.dir != path }
            .groupBy { (if (path.isEmpty()) it.dir else it.dir.removePrefix("$path/")).substringBefore('/') }
            .map { (name, vs) ->
                val sub = if (path.isEmpty()) name else "$path/$name"
                Dir(name, Place(root, sub), vs.size, vs.filter { it.dir != sub }.map { it.dir.removePrefix("$sub/").substringBefore('/') }.distinct().size)
            }
            .sortedWith(compareBy(natural) { it.name })
        val title = if (path.isEmpty()) folders.firstOrNull { it.uri == root }?.name else path.substringAfterLast('/')
        return Shelf(Place(root, path), title, dirs, here)
    }

    /** L'endroit au-dessus (null si on est déjà en haut). */
    fun up(place: Place): Place? {
        val root = place.root ?: return null
        if (place.path.isNotEmpty()) return Place(root, place.path.substringBeforeLast('/', ""))
        return if (folders.count { f -> videos.any { it.folder == f.uri } } > 1) Place() else null
    }

    fun toggleView() {
        grid = !grid
        prefs.edit().putBoolean("grid", grid).apply()
    }

    // --- Dossiers ---------------------------------------------------------------------------

    private fun loadFolders(): List<Folder> = runCatching {
        val a = JSONArray(prefs.getString("folders", "[]"))
        List(a.length()) { Folder(a.getString(it)) }
    }.getOrElse { emptyList() }

    private fun saveFolders() {
        prefs.edit().putString("folders", JSONArray(folders.map { it.uri }).toString()).apply()
    }

    fun addFolder(tree: Uri) {
        if (!Smb.isSmb(tree)) runCatching { context.contentResolver.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION) }
        if (folders.any { it.uri == tree.toString() }) return
        folders = folders + Folder(tree.toString())
        saveFolders()
        refresh()
    }

    fun removeFolder(f: Folder) {
        folders = folders.filter { it.uri != f.uri }
        saveFolders()
        if (!f.network) runCatching { context.contentResolver.releasePersistableUriPermission(Uri.parse(f.uri), Intent.FLAG_GRANT_READ_URI_PERMISSION) }
        videos = videos.filter { it.folder != f.uri }
        refresh()
    }

    // --- Relecture ----------------------------------------------------------------------------

    /** Liste gardée, puis relecture des dossiers en arrière-plan. */
    fun start() {
        worker.execute {
            val cached = readList()
            main.post { if (videos.isEmpty()) videos = cached }
        }
        refresh()
    }

    fun refresh() {
        val roots = folders.map { it.uri }
        main.post { scanning = true }
        worker.execute {
            waitWhileHeld()
            val old = readList().associateBy { it.uri }
            val found = mutableListOf<Video>()
            var complete = true
            for (root in roots) {
                val list = runCatching { if (Smb.isSmb(root)) scanSmb(Uri.parse(root)) else scan(Uri.parse(root)) }.getOrNull()
                if (list == null) {
                    // Dossier momentanément illisible (carte retirée) : on garde ce qu'on en savait.
                    complete = false
                    found += old.values.filter { it.folder == root }
                } else {
                    found += list.map { v -> old[v.uri]?.takeIf { it.thumb == v.thumb }?.let { v.copy(duration = it.duration) } ?: v }
                }
            }
            val sorted = found.distinctBy { it.uri }.sortedByDescending { it.modified }
            if (sorted != old.values.sortedByDescending { it.modified }) writeList(sorted)
            main.post { videos = sorted; scanning = false }
            if (complete) prune(sorted)
            makeThumbs(sorted)
        }
    }

    /** Vidéos d'un arbre, sous-dossiers compris (dossiers cachés exclus). */
    private fun scan(tree: Uri): List<Video> {
        val out = mutableListOf<Video>()
        val cols = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE, DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )
        val pending = ArrayDeque<Pair<String, String>>()
        pending += DocumentsContract.getTreeDocumentId(tree) to ""
        var first = true
        while (pending.isNotEmpty()) {
            val (doc, rel) = pending.removeFirst()
            val depth = if (rel.isEmpty()) 0 else rel.count { it == '/' } + 1
            val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, doc)
            val cursor = context.contentResolver.query(children, cols, null, null, null)
            if (cursor == null) {
                if (first) error("inaccessible")
                continue
            }
            first = false
            cursor.use { c ->
                while (c.moveToNext()) {
                    val id = c.getString(0) ?: continue
                    val name = c.getString(1) ?: continue
                    if (name.startsWith('.')) continue
                    val mime = c.getString(2).orEmpty()
                    if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
                        if (depth < 8) pending += id to (if (rel.isEmpty()) name else "$rel/$name")
                    } else if (isVideo(name, mime)) {
                        out += Video(DocumentsContract.buildDocumentUriUsingTree(tree, id).toString(), name, c.getLong(3), c.getLong(4), tree.toString(), dir = rel)
                    }
                }
            }
        }
        return out
    }

    /** Vidéos d'un dossier réseau, sous-dossiers compris (dossiers cachés exclus). */
    private fun scanSmb(root: Uri): List<Video> {
        val out = mutableListOf<Video>()
        val pending = ArrayDeque<Triple<Uri, Int, String>>()
        pending += Triple(root, 0, "")
        var first = true
        while (pending.isNotEmpty()) {
            val (dir, depth, rel) = pending.removeFirst()
            val entries = if (first) Smb.list(context, dir) else runCatching { Smb.list(context, dir) }.getOrNull() ?: continue
            first = false
            for (e in entries) {
                if (e.name.startsWith('.')) continue
                val u = dir.buildUpon().appendPath(e.name).build()
                if (e.dir) {
                    if (depth < 8) pending += Triple(u, depth + 1, if (rel.isEmpty()) e.name else "$rel/${e.name}")
                } else if (isVideo(e.name, "")) {
                    out += Video(u.toString(), e.name, e.size, e.modified, root.toString(), dir = rel)
                }
            }
        }
        return out
    }

    private fun isVideo(name: String, mime: String) =
        mime.startsWith("video/") || mime == "application/x-matroska" ||
            name.substringAfterLast('.', "").lowercase() in VIDEO_EXT

    // --- Miniatures -----------------------------------------------------------------------------

    fun thumbFile(v: Video) = File(thumbDir, "${v.thumb}.webp")

    /** Miniature décodée (à appeler hors du fil principal), ou null si pas encore faite. */
    fun loadThumb(v: Video): Bitmap? {
        memory.get(v.thumb)?.let { return it }
        val f = thumbFile(v)
        if (!f.exists()) return null
        return BitmapFactory.decodeFile(f.path)?.also { memory.put(v.thumb, it) }
    }

    private fun prune(list: List<Video>) {
        val keep = list.mapTo(HashSet()) { "${it.thumb}.webp" }
        thumbDir.listFiles()?.forEach { if (it.name !in keep) it.delete() }
    }

    private fun makeThumbs(list: List<Video>) {
        val todo = list.filter { !thumbFile(it).exists() }
        if (todo.isEmpty()) return
        val durations = HashMap<String, Double>()
        for (v in todo) {
            waitWhileHeld()
            val (bmp, dur) = runCatching { frame(Uri.parse(v.uri)) }.getOrNull() ?: (null to 0.0)
            if (dur > 0) durations[v.uri] = dur
            if (bmp != null) {
                val tmp = File(thumbDir, "${v.thumb}.tmp")
                tmp.outputStream().use { bmp.compress(Bitmap.CompressFormat.WEBP_LOSSY, 78, it) }
                tmp.renameTo(thumbFile(v))
                bmp.recycle()
            } else {
                // Illisible : une image vide, pour ne pas réessayer à chaque lancement.
                thumbFile(v).writeBytes(ByteArray(0))
            }
            main.post { thumbsMade++ }
        }
        if (durations.isNotEmpty()) {
            val updated = list.map { v -> durations[v.uri]?.let { v.copy(duration = it) } ?: v }
            writeList(updated)
            main.post { if (videos.map { it.uri } == updated.map { it.uri }) videos = updated }
        }
    }

    /** Une image représentative : vers 15 % du film, plus loin si elle est presque noire. */
    private fun frame(uri: Uri): Pair<Bitmap?, Double> {
        val r = MediaMetadataRetriever()
        var pfd: android.os.ParcelFileDescriptor? = null
        try {
            pfd = Picture.setSource(r, context, uri)
            val dur = (r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull() ?: 0L) / 1000.0
            var best: Bitmap? = null
            for (f in listOf(0.15, 0.32, 0.5)) {
                val b = r.getScaledFrameAtTime((dur * f * 1e6).toLong(), MediaMetadataRetriever.OPTION_CLOSEST_SYNC, THUMB_W, THUMB_H) ?: continue
                best?.recycle()
                best = b
                if (luma(b) > 28) break
            }
            return best?.let(::trimBars) to dur
        } finally {
            r.release()
            runCatching { pfd?.close() }
        }
    }

    private fun luma(b: Bitmap): Int {
        var sum = 0L; var n = 0
        for (y in 0 until b.height step 8) for (x in 0 until b.width step 8) {
            val p = b.getPixel(x, y)
            sum += ((p shr 16 and 0xFF) * 54 + (p shr 8 and 0xFF) * 183 + (p and 0xFF) * 19) shr 8
            n++
        }
        return if (n == 0) 0 else (sum / n).toInt()
    }

    /** Ôte les bandes noires en haut et en bas : la vignette montre l'image, pas le cadre. */
    private fun trimBars(b: Bitmap): Bitmap {
        fun dark(y: Int): Boolean {
            for (x in 0 until b.width step 4) {
                val p = b.getPixel(x, y)
                if (((p shr 16 and 0xFF) * 54 + (p shr 8 and 0xFF) * 183 + (p and 0xFF) * 19) shr 8 > 30) return false
            }
            return true
        }
        var top = 0
        while (top < b.height / 3 && dark(top)) top++
        var bottom = b.height
        while (bottom > b.height * 2 / 3 && dark(bottom - 1)) bottom--
        if (top < 3 && b.height - bottom < 3) return b
        val out = Bitmap.createBitmap(b, 0, top, b.width, bottom - top)
        if (out !== b) b.recycle()
        return out
    }

    private fun waitWhileHeld() {
        while (hold) Thread.sleep(1000)
    }

    // --- Liste gardée -----------------------------------------------------------------------------

    private fun readList(): List<Video> = runCatching {
        val a = JSONArray(listFile.readText())
        List(a.length()) { a.getJSONObject(it) }.map {
            Video(it.getString("u"), it.getString("n"), it.getLong("s"), it.getLong("m"), it.getString("f"), it.optDouble("d", 0.0), it.optString("r"))
        }
    }.getOrElse { emptyList() }

    private fun writeList(list: List<Video>) {
        val a = JSONArray()
        list.forEach { a.put(JSONObject().put("u", it.uri).put("n", it.name).put("s", it.size).put("m", it.modified).put("f", it.folder).put("d", it.duration).put("r", it.dir)) }
        val tmp = File(listFile.path + ".tmp")
        tmp.writeText(a.toString())
        tmp.renameTo(listFile)
    }

    /** Position mémorisée (secondes), pour la barre de progression des vignettes. */
    fun position(v: Video): Double = prefs.getFloat("pos:${v.key}", 0f).toDouble()

    fun close() = worker.shutdownNow()

    companion object {
        const val THUMB_W = 320
        const val THUMB_H = 180
        val VIDEO_EXT = setOf("mkv", "mk3d", "webm", "mp4", "m4v", "mov", "avi", "m2ts", "mts", "ts", "wmv", "flv", "ogv", "3gp")
    }
}

private fun hash(s: String): String =
    MessageDigest.getInstance("SHA-1").digest(s.toByteArray()).take(10).joinToString("") { "%02x".format(it) }
