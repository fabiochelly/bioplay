package app.bioplay.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.bioplay.Recent
import app.bioplay.clock

/** Accueil : ouvrir une vidéo ou reprendre une des dernières. */
@Composable
fun HomeScreen(recents: List<Recent>, onOpen: () -> Unit, onResume: (Recent) -> Unit, onForget: (Recent) -> Unit) {
    Box(Modifier.fillMaxSize().background(Color(0xFF05070C))) {
        // Lumières de fond floues : c'est sur elles que le verre prend son relief.
        Canvas(Modifier.fillMaxSize()) {
            fun glow(x: Float, y: Float, r: Float, c: Color) =
                drawCircle(Brush.radialGradient(listOf(c, Color.Transparent), Offset(size.width * x, size.height * y), size.maxDimension * r), size.maxDimension * r, Offset(size.width * x, size.height * y))
            glow(0.18f, 0.25f, 0.45f, Color(0x663A7BFF))
            glow(0.85f, 0.8f, 0.5f, Color(0x55A04BFF))
            glow(0.6f, 0.1f, 0.3f, Color(0x3328E0D0))
        }
        Row(Modifier.fillMaxSize().padding(horizontal = 48.dp, vertical = 32.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(0.9f)) {
                BasicText("bioplay", style = text(44, FontWeight.SemiBold))
                BasicText("Lecteur vidéo, rien de superflu.", style = text(16, color = Ink.secondary), modifier = Modifier.padding(top = 4.dp, bottom = 28.dp))
                GlassButton(onOpen, shape = Capsule, size = 56.dp) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.folder, size = 22.dp)
                        BasicText("Ouvrir une vidéo", style = text(16, FontWeight.SemiBold), modifier = Modifier.padding(start = 10.dp, end = 6.dp))
                    }
                }
            }
            if (recents.isNotEmpty()) {
                Column(Modifier.weight(1.1f).widthIn(max = 520.dp).glass(RoundedCornerShape(28.dp)).padding(14.dp)) {
                    BasicText("Reprendre", style = text(13, FontWeight.SemiBold, Ink.secondary), modifier = Modifier.padding(start = 8.dp, top = 4.dp, bottom = 8.dp))
                    LazyColumn(Modifier.heightIn(max = 320.dp)) {
                        items(recents, key = { it.uri }) { r -> RecentRow(r, { onResume(r) }, { onForget(r) }) }
                    }
                }
            }
        }
    }
}

@Composable
private fun RecentRow(r: Recent, onClick: () -> Unit, onForget: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).clickable(onClick = onClick).padding(horizontal = 10.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            BasicText(r.name.substringBeforeLast('.'), maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(15, FontWeight.SemiBold))
            val f = if (r.duration > 0) (r.position / r.duration).toFloat().coerceIn(0f, 1f) else 0f
            BasicText(
                when {
                    r.duration <= 0 -> "Pas encore lue"
                    r.position < 5 -> "Terminée ou non commencée · ${clock(r.duration)}"
                    else -> "Reste ${clock(r.duration - r.position)}"
                },
                style = text(12, color = Ink.secondary), modifier = Modifier.padding(top = 2.dp, bottom = 6.dp),
            )
            Canvas(Modifier.fillMaxWidth().height(3.dp)) {
                val r3 = CornerRadius(size.height / 2)
                drawRoundRect(Color.White.copy(alpha = 0.14f), cornerRadius = r3)
                drawRoundRect(Ink.accent, size = Size(size.width * f, size.height), cornerRadius = r3)
            }
        }
        Spacer(Modifier.width(10.dp))
        GlassButton(onForget, plain = true, size = 36.dp) { Icon(Icons.close, size = 16.dp, tint = Ink.faint) }
    }
}
