package app.bioplay.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.bioplay.Session
import app.bioplay.Lang
import app.bioplay.t
import app.bioplay.Track
import app.bioplay.clock
import kotlin.math.abs
import kotlin.math.roundToInt

private val Row16 = RoundedCornerShape(16.dp)

@Composable
private fun Header(title: String, trailing: String? = null) {
    Row(Modifier.fillMaxWidth().padding(start = 4.dp, bottom = 10.dp), verticalAlignment = Alignment.Bottom) {
        BasicText(title, style = text(13, FontWeight.SemiBold, Ink.secondary))
        Spacer(Modifier.weight(1f))
        trailing?.let { BasicText(it, style = text(13, color = Ink.faint)) }
    }
}

@Composable
private fun Choice(selected: Boolean, onClick: () -> Unit, title: String, detail: String? = null, modifier: Modifier = Modifier, flag: String? = null) {
    Row(
        modifier.fillMaxWidth().padding(vertical = 2.dp).initialFocus(selected).focusRing(Row16).clip16()
            .background(if (selected) Ink.accent.copy(alpha = 0.16f) else Color.Transparent)
            .then(if (selected) Modifier.border(1.dp, Ink.accent.copy(alpha = 0.45f), Row16) else Modifier)
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (flag != null) LangFlag(flag, Modifier.padding(end = 12.dp).width(27.dp).height(18.dp), if (selected) 1f else 0.8f)
        Column(Modifier.weight(1f)) {
            BasicText(title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(15, FontWeight.SemiBold, if (selected) Ink.accent else Ink.primary))
            if (!detail.isNullOrBlank()) BasicText(detail, maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(12, color = Ink.secondary))
        }
        if (selected) Icon(Icons.check, size = 20.dp, tint = Ink.accent)
    }
}

private fun Modifier.clip16() = clip(Row16)

@Composable
fun Chip(label: String, selected: Boolean, compact: Boolean = false, onClick: () -> Unit) {
    BasicText(
        label,
        style = text(if (compact) 12 else 14, FontWeight.SemiBold, if (selected) Color(0xFF06131C) else Ink.primary),
        modifier = Modifier
            .initialFocus(selected)
            .focusRing(Capsule)
            .clip(Capsule)
            .background(if (selected) Ink.accent else Color.White.copy(alpha = 0.10f))
            .border(1.dp, Color.White.copy(alpha = if (selected) 0f else 0.16f), Capsule)
            .clickable(onClick = onClick)
            .padding(horizontal = if (compact) 10.dp else 16.dp, vertical = if (compact) 6.dp else 9.dp),
    )
}

// --- Pistes : audio et sous-titres côte à côte, avec leurs décalages -------------------------

@Composable
fun TracksPanel(session: Session, onAddSubtitle: () -> Unit) {
    val mpv = session.mpv
    // Listes raccourcies sur un écran bas (téléphone en paysage) : décalages toujours visibles.
    val listMax = ((LocalConfiguration.current.screenHeightDp * 0.92f - 150f).coerceIn(110f, 420f)).dp
    val audio = mpv.tracks.filter { it.type == "audio" }
    val subs = mpv.tracks.filter { it.type == "sub" }
    Row(horizontalArrangement = Arrangement.spacedBy(18.dp)) {
        Column(Modifier.weight(1f)) {
            Header(t("Audio", "Audio"), "${audio.size}")
            LazyColumn(Modifier.heightIn(max = listMax)) {
                items(audio, key = { it.id }) { t -> TrackChoice(t, mpv.aid == t.id) { session.chooseAudio(t) } }
                if (audio.isEmpty()) item { BasicText(t("Aucune piste audio", "No audio track"), style = text(14, color = Ink.faint), modifier = Modifier.padding(14.dp)) }
            }
        }
        Column(Modifier.weight(1f)) {
            Header(t("Sous-titres", "Subtitles"), "${subs.size}")
            LazyColumn(Modifier.heightIn(max = listMax)) {
                item { Choice(mpv.sid == null, { session.chooseSub(null) }, t("Désactivés", "Off")) }
                items(subs, key = { it.id }) { t -> TrackChoice(t, mpv.sid == t.id) { session.chooseSub(t) } }
                item {
                    Row(
                        Modifier.fillMaxWidth().padding(top = 4.dp).clip16().clickable(onClick = onAddSubtitle).padding(horizontal = 14.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(Icons.add, size = 18.dp, tint = Ink.secondary)
                        BasicText(t("Ajouter un fichier…", "Add a file…"), style = text(14, color = Ink.secondary), modifier = Modifier.padding(start = 10.dp))
                    }
                }
            }
        }
    }
    Box(Modifier.padding(vertical = 10.dp).fillMaxWidth().height(1.dp).background(Color.White.copy(alpha = 0.12f)))
    // Décalages et langue sur une seule ligne : la place va aux listes de pistes.
    Row(horizontalArrangement = Arrangement.spacedBy(20.dp), verticalAlignment = Alignment.CenterVertically) {
        Delay(t("Décalage audio", "Audio delay"), mpv.audioDelay, 0.05, Modifier.weight(1f)) { mpv.audioDelayTo(it) }
        Delay(t("Décalage sous-titres", "Subtitle delay"), mpv.subDelay, 0.1, Modifier.weight(1f)) { mpv.subDelayTo(it) }
        Spacer(Modifier.width(10.dp))
        LanguageSwitch(compact = true)
    }
}

@Composable
private fun TrackChoice(t: Track, selected: Boolean, onClick: () -> Unit) = Choice(selected, onClick, t.label, t.detail, flag = flagFor(t.lang, t.title))

@Composable
private fun Delay(label: String, value: Double, step: Double, modifier: Modifier, set: (Double) -> Unit) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f).padding(start = 4.dp)) {
            BasicText(label, style = text(13, FontWeight.SemiBold, Ink.secondary))
            val ms = (value * 1000).roundToInt()
            BasicText(
                if (ms == 0) "0 ms" else "%+d ms".format(ms).replace('-', '−'),
                style = text(17, FontWeight.SemiBold, if (ms == 0) Ink.primary else Ink.accent),
                modifier = Modifier.clickable { set(0.0) },
            )
        }
        GlassButton({ set(value - step) }, size = 42.dp) { Icon(Icons.minus, size = 20.dp) }
        Spacer(Modifier.width(8.dp))
        GlassButton({ set(value + step) }, size = 42.dp) { Icon(Icons.add, size = 20.dp) }
    }
}

