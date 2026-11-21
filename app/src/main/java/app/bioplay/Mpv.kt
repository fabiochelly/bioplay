package app.bioplay

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Surface
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.jdtech.mpv.MPVLib
import dev.jdtech.mpv.MPVLib.MpvFormat
import org.json.JSONArray
import java.io.File
import java.util.Locale

class Track(
    val id: Int,
    val type: String,
    val srcId: Long,
    val title: String,
    val lang: String,
    val codec: String,
    val channels: Int,
    val external: String?,
    val forced: Boolean,
    val default: Boolean,
    /** Drapeau « piste originale » du conteneur (la VO). */
    val original: Boolean = false,
) {
    /** Sous-titres en images (PGS, VobSub) : non affichés en mode direct. */
    val image get() = codec in setOf("hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle", "pgs", "vobsub")
    /** « Français », « English (ASS) », sinon la langue lisible, sinon « Piste 3 ». */
    val label: String
        get() = title.ifBlank { languageName(lang) ?: "${t("Piste", "Track")} $id" }

    val detail: String
        get() = buildList {
            if (title.isNotBlank()) languageName(lang)?.let { add(it) }
            add(codecName(codec))
            if (channels > 0) add(channelName(channels))
            if (forced) add(t("Forcés", "Forced"))
            if (external != null) add(t("Externe", "External"))
        }.joinToString(" · ")
}

private val bibliographic = mapOf(
    "fre" to "fr", "ger" to "de", "dut" to "nl", "chi" to "zh", "cze" to "cs", "gre" to "el",
    "per" to "fa", "rum" to "ro", "slo" to "sk", "alb" to "sq", "arm" to "hy", "baq" to "eu",
    "bur" to "my", "geo" to "ka", "ice" to "is", "mac" to "mk", "may" to "ms", "wel" to "cy",
)

fun languageName(code: String): String? {
    if (code.isBlank() || code == "und") return null
    val tag = bibliographic[code] ?: code
    val name = Locale.forLanguageTag(tag).getDisplayName(Lang.locale)
    if (name.isBlank() || name.equals(tag, ignoreCase = true)) return code.uppercase()
    return name.replaceFirstChar { it.titlecase(Lang.locale) }
}

private fun codecName(c: String) = when (c) {
    "subrip" -> "SRT"; "ass", "ssa" -> "ASS"; "hdmv_pgs_subtitle" -> "PGS"; "dvd_subtitle" -> "VobSub"
    "webvtt" -> "WebVTT"; "mov_text" -> t("Texte", "Text"); "eac3" -> "E-AC3"; "truehd" -> "TrueHD"
    else -> c.uppercase()
}

private fun channelName(n: Int) = when (n) {
    1 -> "Mono"; 2 -> t("Stéréo", "Stereo"); 6 -> "5.1"; 8 -> "7.1"; else -> "$n ${t("canaux", "channels")}"
}

/**
 * libmpv et son état, observé par l'interface. Les événements arrivent sur le fil de mpv et
 * sont reportés sur le fil principal : l'état Compose n'est écrit que là.
 */
class Mpv(context: Context) : MPVLib.EventObserver {
    @Volatile private var alive = true

    private val lib = MPVLib.create(context) ?: error("libmpv indisponible")
    private val main = Handler(Looper.getMainLooper())

    /**
     * Position pour l'interface. Non observée (mpv la publierait à chaque image) : [refresh] la
     * lit, seulement quand les commandes sont affichées.
     */
    var position by mutableDoubleStateOf(0.0); private set
    /** Position exacte, lue à la demande (sauts de sous-titre). */
    fun now(): Double = if (alive && loaded) lib.getPropertyDouble("time-pos") ?: position else position

    /** Relit position et cache (appelé 4 fois par seconde, commandes affichées seulement). */
    fun refresh() {
        if (!alive || !loaded) return
        lib.getPropertyDouble("time-pos")?.let { position = it }
        lib.getPropertyDouble("demuxer-cache-time")?.let { cached = it }
    }
    var duration by mutableDoubleStateOf(0.0); private set
    var cached by mutableDoubleStateOf(0.0); private set
    var paused by mutableStateOf(false); private set
    var speed by mutableDoubleStateOf(1.0); private set
    var subDelay by mutableDoubleStateOf(0.0); private set
    var audioDelay by mutableDoubleStateOf(0.0); private set
    var tracks by mutableStateOf<List<Track>>(emptyList()); private set
    var aid by mutableStateOf<Int?>(null); private set
    var sid by mutableStateOf<Int?>(null); private set
    var fps by mutableDoubleStateOf(0.0); private set
    var loaded by mutableStateOf(false); private set
    var ended by mutableStateOf(false); private set
    var hwdec by mutableStateOf(""); private set
    /** Mode direct (décodeur matériel → écran) ; faux pendant un repli sur le rendu de mpv. */
    var direct by mutableStateOf(true); private set
    private val currentVo get() = if (direct) DIRECT_VO else VO
    /** Sous-titre affiché (texte), dessiné par l'app en mode direct. */
    var subText by mutableStateOf(""); private set


