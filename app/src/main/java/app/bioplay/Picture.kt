package app.bioplay

import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.net.Uri
import kotlin.math.abs

/** Partie utile de l'image, en fractions de l'image entière (bandes noires incrustées ôtées). */
data class Content(val left: Float, val top: Float, val right: Float, val bottom: Float) {
    val full get() = left <= 0f && top <= 0f && right >= 1f && bottom >= 1f
    fun encode() = "$left,$top,$right,$bottom"

    companion object {
        val FULL = Content(0f, 0f, 1f, 1f)
        fun decode(s: String?): Content? = s?.split(',')?.mapNotNull { it.toFloatOrNull() }
            ?.takeIf { it.size == 4 }?.let { Content(it[0], it[1], it[2], it[3]) }
    }
}

/**
 * Mesure les bandes noires incrustées dans l'image : six images réparties dans le film,
 * réduites, bord par bord, la mesure la plus fréquente.
 * Une seule fois par fichier, en arrière-plan.
 */
object Picture {
    private const val W = 256
    private const val H = 144

    fun measure(context: Context, uri: Uri, durationS: Double): Content? = runCatching {
        val r = MediaMetadataRetriever()
        var pfd: android.os.ParcelFileDescriptor? = null
        try {
            pfd = setSource(r, context, uri)
            val found = mutableListOf<Content>()
            for (f in listOf(0.12, 0.27, 0.42, 0.57, 0.72, 0.87)) {
                val bmp = r.getScaledFrameAtTime((durationS * f * 1e6).toLong(), MediaMetadataRetriever.OPTION_CLOSEST_SYNC, W, H) ?: continue
                bounds(bmp)?.let { found += it }
                bmp.recycle()
            }
            if (found.size < 3) return@runCatching null
            // Vote par bord : une scène sombre (bords noirs) ou un passage plein cadre ne
            // décident pas seuls ; à égalité, la valeur la plus prudente (moins de rognage).
            val left = vote(found.map { it.left }, low = true)
            val top = vote(found.map { it.top }, low = true)
            val right = vote(found.map { it.right }, low = false)
            val bottom = vote(found.map { it.bottom }, low = false)
            // Bandes trop fines : rien à rogner (évite un zoom inutile de 1 %).
            val c = Content(
                if (left > 0.02f) left else 0f, if (top > 0.02f) top else 0f,
                if (right < 0.98f) right else 1f, if (bottom < 0.98f) bottom else 1f,
            )
            c
        } finally {
            r.release()
            runCatching { pfd?.close() }
        }
    }.getOrNull()

    /** Source du lecteur d'images : fichier local, ou descripteur pour un partage réseau. */
    fun setSource(r: MediaMetadataRetriever, context: Context, uri: Uri): android.os.ParcelFileDescriptor? {
        if (Smb.isSmb(uri)) {
            val pfd = Smb.open(context, uri) ?: error("smb")
            r.setDataSource(pfd.fileDescriptor)
            return pfd
        }
        r.setDataSource(context, uri)
        return null
    }

    /** Valeur sur laquelle le plus d'images s'accordent (à 1,5 % près). */
    private fun vote(values: List<Float>, low: Boolean): Float {
        val best = values.map { v -> v to values.count { abs(it - v) <= 0.015f } }
        val n = best.maxOf { it.second }
        val pick = best.filter { it.second == n }.map { it.first }
        return if (low) pick.min() else pick.max()
    }

    /** Lignes et colonnes où l'image n'est pas noire ; null si l'image est presque toute noire. */
    private fun bounds(b: Bitmap): Content? {
        val w = b.width; val h = b.height
        val px = IntArray(w * h)
        b.getPixels(px, 0, w, 0, 0, w, h)
        fun lit(p: Int): Boolean {
            val r = p shr 16 and 0xFF; val g = p shr 8 and 0xFF; val bl = p and 0xFF
            return (r * 54 + g * 183 + bl * 19) shr 8 > 30
        }
        val rows = IntArray(h); val cols = IntArray(w)
        for (y in 0 until h) for (x in 0 until w) if (lit(px[y * w + x])) { rows[y]++; cols[x]++ }
        val rowMin = maxOf(2, w / 50); val colMin = maxOf(2, h / 50)
        val t = rows.indexOfFirst { it >= rowMin }; val bt = rows.indexOfLast { it >= rowMin }
        val l = cols.indexOfFirst { it >= colMin }; val rt = cols.indexOfLast { it >= colMin }
        if (t < 0 || l < 0 || bt - t < h / 5 || rt - l < w / 5) return null
        return Content(l.toFloat() / w, t.toFloat() / h, (rt + 1f) / w, (bt + 1f) / h)
    }
}

/** Choix des pistes par défaut : la VO, et les premiers sous-titres complets. */
object Picks {
    private val dub = Regex("""\b(vf|vff|vfq|vfi|vf2|truefrench|dub|dubbed|doublage|doublé)\b""", RegexOption.IGNORE_CASE)
    private val original = Regex("""\b(vo|vost|vostfr|original|v\.o\.?)\b""", RegexOption.IGNORE_CASE)
    private val commentary = Regex("""comment""", RegexOption.IGNORE_CASE)
    private val forced = Regex("""forc|signs|panneaux""", RegexOption.IGNORE_CASE)

    fun audio(tracks: List<Track>): Track? {
        val a = tracks.filter { it.type == "audio" }
        val usable = a.filterNot { commentary.containsMatchIn(it.title) }.ifEmpty { a }
        return usable.firstOrNull { it.original }
            ?: usable.firstOrNull { original.containsMatchIn(it.title) }
            ?: usable.firstOrNull { !dub.containsMatchIn(it.title) }
            ?: usable.firstOrNull()
    }

    fun sub(tracks: List<Track>): Track? {
        val s = tracks.filter { it.type == "sub" && !it.forced && !forced.containsMatchIn(it.title) && !commentary.containsMatchIn(it.title) }
        // Le mode direct n'affiche que les sous-titres texte : ceux-là d'abord.
        return s.firstOrNull { !it.image } ?: s.firstOrNull()
    }
}