/** Bascule de langue : un bouton double, drapeau discret et code, la langue active teintée. */
@Composable
fun LanguageSwitch(compact: Boolean = false) {
    val context = androidx.compose.ui.platform.LocalContext.current
    Row(
        Modifier.height(if (compact) 42.dp else 46.dp).clip(Capsule)
            .background(Color.White.copy(alpha = 0.06f))
            .border(1.dp, Color.White.copy(alpha = 0.18f), Capsule),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        LanguageHalf("FR", !Lang.en, compact, french = true) { if (Lang.en) Lang.toggle(context) }
        LanguageHalf("EN", Lang.en, compact, french = false) { if (!Lang.en) Lang.toggle(context) }
    }
}

@Composable
private fun LanguageHalf(code: String, selected: Boolean, compact: Boolean, french: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxHeight().background(if (selected) Ink.accent.copy(alpha = 0.20f) else Color.Transparent)
            .clickable(onClick = onClick)
            .padding(horizontal = if (compact) 12.dp else 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        LangFlag(if (french) "fr" else "en", Modifier.width(if (compact) 24.dp else 27.dp).height(if (compact) 16.dp else 18.dp), if (selected) 1f else 0.55f)
        BasicText(code, style = text(if (compact) 12 else 14, FontWeight.SemiBold, if (selected) Ink.accent else Ink.secondary), modifier = Modifier.padding(start = 7.dp))
    }
}

// --- Vitesse ----------------------------------------------------------------------------------

@Composable
fun SpeedPanel(session: Session) {
    val mpv = session.mpv
    Row(verticalAlignment = Alignment.Bottom) {
        Column(Modifier.weight(1f).padding(start = 4.dp)) {
            BasicText(t("Vitesse", "Speed"), style = text(13, FontWeight.SemiBold, Ink.secondary))
            BasicText(t("Voix préservée, sans distorsion", "Pitch preserved, no distortion"), style = text(12, color = Ink.faint))
        }
        BasicText(speedLabel(mpv.speed), style = text(34, FontWeight.SemiBold, if (mpv.speed != 1.0) Ink.accent else Ink.primary))
    }
    Spacer(Modifier.height(14.dp))
    GlassSlider(mpv.speed, 0.5, 2.0, 0.05, marks = listOf(1.0, 1.5)) { mpv.speedTo(it) }
    Spacer(Modifier.height(14.dp))
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf(0.75, 1.0, 1.1, 1.25, 1.4, 1.5).forEach { s -> Chip(speedLabel(s), abs(mpv.speed - s) < 0.001) { mpv.speedTo(s) } }
    }
}

// --- Zoom -------------------------------------------------------------------------------------

@Composable
fun ZoomPanel(session: Session) {
    Row(verticalAlignment = Alignment.Bottom) {
        Column(Modifier.weight(1f).padding(start = 4.dp)) {
            BasicText(t("Image", "Picture"), style = text(13, FontWeight.SemiBold, Ink.secondary))
            BasicText(t("Pincez l'écran pour zoomer, deux doigts pour déplacer", "Pinch to zoom, two fingers to move"), style = text(12, color = Ink.faint))
        }
        BasicText("%.1f×".format(session.zoom), style = text(34, FontWeight.SemiBold, if (session.zoom > 1.01) Ink.accent else Ink.primary))
    }
    Spacer(Modifier.height(14.dp))
    GlassSlider(session.zoom, 1.0, 4.0, 0.1, marks = listOf(2.0, 3.0)) { session.setZoomLevel(it) }
    Spacer(Modifier.height(14.dp))
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Chip(t("Image entière", "Fit"), !session.fill && session.zoom <= 1.01) { session.resetZoom() }
        Chip(t("Remplir l'écran", "Fill screen"), session.fill) { session.toggleFill() }
    }
}

