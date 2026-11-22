package app.bioplay.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.clickable
import androidx.compose.foundation.Image
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.composed
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import android.content.res.AssetManager

object Ink {
    val primary = Color(0xFFF7F8FA)
    val secondary = Color(0xB8F7F8FA)
    val faint = Color(0x66F7F8FA)
    /** Accent : un bleu glacier lumineux, lisible sur toute image. */
    val accent = Color(0xFF8FD8FF)
    val accentDeep = Color(0xFF4FB2F0)
}

lateinit var Inter: FontFamily

fun loadFonts(assets: AssetManager) {
    Inter = FontFamily(
        Font("inter-medium.ttf", assets, FontWeight.Medium),
        Font("inter-semibold.ttf", assets, FontWeight.SemiBold),
    )
}

/** Tailles de texte un peu plus grandes que sur le bureau : lues à bout de bras sur un téléphone. */
private const val MOBILE = 1.15f

fun text(size: Int, weight: FontWeight = FontWeight.Medium, color: Color = Ink.primary) =
    TextStyle(fontFamily = Inter, fontWeight = weight, fontSize = (size * MOBILE).sp, color = color, letterSpacing = (-0.01 * size).sp)

/** Une plaque de verre à l'écran (pixels de la fenêtre) : mpv floute la vidéo dessous. */
data class Plate(val x: Float, val y: Float, val w: Float, val h: Float, val radius: Float)

class GlassPlates {
    val map = mutableStateMapOf<Any, Plate>()
}

val LocalPlates = staticCompositionLocalOf<GlassPlates?> { null }

/**
 * Verre liquide : teinte sombre légère, reflet vertical, liseré spéculaire plus vif en haut,
 * et halo lumineux. Tout est dessiné en une passe, sans calque hors écran.
 */
fun Modifier.glass(shape: Shape, tint: Float = 0.42f, glow: Float = 1f): Modifier = composed {
    // Chaque plaque signale sa place : le shader de mpv floute la vidéo exactement dessous.
    val plates = LocalPlates.current
    val key = remember { Any() }
    val density = LocalDensity.current
    val dir = LocalLayoutDirection.current
    DisposableEffect(plates) { onDispose { plates?.map?.remove(key) } }
    this.then(
        if (plates == null) Modifier else Modifier.onGloballyPositioned { c ->
            val b = c.boundsInWindow()
            val outline = shape.createOutline(Size(b.width, b.height), dir, density)
            val r = (outline as? Outline.Rounded)?.roundRect?.topLeftCornerRadius?.x ?: 0f
            plates.map[key] = Plate(b.left, b.top, b.width, b.height, r)
        }
    ).glassLook(shape, tint, glow)
}

private fun Modifier.glassLook(shape: Shape, tint: Float, glow: Float): Modifier = this
    .clip(shape)
    .background(Color.Black.copy(alpha = tint), shape)
    .background(
        Brush.verticalGradient(
            0f to Color.White.copy(alpha = 0.09f * glow),
            0.45f to Color.White.copy(alpha = 0.04f * glow),
            1f to Color.White.copy(alpha = 0.05f * glow),
        ),
        shape,
    )
    .drawWithContent {
        // Reflet : une lueur douce en haut à gauche, comme une lumière venue du dessus.
        drawRect(
            Brush.radialGradient(
                listOf(Color.White.copy(alpha = 0.16f * glow), Color.Transparent),
                center = Offset(size.width * 0.22f, 0f),
                radius = size.maxDimension * 0.6f,
            )
        )
        drawContent()
    }
    .border(
        1.dp,
        Brush.verticalGradient(
            0f to Color.White.copy(alpha = 0.24f * glow),
            0.5f to Color.White.copy(alpha = 0.12f * glow),
            1f to Color.White.copy(alpha = 0.16f * glow),
        ),
        shape,
    )

val Capsule = RoundedCornerShape(percent = 50)

/**
 * Focus à la télécommande ou au clavier : l'élément grossit un peu et prend un liseré d'accent.
 * Rien au toucher (un élément touché ne prend pas le focus). À placer avant `clickable`.
 */
fun Modifier.focusRing(shape: Shape): Modifier = composed {
    var focused by remember { androidx.compose.runtime.mutableStateOf(false) }
    val k by animateFloatAsState(if (focused) 1f else 0f, androidx.compose.animation.core.tween(120), label = "focus")
    this
        .onFocusChanged { focused = it.isFocused || it.hasFocus }
        .graphicsLayer { val sc = 1f + 0.06f * k; scaleX = sc; scaleY = sc }
        .drawWithContent {
            drawContent()
            if (k > 0f) {
                val o = shape.createOutline(size, layoutDirection, this)
                drawOutline(o, Color.White.copy(alpha = 0.10f * k))
                drawOutline(o, Ink.accent.copy(alpha = k), style = androidx.compose.ui.graphics.drawscope.Stroke(2.5.dp.toPx()))
            }
        }
}

/** Élément à qui donner le focus à l'ouverture d'un tiroir (le choix en cours). */
val LocalDrawerFocus = staticCompositionLocalOf<androidx.compose.ui.focus.FocusRequester?> { null }

/** Attache le focus initial du tiroir à cet élément s'il est le choix en cours. */
@Composable
fun Modifier.initialFocus(selected: Boolean): Modifier {
    val fr = LocalDrawerFocus.current
    return if (selected && fr != null) this.focusRequester(fr) else this
}

/** Bouton de verre rond ou en capsule, qui s'enfonce légèrement sous le doigt. */
@Composable
fun GlassButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    shape: Shape = CircleShape,
    size: Dp = 48.dp,
    plain: Boolean = false,
    content: @Composable BoxScope.() -> Unit,
) {
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.9f else 1f, spring(dampingRatio = 0.55f, stiffness = 900f), label = "press")
    Box(
        modifier
            .focusRing(shape)
            .graphicsLayer { scaleX = scale; scaleY = scale }
            .then(if (plain) Modifier.clip(shape) else Modifier.glass(shape))
            .background(if (pressed) Color.White.copy(alpha = 0.14f) else Color.Transparent)
            .clickable(source, indication = null, onClick = onClick)
            .then(if (shape === CircleShape) Modifier.size(size) else Modifier.height(size).padding(horizontal = 16.dp)),
        contentAlignment = Alignment.Center,
        content = content,
    )
}

@Composable
fun Icon(icon: ImageVector, modifier: Modifier = Modifier, size: Dp = 24.dp, tint: Color = Ink.primary) {
    Image(icon, null, modifier.size(size), colorFilter = ColorFilter.tint(tint))
}
