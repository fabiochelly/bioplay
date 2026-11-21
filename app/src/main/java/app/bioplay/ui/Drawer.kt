package app.bioplay.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.bioplay.Session
import kotlinx.coroutines.flow.distinctUntilChanged
import java.util.Locale

/**
 * Tiroir de verre, dans l'écran : un toucher à côté le ferme. Le flou de la vidéo dessous
 * vient du shader de mpv (indépendant du téléphone, contrairement au flou entre fenêtres).
 */
@Composable
fun GlassDrawer(onDismiss: () -> Unit, width: Dp, content: @Composable ColumnScope.() -> Unit) {
    val screenH = LocalConfiguration.current.screenHeightDp.dp
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss))
        Column(
            Modifier.align(Alignment.BottomCenter).padding(horizontal = 14.dp, vertical = 8.dp)
                .widthIn(max = width).fillMaxWidth().heightIn(max = screenH * 0.92f)
                .glass(RoundedCornerShape(30.dp), tint = 0.5f)
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .padding(horizontal = 20.dp, vertical = 16.dp),
            content = content,
        )
    }
}

/**
 * Transmet à mpv la place des plaques de verre (en pixels de l'image vidéo, bandes noires
 * retirées). Aucune plaque : glass_n=0, le shader ne coûte rien.
 */
@Composable
fun GlassSync(session: Session, plates: GlassPlates) {
    val mpv = session.mpv
    LaunchedEffect(plates) {
        snapshotFlow {
            val list = plates.map.values.take(8)
            buildString {
                append("glass_n=").append(list.size)
                list.forEachIndexed { i, p ->
                    append(String.format(Locale.ROOT, ",r%dx=%.1f,r%dy=%.1f,r%dw=%.1f,r%dh=%.1f,r%dr=%.1f,r%da=1",
                        i, p.x - mpv.marginLeft, i, p.y - mpv.marginTop, i, p.w, i, p.h, i, p.radius, i))
                }
            }
        }.distinctUntilChanged().collect { mpv.setGlass(it) }
    }
}