    var onLoaded: (() -> Unit)? = null

    init {
        val fonts = File(context.filesDir, "fonts").apply { mkdirs() }
        File(fonts, "inter-semibold.ttf").takeUnless { it.exists() }?.let { f ->
            context.assets.open("inter-semibold.ttf").use { i -> f.outputStream().use { i.copyTo(it) } }
        }

        listOf(
            "config" to "no",
            // Mode direct : les images du décodeur matériel vont droit à l'écran (comme MX
            // Player), sans copie ni rendu OpenGL. Repli sur le rendu de mpv si le téléphone ne
            // sait pas décoder un format (voir [checkDirect]).
            "vo" to DIRECT_VO,
            "hwdec" to "mediacodec",
            "hwdec-codecs" to "all",
            "gpu-context" to "android",
            "opengl-es" to "yes",
            "profile" to "fast",
            // Pas d'AAudio : avec certaines pistes (E-AC3 5.1), l'image se figeait après un
            // changement de piste audio.
            "ao" to "audiotrack,opensles",
            "audio-buffer" to "0.4",
            "audio-pitch-correction" to "yes",
            "input-default-bindings" to "no",
            "force-window" to "no",
            "idle" to "yes",
            "keep-open" to "yes",
            // Cache qui garde aussi l'arrière : retour en arrière instantané, mémoire contenue.
            "cache" to "yes",
            // Avec le cache, cache-secs (énorme par défaut) l'emporte sur readahead : on le borne
            // aussi à 10 s, sinon mpv remplit toujours tout le plafond.
            "cache-secs" to "10",
            "demuxer-readahead-secs" to "10",
            "demuxer-max-bytes" to "24MiB",
            "demuxer-max-back-bytes" to "6MiB",
            "demuxer-seekable-cache" to "yes",
            "hr-seek" to "yes",
            "hr-seek-framedrop" to "yes",
            // Sous-titres : dessinés par l'app (texte fourni par mpv) ; en repli, par mpv.
            "sub-fonts-dir" to fonts.path,
            "sub-font" to "Inter 18pt SemiBold",
            "sub-font-size" to "60",
            "sub-color" to "#EBEBEB",
            "sub-outline-size" to "1.6",
            "sub-outline-color" to "#0A0A0A",
            "sub-shadow-offset" to "0",
            "sub-ass-override" to "force",
            "sub-margin-y" to "46",
            "sub-auto" to "no",
            // Mode direct : l'app dessine les sous-titres ; mpv ne charge ni libass ni les
            // polices jointes au MKV (souvent 10 à 30 Mo). Rétablis en repli.
            "sub-ass" to "no",
            "embeddedfonts" to "no",
            // Le matériel décode l'image : un seul fil logiciel suffit (moins de mémoire).
            "vd-lavc-threads" to "1",
            "ad-lavc-threads" to "1",
            "osd-level" to "0",
            // Aucun script Lua intégré (stats, console, menus…) : ni fils ni machines Lua.
            "load-scripts" to "no",
            "osc" to "no",
            "ytdl" to "no",
            "load-stats-overlay" to "no",
            "load-console" to "no",
            "load-auto-profiles" to "no",
            "load-select" to "no",
            "load-positioning" to "no",
            "load-commands" to "no",
            "load-context-menu" to "no",
            "screenshot-directory" to context.cacheDir.path,
        ).forEach { (k, v) -> lib.setOptionString(k, v) }
        lib.init()
        lib.addObserver(this)
        if (BuildConfig.DEBUG) {
            lib.addLogObserver(object : MPVLib.LogObserver {
                override fun logMessage(prefix: String, level: Int, text: String) {
                    Log.println(if (level <= 20) Log.WARN else Log.INFO, "mpv", "[$prefix] ${text.trimEnd()}")
                }
            })
            lib.requestLogMessages("info")
        }
        mapOf(
            "duration" to MpvFormat.MPV_FORMAT_DOUBLE,
            "pause" to MpvFormat.MPV_FORMAT_FLAG,
            "speed" to MpvFormat.MPV_FORMAT_DOUBLE,
            "sub-delay" to MpvFormat.MPV_FORMAT_DOUBLE,
            "audio-delay" to MpvFormat.MPV_FORMAT_DOUBLE,
            "track-list" to MpvFormat.MPV_FORMAT_STRING,
            "aid" to MpvFormat.MPV_FORMAT_STRING,
            "sid" to MpvFormat.MPV_FORMAT_STRING,
            "container-fps" to MpvFormat.MPV_FORMAT_DOUBLE,
            "video-params/aspect" to MpvFormat.MPV_FORMAT_DOUBLE,
            "eof-reached" to MpvFormat.MPV_FORMAT_FLAG,
            "hwdec-current" to MpvFormat.MPV_FORMAT_STRING,
            "sub-text" to MpvFormat.MPV_FORMAT_STRING,
        ).forEach { (k, f) -> lib.observeProperty(k, f) }
    }