// --- Liste des sous-titres ---------------------------------------------------------------------

@Composable
fun ListPanel(session: Session) {
    val idx = session.subIndex
    val track = session.subTrack
    val height = (LocalConfiguration.current.screenHeightDp * 0.8f - 110f).coerceAtLeast(120f).dp
    Header(
        track?.let { "${t("Sous-titres", "Subtitles")} · ${it.label}" } ?: t("Sous-titres", "Subtitles"),
        idx?.let { "${it.cues.size} ${t("lignes", "lines")}" },
    )
    if (idx == null) {
        val why = when {
            track == null -> t("Choisissez une piste de sous-titres.", "Pick a subtitle track.")
            session.indexing -> t("Lecture des sous-titres…", "Reading subtitles…")
            else -> t("Liste indisponible pour ce format. Les sauts ligne à ligne passent par mpv.", "No list for this format. Line jumps go through mpv.")
        }
        BasicText(why, style = text(15, color = Ink.secondary), modifier = Modifier.padding(4.dp, 8.dp, 4.dp, 12.dp))
        return
    }
    val current by remember(idx) { derivedStateOf { idx.at(session.subClock()) } }
    val list = rememberLazyListState(initialFirstVisibleItemIndex = (current - 2).coerceAtLeast(0))
    var touched by remember { mutableLongStateOf(0L) }
    LaunchedEffect(list) {
        snapshotFlow { list.isScrollInProgress }.collect { if (it) touched = System.currentTimeMillis() }
    }
    // Suit la lecture, sauf si l'on vient de faire défiler la liste à la main.
    LaunchedEffect(current) {
        if (current >= 0 && System.currentTimeMillis() - touched > 4000 && !list.isScrollInProgress) {
            list.animateScrollToItem((current - 2).coerceAtLeast(0))
        }
    }
    LazyColumn(Modifier.height(height), state = list) {
        itemsIndexed(idx.cues) { i, c ->
            val now = i == current
            Row(
                Modifier.fillMaxWidth().clip16()
                    .background(if (now) Ink.accent.copy(alpha = 0.14f) else Color.Transparent)
                    .clickable { session.mpv.seek(session.cueTime(c)) }
                    .padding(horizontal = 12.dp, vertical = 9.dp),
            ) {
                BasicText(clock(c.start / 1000.0), style = text(13, FontWeight.SemiBold, if (now) Ink.accent else Ink.faint), modifier = Modifier.width(72.dp).padding(top = 1.dp))
                BasicText(c.text, style = text(15, if (now) FontWeight.SemiBold else FontWeight.Medium, if (now) Ink.primary else Ink.secondary))
            }
        }
    }
}

// --- Curseur ----------------------------------------------------------------------------------

@Composable
fun GlassSlider(value: Double, min: Double, max: Double, step: Double, marks: List<Double> = emptyList(), onChange: (Double) -> Unit) {
    fun snap(f: Float): Double {
        val v = min + f.coerceIn(0f, 1f) * (max - min)
        val s = (v / step).roundToInt() * step
        // Attraction douce vers les repères (1×, 1,5×…).
        return marks.firstOrNull { abs(it - s) < step * 0.9 } ?: s.coerceIn(min, max)
    }
    Canvas(
        Modifier.fillMaxWidth().height(40.dp).pointerInput(min, max) {
            awaitEachGesture {
                val down = awaitFirstDown()
                down.consume()
                onChange(snap(down.position.x / size.width))
                while (true) {
                    val ev = awaitPointerEvent()
                    val c = ev.changes.firstOrNull { it.id == down.id } ?: break
                    if (!c.pressed) break
                    c.consume()
                    onChange(snap(c.position.x / size.width))
                }
            }
        }
    ) {
        val h = 8.dp.toPx()
        val top = (size.height - h) / 2
        val f = ((value - min) / (max - min)).toFloat().coerceIn(0f, 1f)
        drawRoundRect(Color.White.copy(alpha = 0.16f), Offset(0f, top), Size(size.width, h), CornerRadius(h / 2))
        drawRoundRect(Brush.horizontalGradient(listOf(Ink.accentDeep, Ink.accent)), Offset(0f, top), Size(size.width * f, h), CornerRadius(h / 2))
        marks.forEach {
            val x = ((it - min) / (max - min)).toFloat() * size.width
            drawCircle(Color.White.copy(alpha = 0.55f), 2.dp.toPx(), Offset(x, size.height / 2))
        }
        drawCircle(Color.Black.copy(alpha = 0.25f), 13.dp.toPx(), Offset(size.width * f, size.height / 2 + 1.dp.toPx()))
        drawCircle(Color.White, 12.dp.toPx(), Offset(size.width * f, size.height / 2))
    }
}
