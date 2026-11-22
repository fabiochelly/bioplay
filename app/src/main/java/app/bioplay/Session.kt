package app.bioplay

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors
import kotlin.math.abs

class Recent(val uri: String, val name: String, val position: Double, val duration: Double, val key: String)

/** Un film ouvert : mpv, l'index de ses sous-titres, le zoom et les messages à l'écran. */
class Session(private val context: Context) {
    val mpv = Mpv(context)
    private val prefs = context.getSharedPreferences("bioplay", Context.MODE_PRIVATE)
    // Indexation des sous-titres en arrière-plan : ne dispute pas le disque ni le CPU au démarrage.
    private val worker = Executors.newSingleThreadExecutor { r ->
        Thread({ android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_BACKGROUND); r.run() }, "subindex")
    }

    /** Ouverture des fichiers réseau (connexion), à part de l'indexation. */
    private val opener = Executors.newSingleThreadExecutor { r -> Thread(r, "open") }

    var uri by mutableStateOf<Uri?>(null); private set
    var title by mutableStateOf(""); private set
    /** Identité du fichier pour la reprise : nom et taille, stables quel que soit le lien. */
    private var key = ""
    /** Pistes intégrées, par numéro de piste du conteneur (le `src-id` de mpv). */
    var embedded by mutableStateOf<Map<Long, SubIndex>>(emptyMap()); private set
    /** Fichiers ajoutés, par nom donné à mpv. */
    val external = mutableStateMapOf<String, SubIndex>()
    var indexing by mutableStateOf(false); private set

    /** Partie utile de l'image (bandes noires incrustées exclues), mesurée une fois par fichier. */
    var content by mutableStateOf(Content.FULL); private set

    init {
        mpv.onLoaded = { pickTracks(); measurePicture() }
    }

    var zoom by mutableDoubleStateOf(1.0); private set
    var panX by mutableDoubleStateOf(0.0); private set
    var panY by mutableDoubleStateOf(0.0); private set
    var fill by mutableStateOf(false); private set
    var unscaled by mutableStateOf(false); private set

    var message by mutableStateOf(""); private set
    var messageAt by mutableLongStateOf(0L); private set

    private var generation = 0

    fun say(text: String) {
        message = text
        messageAt = System.nanoTime()
    }

    fun open(target: Uri) {
        save()
        if (Smb.isSmb(target)) {
            // Partage réseau : connexion et ouverture hors du fil principal.
            uri = target
            title = Smb.label(target)
            say(t("Connexion…", "Connecting…"))
            val gen = ++generation
            opener.execute {
                val size = Smb.size(context, target)
                val fd = Smb.open(context, target)?.detachFd()
                mpv.post {
                    if (gen == generation) start(target, title, size, fd?.let { "fdclose://$it" })
                    else fd?.let { android.os.ParcelFileDescriptor.adoptFd(it).close() }
                }
            }
            return
        }
        start(target, displayName(target), fileSize(target), mpvUrl(target))
    }

    private fun start(target: Uri, name: String, size: Long, url: String?) {
        uri = target
        title = name
        key = "$title|$size"
        embedded = emptyMap()
        keyframes = DoubleArray(0)
        aim = Double.NaN
        external.clear()
        resetZoom()
        content = Content.decode(prefs.getString("crop3:$key", null)) ?: Content.FULL
        if (url == null) return say(t("Fichier illisible", "Unreadable file"))
        val resume = prefs.getFloat("pos:$key", 0f).toDouble()
        mpv.load(url, resume)
        if (resume > 5) say("${t("Reprise à", "Resuming at")} ${clock(resume)}")
        remember(target, title)
        val gen = ++generation
        indexing = true
        worker.execute {
            // Images clés d'abord (index du fichier, quelques Ko) : sauts rapides tout de suite.
            val keys = runCatching { readFd(target)?.let { Core.keyframes(it) } }.getOrNull() ?: DoubleArray(0)
            mpv.post { if (gen == generation) keyframes = keys }
            val found = runCatching { readFd(target)?.let { SubIndex.parse(Core.index(it)) } }.getOrNull().orEmpty()
            mpv.post {
                if (gen == generation) {
                    embedded = found.associateBy { it.number }
                    indexing = false
                }
            }
        }
    }

    /** Descripteur de lecture (détaché : celui qui le reçoit le ferme). */
    private fun readFd(u: Uri): Int? =
        if (Smb.isSmb(u)) Smb.open(context, u)?.detachFd()
        else context.contentResolver.openFileDescriptor(u, "r")?.detachFd()

    fun addSubtitle(target: Uri) {
        val url = mpvUrl(target) ?: return say(t("Sous-titre illisible", "Unreadable subtitle"))
        val name = displayName(target)
        mpv.addSub(url, name.substringBeforeLast('.'))
        say("${t("Sous-titres", "Subtitles")} : $name")
        worker.execute {
            val found = runCatching { readFd(target)?.let { SubIndex.parse(Core.index(it)) } }.getOrNull()?.firstOrNull()
            if (found != null) mpv.post { external[url] = found }
        }
    }

    // --- Pistes par défaut ----------------------------------------------------------------

    /** Choix mémorisé pour ce fichier, sinon la VO et les premiers sous-titres complets. */
    private fun pickTracks() {
        val tracks = mpv.currentTracks()
        val a = prefs.getInt("aid:$key", 0)
        val s = prefs.getInt("sid:$key", 0)
        if (a > 0) mpv.setAudio(a) else Picks.audio(tracks)?.let { mpv.setAudio(it.id) }
        when {
            s > 0 -> mpv.setSub(s)
            s < 0 -> mpv.setSub(null)
            else -> mpv.setSub(Picks.sub(tracks)?.id)
        }
    }

    fun chooseAudio(t: Track) {
        mpv.setAudio(t.id)
        prefs.edit().putInt("aid:$key", t.id).apply()
        say("${t("Audio", "Audio")} : ${t.label}")
    }

    fun chooseSub(t: Track?) {
        mpv.setSub(t?.id)
        prefs.edit().putInt("sid:$key", t?.id ?: -1).apply()
        say(if (t == null) t("Sous-titres désactivés", "Subtitles off") else "${t("Sous-titres", "Subtitles")} : ${t.label}")
    }

    /** Bandes noires incrustées : mesurées une fois par fichier, en arrière-plan. */
    private fun measurePicture() {
        if (prefs.contains("crop3:$key")) return
        val u = uri ?: return
        val k = key
        val dur = mpv.duration.takeIf { it > 0 } ?: (mpv.currentDuration())
        if (dur <= 0) return
        worker.execute {
            val c = Picture.measure(context, u, dur) ?: return@execute
            prefs.edit().putString("crop3:$k", c.encode()).apply()
            mpv.post { if (key == k) content = c }
        }
    }

    // --- Sous-titres ---------------------------------------------------------------------

    val subTrack: Track?
        get() = mpv.sid?.let { sid -> mpv.tracks.firstOrNull { it.type == "sub" && it.id == sid } }

    /** Index de la piste de sous-titres affichée, s'il a pu être lu (MKV, SRT, ASS, VTT). */
    val subIndex: SubIndex?
        get() {
            val t = subTrack ?: return null
            return if (t.external != null) external[t.external] else embedded[t.srcId]
        }

    /** Position dans le temps des sous-titres (décalage compris), en ms. */
    fun subClock(seconds: Double = mpv.now()) = ((seconds - mpv.subDelay) * 1000).toLong()

    fun cueTime(c: Cue) = c.start / 1000.0 + mpv.subDelay

    /** Ligne visée par [steps] crans, sans bouger (aperçu pendant le glissé). */
    fun cueAfter(steps: Int, from: Double = mpv.now()): Cue? {
        val idx = subIndex ?: return null
        val i = idx.step(subClock(from), steps) ?: return null
        return idx.cues[i]
    }

    fun cueAt(seconds: Double): Cue? {
        val idx = subIndex ?: return null
        val c = idx.cues.getOrNull(idx.at(subClock(seconds))) ?: return null
        return c.takeIf { subClock(seconds) < it.end }
    }

    fun stepSub(steps: Int, from: Double = from()) {
        if (steps == 0) return
        val cue = cueAfter(steps, from)
        if (cue != null) seekLine(cueTime(cue)) else mpv.subSeek(steps)
    }

    // --- Sauts sur les images clés ---------------------------------------------------------

    /** Images clés de la vidéo (s, MKV) : sauts immédiats quand l'une tombe près de la cible. */
    private var keyframes = DoubleArray(0)
    /** Dernier saut : cible voulue et image atteinte (une image clé un peu avant, parfois). */
    private var aim = Double.NaN
    private var landing = Double.NaN

    /**
     * D'où partir pour un saut : la cible du saut précédent tant que la lecture n'a pas quitté
     * l'intervalle image atteinte – cible (sinon « réplique suivante » viserait la même).
     * Pendant un saut, mpv donne déjà sa cible comme position.
     */
    fun from(): Double {
        val p = mpv.now()
        val t = aim
        if (!t.isNaN() && p >= minOf(t, landing) - 0.1 && p <= maxOf(t, landing) + 0.1) return t
        return p
    }

    /**
     * Début d'une réplique : sur une image clé s'il y en a une entre 3 s avant et 0,15 s après
     * (immédiat ; arriver un peu avant ne fait rien perdre), sinon saut précis (200 à 700 ms).
     * Répliques concernées : The Rookie 86 %, Moana 84 %, One Piece 71 %.
     */
    fun seekLine(t: Double) {
        val k = keyframes
        var i = k.binarySearch(t + 0.15).let { if (it >= 0) it else -it - 2 }
        while (i >= 0 && k[i] > t + 0.15) i--
        val key = k.getOrNull(i)?.takeIf { it >= t - 3.0 }
        if (key != null) mpv.seekKeyframe(key) else mpv.seek(t)
        aim = t
        landing = key ?: t
    }

    /**
     * Saut d'une durée depuis [from] vers [t] : toujours sur une image clé (immédiat ; la
     * précision compte peu ici), la plus proche de la cible dans le bon sens. Rend l'instant atteint.
     */
    fun seekSpan(from: Double, t: Double): Double {
        val max = if (mpv.duration > 0) mpv.duration else Double.MAX_VALUE
        val goal = t.coerceIn(0.0, max)
        val dir = if (goal >= from) 1.0 else -1.0
        val k = keyframes
        if (k.isEmpty()) {
            mpv.seekRelativeKeyframe(goal - from)
            aim = goal; landing = goal
            return goal
        }
        val i = k.binarySearch(goal).let { if (it >= 0) it else -it - 1 }
        val near = (maxOf(0, i - 2) until minOf(k.size, i + 2)).map { k[it] }
        val beyond = if (dir > 0) (i until k.size).asSequence().map { k[it] }.firstOrNull { it > from + 1.0 }
        else (i - 1 downTo 0).asSequence().map { k[it] }.firstOrNull { it < from - 1.0 }
        val key = (near + listOfNotNull(beyond)).filter { (it - from) * dir > 1.0 }.minByOrNull { abs(it - goal) }
        return if (key != null) {
            mpv.seekKeyframe(key)
            aim = key; landing = key
            key
        } else {
            mpv.seek(goal, exact = false)
            aim = goal; landing = goal
            goal
        }
    }

    // --- Zoom ----------------------------------------------------------------------------

    /** Zoom depuis le centre : la surface vidéo est agrandie (mode direct compris). */
    fun zoomBy(factor: Double) {
        zoom = (zoom * factor).coerceIn(1.0, 8.0)
    }

    fun setZoomLevel(z: Double) = zoomBy(z / zoom)

    fun resetZoom() {
        zoom = 1.0; panX = 0.0; panY = 0.0; fill = false; unscaled = false
    }

    fun toggleFill() {
        fill = !fill
        say(if (fill) t("Remplir l'écran", "Fill screen") else t("Image entière", "Fit"))
    }


    // --- Mémoire -------------------------------------------------------------------------

    fun save() {
        val u = uri ?: return
        val pos = mpv.now()
        val dur = mpv.duration
        val done = dur > 0 && (dur - pos < 20 || pos / dur > 0.97)
        prefs.edit().apply {
            if (done || pos < 5) remove("pos:$key") else putFloat("pos:$key", pos.toFloat())
        }.apply()
        update(u.toString()) { it.put("pos", if (done) 0.0 else pos).put("dur", dur) }
    }

    fun recents(): List<Recent> = runCatching {
        val a = JSONArray(prefs.getString("recents", "[]"))
        List(a.length()) { a.getJSONObject(it) }.map {
            Recent(it.getString("uri"), it.optString("name"), it.optDouble("pos", 0.0), it.optDouble("dur", 0.0), it.optString("key"))
        }
    }.getOrElse { emptyList() }

    private fun remember(u: Uri, name: String) {
        val list = recents().filter { it.uri != u.toString() }
        val a = JSONArray()
        val old = list.firstOrNull { it.key == key }
        a.put(JSONObject().put("uri", u.toString()).put("name", name).put("key", key)
            .put("pos", prefs.getFloat("pos:$key", 0f).toDouble()).put("dur", old?.duration ?: 0.0))
        list.filter { it.key != key }.take(7).forEach { it.json(a) }
        prefs.edit().putString("recents", a.toString()).apply()
    }

    private fun update(u: String, f: (JSONObject) -> Unit) {
        val a = runCatching { JSONArray(prefs.getString("recents", "[]")) }.getOrElse { return }
        for (i in 0 until a.length()) if (a.getJSONObject(i).optString("uri") == u) f(a.getJSONObject(i))
        prefs.edit().putString("recents", a.toString()).apply()
    }

    fun forget(u: String) {
        val a = JSONArray()
        val gone = recents().filter { it.uri == u }
        recents().filter { it.uri != u }.forEach { it.json(a) }
        prefs.edit().putString("recents", a.toString()).apply {
            gone.forEach { remove("pos:${it.key}") }
        }.apply()
    }

    // --- Fichiers ------------------------------------------------------------------------

    /** `content://` : descripteur ouvert ici et confié à mpv (`fdclose://`), qui le fermera. */
    private fun mpvUrl(u: Uri): String? = when (u.scheme) {
        "content" -> runCatching { context.contentResolver.openFileDescriptor(u, "r")?.detachFd() }
            .getOrNull()?.let { "fdclose://$it" }
        "file" -> u.path
        else -> u.toString()
    }

    private fun Recent.json(a: JSONArray) {
        a.put(JSONObject().put("uri", uri).put("name", name).put("pos", position).put("dur", duration).put("key", key))
    }

    private fun fileSize(u: Uri): Long = runCatching {
        if (u.scheme == "file") java.io.File(u.path!!).length()
        else context.contentResolver.query(u, arrayOf(OpenableColumns.SIZE), null, null, null)?.use {
            if (it.moveToFirst()) it.getLong(0) else -1L
        } ?: -1L
    }.getOrDefault(-1L)

    private fun displayName(u: Uri): String = runCatching {
        context.contentResolver.query(u, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }
    }.getOrNull() ?: u.lastPathSegment?.substringAfterLast('/') ?: t("Vidéo", "Video")

    fun close() {
        save()
        worker.shutdownNow()
        opener.shutdownNow()
        mpv.destroy()
    }
}

fun clock(seconds: Double): String {
    val s = abs(seconds).toLong()
    val sign = if (seconds < 0) "−" else ""
    return if (s >= 3600) "%s%d:%02d:%02d".format(sign, s / 3600, s / 60 % 60, s % 60)
    else "%s%d:%02d".format(sign, s / 60, s % 60)
}