    // --- Surface -------------------------------------------------------------------------

    private var surface = false
    /** Chargement demandé avant que la surface existe : sans elle, mpv couperait la vidéo. */
    private var pending: (() -> Unit)? = null

    fun attach(s: Surface) {
        if (!alive) return
        lib.attachSurface(s)
        lib.setOptionString("force-window", "yes")
        surface = true
        val p = pending
        pending = null
        if (p != null) p() else lib.setPropertyString("vo", currentVo)
    }

    /** Taille de la surface vidéo (pixels), pour le zoom. */
    var surfaceW = 1; private set
    var surfaceH = 1; private set
    var aspect by mutableDoubleStateOf(16.0 / 9.0); private set

    fun resize(w: Int, h: Int) {
        if (!alive) return
        surfaceW = w.coerceAtLeast(1); surfaceH = h.coerceAtLeast(1)
        lib.setPropertyString("android-surface-size", "${w}x$h")
    }

    fun detach() {
        if (!alive) return
        surface = false
        lib.setPropertyString("vo", "null")
        lib.setOptionString("force-window", "no")
        lib.detachSurface()
    }

    fun destroy() {
        if (!alive) return
        lib.removeObserver(this)
        alive = false
        lib.destroy()
    }

    // --- Commandes -----------------------------------------------------------------------

    fun load(url: String, start: Double) {
        if (!alive) return
        loaded = false
        ended = false
        position = start
        if (!surface) {
            pending = { load(url, start) }
            return
        }
        // Chaque fichier repart en mode direct.
        if (!direct) {
            direct = true
            lib.setPropertyString("sub-ass", "no")
            lib.setPropertyString("embeddedfonts", "no")
            lib.setPropertyString("vd-lavc-threads", "1")
            lib.setPropertyString("hwdec", "mediacodec")
        }
        lib.setPropertyString("vo", currentVo)
        lib.setPropertyString("start", if (start > 0) "%.3f".format(Locale.ROOT, start) else "none")
        lib.command(arrayOf("loadfile", url, "replace"))
    }

    fun stop() {
        if (!alive) return
        lib.command(arrayOf("stop"))
    }

    fun togglePause() = pauseTo(!paused)

    fun pauseTo(p: Boolean) {
        if (!alive) return
        if (!p && ended) seek(0.0)
        lib.setPropertyBoolean("pause", p)
    }

    /** Saut précis (`exact`) ou rapide sur image clé, pour l'aperçu pendant le glissé. */
    fun seek(seconds: Double, exact: Boolean = true) {
        if (!alive) return
        val t = seconds.coerceIn(0.0, if (duration > 0) duration else Double.MAX_VALUE)
        lib.command(arrayOf("seek", "%.3f".format(Locale.ROOT, t), if (exact) "absolute+exact" else "absolute+keyframes"))
    }

    fun subSeek(steps: Int) {
        if (!alive) return
        lib.command(arrayOf("sub-seek", steps.toString()))
    }

    fun speedTo(s: Double) {
        if (!alive) return
        lib.setPropertyDouble("speed", s)
    }

    fun setAudio(id: Int?) {
        if (!alive) return
        lib.setPropertyString("aid", id?.toString() ?: "no")
    }

    fun setSub(id: Int?) {
        if (!alive) return
        lib.setPropertyString("sid", id?.toString() ?: "no")
    }

