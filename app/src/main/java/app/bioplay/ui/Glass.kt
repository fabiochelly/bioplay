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
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
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

fun text(size: Int, weight: FontWeight = FontWeight.Medium, color: Color = Ink.primary) =
    TextStyle(fontFamily = Inter, fontWeight = weight, fontSize = size.sp, color = color, letterSpacing = (-0.01 * size).sp)

/**
 * Verre liquide : teinte sombre légère, reflet vertical, liseré spéculaire plus vif en haut,
 * et halo lumineux. Tout est dessiné en une passe, sans calque hors écran.
 */
fun Modifier.glass(shape: Shape, tint: Float = 0.28f, glow: Float = 1f): Modifier = this
    .clip(shape)
    .background(Color.Black.copy(alpha = tint), shape)
    .background(
        Brush.verticalGradient(
            0f to Color.White.copy(alpha = 0.17f * glow),
            0.45f to Color.White.copy(alpha = 0.07f * glow),
            1f to Color.White.copy(alpha = 0.10f * glow),
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
            0f to Color.White.copy(alpha = 0.55f * glow),
            0.5f to Color.White.copy(alpha = 0.10f * glow),
            1f to Color.White.copy(alpha = 0.26f * glow),
        ),
        shape,
    )

val Capsule = RoundedCornerShape(percent = 50)

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
