package app.bioplay.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.dp
import kotlin.math.cos
import kotlin.math.sin

/** Langues qui ont un drapeau : code ISO 639-1. */
val FLAGS = setOf("fr", "en", "de", "es", "it", "pt", "ja", "zh", "ko", "ru", "nl", "pl", "sv", "tr")

private val CODES = mapOf(
    "fre" to "fr", "fra" to "fr", "eng" to "en", "ger" to "de", "deu" to "de", "spa" to "es", "ita" to "it",
    "por" to "pt", "jpn" to "ja", "chi" to "zh", "zho" to "zh", "kor" to "ko", "rus" to "ru", "dut" to "nl",
    "nld" to "nl", "pol" to "pl", "swe" to "sv", "tur" to "tr",
)

private val NAMES = listOf(
    "fr" to listOf("français", "francais", "french", "vff", "vfq", "vf"),
    "en" to listOf("english", "anglais"),
    "de" to listOf("deutsch", "german", "allemand"),
    "es" to listOf("español", "espanol", "spanish", "espagnol", "castellano"),
    "it" to listOf("italiano", "italian", "italien"),
    "pt" to listOf("português", "portugues", "portuguese", "portugais"),
    "ja" to listOf("日本語", "japanese", "japonais"),
    "zh" to listOf("中文", "chinese", "chinois", "mandarin"),
    "ko" to listOf("한국어", "korean", "coréen"),
    "ru" to listOf("русский", "russian", "russe"),
    "nl" to listOf("nederlands", "dutch", "néerlandais"),
    "pl" to listOf("polski", "polish", "polonais"),
    "sv" to listOf("svenska", "swedish", "suédois"),
    "tr" to listOf("türkçe", "turkish", "turc"),
)

/** Drapeau d'une piste : d'après son code de langue, sinon d'après son titre. */
fun flagFor(lang: String, title: String): String? {
    val code = lang.lowercase().substringBefore('-').let { CODES[it] ?: it }
    if (code in FLAGS) return code
    val t = title.lowercase()
    return NAMES.firstOrNull { (_, words) -> words.any { w -> Regex("(^|[^a-zà-ÿ])" + Regex.escape(w) + "($|[^a-zà-ÿ])").containsMatchIn(t) } }?.first
}

/**
 * Drapeau pris dans une dalle de verre : peint opaque (les bandes superposées ne se voient
 * pas à travers), puis rendu transparent d'un bloc ; reflet en haut, ombre en bas, liseré.
 */
@Composable
fun LangFlag(code: String, modifier: Modifier, alpha: Float = 1f) {
    Canvas(modifier.graphicsLayer { this.alpha = 0.9f * alpha; compositingStrategy = CompositingStrategy.Offscreen }.clip(RoundedCornerShape(4.dp))) {
        paintFlag(code)
        val w = size.width
        val h = size.height
        drawRect(Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.42f), Color.White.copy(alpha = 0.06f)), endY = h * 0.5f), size = Size(w, h * 0.5f))
        drawRect(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.22f)), startY = h * 0.55f), Offset(0f, h * 0.55f), Size(w, h * 0.45f))
        drawRoundRect(Color.White.copy(alpha = 0.5f), style = Stroke(1.2f), cornerRadius = CornerRadius(4.dp.toPx()))
    }
}

private val BLUE = Color(0xFF2F5DB5)
private val RED = Color(0xFFE0473F)
private val WHITE = Color(0xFFF4F5F7)
private val YELLOW = Color(0xFFF5C23C)

private fun DrawScope.vertical(vararg bands: Pair<Float, Color>) {
    var x = 0f
    for ((f, c) in bands) {
        drawRect(c, Offset(x * size.width, 0f), Size(f * size.width + 1, size.height)); x += f
    }
}

private fun DrawScope.horizontal(vararg bands: Pair<Float, Color>) {
    var y = 0f
    for ((f, c) in bands) {
        drawRect(c, Offset(0f, y * size.height), Size(size.width, f * size.height + 1)); y += f
    }
}

private fun DrawScope.star(cx: Float, cy: Float, r: Float, color: Color, rot: Float = 0f) {
    val p = Path()
    for (i in 0 until 10) {
        val rr = if (i % 2 == 0) r else r * 0.4f
        val a = rot - Math.PI.toFloat() / 2 + i * Math.PI.toFloat() / 5
        val x = cx + rr * cos(a)
        val y = cy + rr * sin(a)
        if (i == 0) p.moveTo(x, y) else p.lineTo(x, y)
    }
    p.close()
    drawPath(p, color)
}