    fun subDelayTo(s: Double) {
        if (!alive) return
        lib.setPropertyDouble("sub-delay", s)
    }

    fun audioDelayTo(s: Double) {
        if (!alive) return
        lib.setPropertyDouble("audio-delay", s)
    }

    fun addSub(url: String, title: String) {
        if (!alive) return
        lib.command(arrayOf("sub-add", url, "select", title))
    }


    private var subMargin = 46

    /** Marge basse des sous-titres (unités mpv, image de 720 lignes). */
    fun subMarginTo(m: Int) {
        if (!alive || m == subMargin) return
        subMargin = m
        lib.setPropertyString("sub-margin-y", m.toString())
    }






    /** Exécute [r] sur le fil principal (là où l'état est écrit). */
    fun post(r: () -> Unit) {
        main.post { if (alive) r() }
    }

    // --- Événements (fil de mpv) ---------------------------------------------------------

    override fun eventProperty(property: String) {}

    override fun eventProperty(property: String, value: Long) {}

    override fun eventProperty(property: String, value: Boolean) = post {
        when (property) {
            "pause" -> paused = value
            "eof-reached" -> ended = value
        }
    }.let {}

    override fun eventProperty(property: String, value: Double) = post {
        when (property) {
            "duration" -> duration = value
            "speed" -> speed = value
            "sub-delay" -> subDelay = value
            "audio-delay" -> audioDelay = value
            "container-fps" -> fps = value
            "video-params/aspect" -> if (value > 0) aspect = value
        }
    }.let {}

    override fun eventProperty(property: String, value: String) {
        when (property) {
            "track-list" -> {
                val list = parseTracks(value)
                post { tracks = list }
            }
            "aid" -> post { aid = value.toIntOrNull() }
            "sid" -> post { sid = value.toIntOrNull() }
            "hwdec-current" -> post { hwdec = value }
            "sub-text" -> post { subText = value }
        }
    }

    override fun event(eventId: Int) {
        when (eventId) {
            MPVLib.MpvEvent.MPV_EVENT_FILE_LOADED -> post {
                loaded = true
                lib.setPropertyString("start", "none")
                onLoaded?.invoke()
                // Le décodeur matériel a-t-il pris la vidéo ? Sinon, repli pour ce fichier.
                main.postDelayed({ if (alive) checkDirect() }, 1200)
            }
        }
    }

    override fun eventEndFile(reason: Int, error: Int) {}

    /**
     * Repli : si la vidéo n'est pas décodée par le matériel (format que le téléphone ne gère
     * pas), le mode direct ne peut pas l'afficher ; on passe au rendu de mpv pour ce fichier.
     */
    private fun checkDirect() {
        if (!direct || !loaded) return
        val hasVideo = tracks.any { it.type == "video" }
        if (hasVideo && !hwdec.startsWith("mediacodec")) {
            direct = false
            lib.setPropertyString("sub-ass", "yes")
            lib.setPropertyString("embeddedfonts", "yes")
            lib.setPropertyString("vd-lavc-threads", "0")
            lib.setPropertyString("hwdec", "mediacodec-copy")
            lib.setPropertyString("vo", VO)
            lib.command(arrayOf("video-reload"))
        }
    }

    fun currentDuration(): Double = if (alive) lib.getPropertyDouble("duration") ?: 0.0 else 0.0

    /** Pistes lues à l'instant (au chargement, avant que l'état observé n'arrive). */
    fun currentTracks(): List<Track> = if (alive) parseTracks(lib.getPropertyString("track-list") ?: "[]") else emptyList()

    private fun parseTracks(json: String): List<Track> = runCatching {
        val a = JSONArray(json)
        List(a.length()) { i ->
            val o = a.getJSONObject(i)
            Track(
                id = o.optInt("id"),
                type = o.optString("type"),
                srcId = o.optLong("src-id", -1),
                title = o.optString("title"),
                lang = o.optString("lang"),
                codec = o.optString("codec"),
                channels = o.optInt("demux-channel-count", 0),
                external = if (o.optBoolean("external")) o.optString("external-filename") else null,
                forced = o.optBoolean("forced"),
                default = o.optBoolean("default"),
                original = o.optBoolean("original"),
            )
        }
    }.getOrElse { emptyList() }

    companion object {
        const val VO = "gpu-next"
        const val DIRECT_VO = "mediacodec_embed"
    }
}
