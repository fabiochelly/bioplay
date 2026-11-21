package app.bioplay.ui

import android.content.Context
import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import kotlin.math.cos
import kotlin.math.sin

/**
 * Drapeaux des langues, décrits une seule fois dans `shared/flags.txt` (formes simples), communs
 * avec le bureau. [load] lit le fichier au démarrage.
 */
object Flags {
    private sealed interface Op
    private class Bands(val vertical: Boolean, val bands: List<Pair<Float, Color>>) : Op
    private class Box(val v: FloatArray, val c: Color) : Op
    private class Disc(val v: FloatArray, val c: Color) : Op
    private class Ring(val v: FloatArray, val c: Color) : Op
    private class Star(val v: FloatArray, val rot: Float, val c: Color) : Op
    private class Line(val v: FloatArray, val c: Color) : Op
    private class Poly(val v: FloatArray, val c: Color) : Op

    private class Flag(val key: String, val codes: List<String>, val names: List<String>, val ops: MutableList<Op> = mutableListOf())

    private var flags: List<Flag> = emptyList()

    private fun color(s: String) = Color(0xFF000000 or s.removePrefix("#").toLong(16))

    fun load(context: Context) {
        if (flags.isNotEmpty()) return
        val out = mutableListOf<Flag>()
        context.assets.open("flags.txt").bufferedReader().useLines { lines ->
            for (raw in lines) {
                val line = raw.trim()
                if (line.isEmpty() || line.startsWith("#")) continue
                if (line.startsWith("@")) {
                    val left = line.drop(1).substringBefore('|').trim().split(Regex("\\s+"))
                    val names = line.substringAfter('|', "").split(',').map { it.trim().lowercase() }.filter { it.isNotEmpty() }
                    out += Flag(left[0], left.getOrNull(1).orEmpty().split(',').map { it.trim().lowercase() }.filter { it.isNotEmpty() }, names)
                    continue
                }
                val flag = out.lastOrNull() ?: continue
                val parts = line.split(Regex("\\s+"))
                val args = parts.drop(1)
                val nums = args.filter { !it.startsWith("#") && ':' !in it }.mapNotNull { it.toFloatOrNull() }.toFloatArray()
                val col = args.firstOrNull { it.startsWith("#") }?.let(::color) ?: Color.White
                flag.ops += when (parts[0]) {
                    "h", "v" -> {
                        val bands = args.mapNotNull { a -> a.split(':').takeIf { it.size == 2 }?.let { (it[0].toFloatOrNull() ?: 1f) to color(it[1]) } }
                        val total = bands.sumOf { it.first.toDouble() }.toFloat()
                        Bands(parts[0] == "v", bands.map { it.first / total to it.second })
                    }
                    "r" -> Box(nums, col)
                    "c" -> Disc(nums, col)
                    "o" -> Ring(nums, col)
                    "s" -> Star(nums, if (args.last().startsWith("#")) 0f else args.last().toFloat(), col)
                    "l" -> Line(nums, col)
                    "p" -> Poly(nums, col)
                    else -> continue
                }
            }
        }
        flags = out
    }

    /** Drapeau d'une piste : d'après son code de langue (régional d'abord), sinon son titre. */
    fun forTrack(lang: String, title: String): String? {
        val l = lang.trim().lowercase().replace('_', '-')
        if (l.isNotEmpty() && l != "und") {
            flags.firstOrNull { l in it.codes }?.let { return it.key }
            val primary = l.substringBefore('-')
            flags.firstOrNull { primary in it.codes }?.let { return it.key }
        }
        val t = title.lowercase()
        val words = t.split(Regex("[^\\p{L}\\p{N}]+")).filter { it.isNotEmpty() }.toSet()
        return flags.firstOrNull { f ->
            f.names.any { n -> if (' ' in n || n.any { it.code > 127 }) n in t else n in words }
        }?.key
    }

    fun has(key: String) = flags.any { it.key == key }

    private fun DrawScope.star(cx: Float, cy: Float, r: Float, rot: Float, c: Color) {
        val p = Path()
        for (i in 0 until 10) {
            val rr = if (i % 2 == 0) r else r * 0.4f
            val a = rot - Math.PI.toFloat() / 2 + i * Math.PI.toFloat() / 5
            if (i == 0) p.moveTo(cx + rr * cos(a), cy + rr * sin(a)) else p.lineTo(cx + rr * cos(a), cy + rr * sin(a))
        }
        p.close()
        drawPath(p, c)
    }

    fun DrawScope.paint(key: String) {
        val flag = flags.firstOrNull { it.key == key } ?: return
        val w = size.width
        val h = size.height
        for (op in flag.ops) when (op) {
            is Bands -> {
                var t = 0f
                for ((f, c) in op.bands) {
                    if (op.vertical) drawRect(c, Offset(t * w, 0f), Size(f * w + 1, h))
                    else drawRect(c, Offset(0f, t * h), Size(w, f * h + 1))
                    t += f
                }
            }
            is Box -> drawRect(op.c, Offset(op.v[0] * w, op.v[1] * h), Size(op.v[2] * w, op.v[3] * h))
            is Disc -> drawCircle(op.c, op.v[2] * h, Offset(op.v[0] * w, op.v[1] * h))
            is Ring -> drawCircle(op.c, op.v[2] * h, Offset(op.v[0] * w, op.v[1] * h), style = Stroke(op.v[3] * h))
            is Star -> star(op.v[0] * w, op.v[1] * h, op.v[2] * h, op.rot, op.c)
            is Line -> drawLine(op.c, Offset(op.v[0] * w, op.v[1] * h), Offset(op.v[2] * w, op.v[3] * h), op.v[4] * h)
            is Poly -> {
                val p = Path()
                for (i in op.v.indices step 2) {
                    if (i == 0) p.moveTo(op.v[i] * w, op.v[i + 1] * h) else p.lineTo(op.v[i] * w, op.v[i + 1] * h)
                }
                p.close()
                drawPath(p, op.c)
            }
        }
    }
}

/** Drapeau d'une piste (ou null si la langue n'a pas de drapeau). */
fun flagFor(lang: String, title: String): String? = Flags.forTrack(lang, title)

/**
 * Drapeau pris dans une dalle de verre : peint opaque (les bandes superposées ne se voient
 * pas à travers), rendu transparent d'un bloc, reflet en haut, ombre en bas, liseré fin.
 * Coins droits : le drapeau ne déborde jamais de son liseré.
 */
@Composable
fun LangFlag(code: String, modifier: Modifier, alpha: Float = 1f) {
    Canvas(modifier.graphicsLayer { this.alpha = 0.92f * alpha; compositingStrategy = CompositingStrategy.Offscreen }.clipToBounds()) {
        with(Flags) { paint(code) }
        val w = size.width
        val h = size.height
        drawRect(Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.4f), Color.White.copy(alpha = 0.05f)), endY = h * 0.5f), size = Size(w, h * 0.5f))
        drawRect(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.2f)), startY = h * 0.55f), Offset(0f, h * 0.55f), Size(w, h * 0.45f))
        drawRect(Color.White.copy(alpha = 0.45f), style = Stroke(1.5f))
    }
}