private fun DrawScope.paintFlag(code: String) {
    val w = size.width
    val h = size.height
    when (code) {
        "fr" -> vertical(1f / 3 to BLUE, 1f / 3 to WHITE, 1f / 3 to RED)
        "it" -> vertical(1f / 3 to Color(0xFF2E9A5A), 1f / 3 to WHITE, 1f / 3 to RED)
        "de" -> horizontal(1f / 3 to Color(0xFF1E1E22), 1f / 3 to Color(0xFFD93A33), 1f / 3 to YELLOW)
        "es" -> horizontal(0.25f to Color(0xFFC8352E), 0.5f to YELLOW, 0.25f to Color(0xFFC8352E))
        "ru" -> horizontal(1f / 3 to WHITE, 1f / 3 to Color(0xFF2B57B0), 1f / 3 to RED)
        "nl" -> horizontal(1f / 3 to Color(0xFFC8352E), 1f / 3 to WHITE, 1f / 3 to Color(0xFF2A4C9A))
        "pl" -> horizontal(0.5f to WHITE, 0.5f to Color(0xFFDC3548))
        "pt" -> {
            vertical(0.4f to Color(0xFF2E8B4E), 0.6f to RED)
            drawCircle(YELLOW, h * 0.22f, Offset(w * 0.4f, h / 2))
            drawCircle(RED, h * 0.13f, Offset(w * 0.4f, h / 2))
        }
        "ja" -> {
            drawRect(WHITE); drawCircle(Color(0xFFC8352E), h * 0.3f, Offset(w / 2, h / 2))
        }
        "zh" -> {
            drawRect(Color(0xFFD7382E))
            star(w * 0.17f, h * 0.28f, h * 0.17f, YELLOW)
            for ((x, y) in listOf(0.33f to 0.1f, 0.4f to 0.22f, 0.4f to 0.38f, 0.33f to 0.5f)) star(w * x, h * y, h * 0.06f, YELLOW)
        }
        "ko" -> {
            drawRect(WHITE)
            val c = Offset(w / 2, h / 2)
            val r = h * 0.25f
            drawArc(Color(0xFFCD3A3F), 180f, 180f, true, Offset(c.x - r, c.y - r), Size(2 * r, 2 * r))
            drawArc(Color(0xFF2B5BA8), 0f, 180f, true, Offset(c.x - r, c.y - r), Size(2 * r, 2 * r))
            val dark = Color(0xFF26262B)
            for ((x, y) in listOf(0.16f to 0.22f, 0.84f to 0.22f, 0.16f to 0.78f, 0.84f to 0.78f)) {
                for (k in -1..1) drawRect(dark, Offset(w * x - w * 0.06f, h * y + k * h * 0.07f - h * 0.025f), Size(w * 0.12f, h * 0.05f))
            }
        }
        "sv" -> {
            drawRect(Color(0xFF2D6CB5))
            drawRect(YELLOW, Offset(w * 0.3f, 0f), Size(w * 0.13f, h))
            drawRect(YELLOW, Offset(0f, h * 0.4f), Size(w, h * 0.2f))
        }
        "tr" -> {
            drawRect(Color(0xFFD7382E))
            drawCircle(WHITE, h * 0.27f, Offset(w * 0.38f, h / 2))
            drawCircle(Color(0xFFD7382E), h * 0.22f, Offset(w * 0.43f, h / 2))
            star(w * 0.62f, h / 2, h * 0.13f, WHITE, rot = -0.3f)
        }
        "en" -> {
            val navy = Color(0xFF233F8C)
            val red = Color(0xFFD23A48)
            drawRect(navy)
            drawLine(WHITE, Offset(-1f, -1f), Offset(w + 1, h + 1), h * 0.2f)
            drawLine(WHITE, Offset(-1f, h + 1), Offset(w + 1, -1f), h * 0.2f)
            drawLine(red, Offset(-1f, -1f), Offset(w + 1, h + 1), h * 0.067f)
            drawLine(red, Offset(-1f, h + 1), Offset(w + 1, -1f), h * 0.067f)
            drawRect(WHITE, Offset(w / 2 - h * 0.167f, 0f), Size(h * 0.333f, h))
            drawRect(WHITE, Offset(0f, h * 0.333f), Size(w, h * 0.333f))
            drawRect(red, Offset(w / 2 - h * 0.1f, 0f), Size(h * 0.2f, h))
            drawRect(red, Offset(0f, h * 0.4f), Size(w, h * 0.2f))
        }
    }
}
