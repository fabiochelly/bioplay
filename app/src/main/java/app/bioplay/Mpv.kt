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
) {
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

    var position by mutableDoubleStateOf(0.0); private set
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

    var onLoaded: (() -> Unit)? = null

    init {
        val fonts = File(context.filesDir, "fonts").apply { mkdirs() }
        File(fonts, "inter-semibold.ttf").takeUnless { it.exists() }?.let { f ->
            context.assets.open("inter-semibold.ttf").use { i -> f.outputStream().use { i.copyTo(it) } }
        }
        listOf(
            "config" to "no",
            "vo" to VO,
            "gpu-context" to "android",
            "opengl-es" to "yes",
            // Décodage matériel, recopié vers le rendu de mpv : zoom, sous-titres et rendu
            // de qualité restent possibles. Repli automatique sur FFmpeg sinon.
            "hwdec" to "mediacodec-copy",
            "hwdec-codecs" to "h264,hevc,mpeg4,mpeg2video,vp8,vp9,av1",
            "ao" to "audiotrack,opensles",
            "audio-pitch-correction" to "yes",
            "input-default-bindings" to "no",
            "force-window" to "no",
            "idle" to "yes",
            "keep-open" to "yes",
            // Cache qui garde aussi l'arrière : retour en arrière instantané.
            "cache" to "yes",
            "demuxer-max-bytes" to "64MiB",
            "demuxer-max-back-bytes" to "48MiB",
            "demuxer-seekable-cache" to "yes",
            "hr-seek" to "yes",
            "hr-seek-framedrop" to "yes",
            "sub-fonts-dir" to fonts.path,
            "sub-font" to "Inter 18pt SemiBold",
            // Liseré noir fin, sans ombre ; appliqué aussi aux ASS (tailles parfois démesurées).
            "sub-font-size" to "34",
            "sub-color" to "#FFFFFF",
            "sub-outline-size" to "1.3",
            "sub-outline-color" to "#000000",
            "sub-shadow-offset" to "0",
            "sub-blur" to "0",
            "sub-ass-override" to "force",
            "sub-margin-y" to "36",
            "sub-auto" to "no",
            "osd-level" to "0",
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
            "time-pos" to MpvFormat.MPV_FORMAT_DOUBLE,
            "duration" to MpvFormat.MPV_FORMAT_DOUBLE,
            "demuxer-cache-time" to MpvFormat.MPV_FORMAT_DOUBLE,
            "pause" to MpvFormat.MPV_FORMAT_FLAG,
            "speed" to MpvFormat.MPV_FORMAT_DOUBLE,
            "sub-delay" to MpvFormat.MPV_FORMAT_DOUBLE,
            "audio-delay" to MpvFormat.MPV_FORMAT_DOUBLE,
            "track-list" to MpvFormat.MPV_FORMAT_STRING,
            "aid" to MpvFormat.MPV_FORMAT_STRING,
            "sid" to MpvFormat.MPV_FORMAT_STRING,
            "container-fps" to MpvFormat.MPV_FORMAT_DOUBLE,
            "eof-reached" to MpvFormat.MPV_FORMAT_FLAG,
            "hwdec-current" to MpvFormat.MPV_FORMAT_STRING,
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
        if (p != null) p() else lib.setPropertyString("vo", VO)
    }

    fun resize(w: Int, h: Int) {
        if (!alive) return
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
        lib.setPropertyString("vo", VO)
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

    /** Zoom libre : [scale] 1 = image entière ; décalage en fraction de l'image. */
    fun setZoom(scale: Double, panX: Double, panY: Double) {
        if (!alive) return
        lib.setPropertyDouble("video-zoom", kotlin.math.log2(scale))
        lib.setPropertyDouble("video-pan-x", panX)
        lib.setPropertyDouble("video-pan-y", panY)
    }

    fun setPanscan(fill: Boolean) {
        if (!alive) return
        lib.setPropertyDouble("panscan", if (fill) 1.0 else 0.0)
    }

    fun setUnscaled(on: Boolean) {
        if (!alive) return
        lib.setPropertyString("video-unscaled", if (on) "yes" else "no")
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
            "time-pos" -> position = value
            "duration" -> duration = value
            "demuxer-cache-time" -> cached = value
            "speed" -> speed = value
            "sub-delay" -> subDelay = value
            "audio-delay" -> audioDelay = value
            "container-fps" -> fps = value
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
        }
    }

    override fun event(eventId: Int) {
        when (eventId) {
            MPVLib.MpvEvent.MPV_EVENT_FILE_LOADED -> post {
                loaded = true
                lib.setPropertyString("start", "none")
                onLoaded?.invoke()
            }
        }
    }

    override fun eventEndFile(reason: Int, error: Int) {}

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
            )
        }
    }.getOrElse { emptyList() }

    companion object {
        const val VO = "gpu-next"
    }
}
