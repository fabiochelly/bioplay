package app.bioplay.ui

import android.os.Build
import android.view.HapticFeedbackConstants
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.runtime.CompositionLocalProvider
import androidx.activity.compose.BackHandler
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.clickable
import androidx.compose.foundation.border
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import app.bioplay.Cue
import app.bioplay.Session
import app.bioplay.clock
import app.bioplay.t
import kotlinx.coroutines.delay
import kotlin.math.abs
import kotlin.math.roundToInt

/** Bandes de l'écran : la hauteur du doigt choisit le pas du glissé horizontal. */
enum class Zone { MINUTE, TEN, LINE }

class Scrub(val zone: Zone, val steps: Int, val target: Double, val cue: Cue?)

enum class Panel { TRACKS, SPEED, ZOOM, LIST }

@Composable
fun PlayerScreen(session: Session, onBack: () -> Unit, onAddSubtitle: () -> Unit) {
    val mpv = session.mpv
    val view = LocalView.current
    var controls by remember { mutableStateOf(true) }
    var poke by remember { mutableIntStateOf(0) }
    var scrub by remember { mutableStateOf<Scrub?>(null) }
    var panel by remember { mutableStateOf<Panel?>(null) }
    var surfaceHolder by remember { mutableStateOf<SurfaceHolder?>(null) }
    val plates = remember { GlassPlates() }

    // Les contrôles s'effacent seuls pendant la lecture, jamais pendant un réglage.
    LaunchedEffect(controls, poke, mpv.paused, panel) {
        if (controls && !mpv.paused && panel == null) {
            delay(2000)
            controls = false
        }
    }

    // La position est notée en continu : on reprend toujours au même endroit, même après
    // un arrêt brutal de l'app.
    LaunchedEffect(mpv.paused) {
        while (!mpv.paused) {
            delay(5000)
            session.save()
        }
    }

    // L'écran passe à la cadence du film (24p sans saccades sur un écran 60/120 Hz).
    LaunchedEffect(mpv.fps, surfaceHolder) {
        val s = surfaceHolder?.surface ?: return@LaunchedEffect
        if (mpv.fps > 1 && s.isValid && Build.VERSION.SDK_INT >= 31) {
            runCatching { s.setFrameRate(mpv.fps.toFloat(), Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE, Surface.CHANGE_FRAME_RATE_ONLY_IF_SEAMLESS) }
        }
    }

    CompositionLocalProvider(LocalPlates provides plates) {
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        AndroidView(
            factory = { ctx ->
                SurfaceView(ctx).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(h: SurfaceHolder) {
                            mpv.attach(h.surface); surfaceHolder = h
                        }
                        override fun surfaceChanged(h: SurfaceHolder, format: Int, w: Int, hgt: Int) = mpv.resize(w, hgt)
                        override fun surfaceDestroyed(h: SurfaceHolder) {
                            surfaceHolder = null; mpv.detach()
                        }
                    })
                }
            },
            modifier = Modifier.fillMaxSize(),
        )

        Gestures(
            session,
            onTap = { controls = !controls; poke++ },
            onDoubleTap = { mpv.togglePause(); controls = !controls },
            onScrub = { scrub = it; poke++ },
            haptic = {
                view.performHapticFeedback(
                    if (Build.VERSION.SDK_INT >= 34) HapticFeedbackConstants.SEGMENT_FREQUENT_TICK else HapticFeedbackConstants.CLOCK_TICK
                )
            },
        )

        AnimatedVisibility(controls && panel == null, enter = fadeIn(tween(160)), exit = fadeOut(tween(260))) {
            Controls(session, onBack, onPanel = { panel = it; poke++ }, onPoke = { poke++ })
        }

        scrub?.let { ScrubOsd(it, session, Modifier.align(Alignment.Center)) }
        Message(session, Modifier.align(Alignment.TopCenter).padding(top = 22.dp))

        // Tiroirs dans l'écran (le flou vient du shader de mpv, pas du compositeur).
        val current = panel
        AnimatedVisibility(
            current != null,
            enter = fadeIn(tween(160)) + slideInVertically(tween(220)) { it / 4 },
            exit = fadeOut(tween(140)) + slideOutVertically(tween(160)) { it / 5 },
        ) {
            var shown by remember { mutableStateOf(current) }
            if (current != null) shown = current
            GlassDrawer({ panel = null }, if (shown == Panel.SPEED || shown == Panel.ZOOM) 480.dp else 620.dp) {
                when (shown) {
                    Panel.TRACKS -> TracksPanel(session, onAddSubtitle)
                    Panel.SPEED -> SpeedPanel(session)
                    Panel.ZOOM -> ZoomPanel(session)
                    Panel.LIST -> ListPanel(session)
                    null -> {}
                }
            }
        }
    }
    }
    BackHandler(panel != null) { panel = null }
    LaunchedEffect(panel != null) { mpv.subsHidden(panel != null) }
    GlassSync(session, plates)
}

/**
 * Un seul détecteur pour tout l'écran : toucher (contrôles, sans attendre un second toucher),
 * double toucher (lecture/pause), glissé horizontal par crans, pincement (zoom, déplacement).
 */
@Composable
private fun Gestures(
    session: Session,
    onTap: () -> Unit,
    onDoubleTap: () -> Unit,
    onScrub: (Scrub?) -> Unit,
    haptic: () -> Unit,
) {
    val mpv = session.mpv
    var lastTap by remember { mutableStateOf(0L to Offset.Zero) }
    Box(
        Modifier.fillMaxSize().pointerInput(session) {
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false)
                val y = down.position.y / size.height
                val zone = when {
                    y < 0.36f -> Zone.MINUTE
                    y < 0.64f -> Zone.TEN
                    else -> Zone.LINE
                }
                val stepPx = when (zone) {
                    Zone.LINE -> 44.dp; Zone.TEN -> 30.dp; Zone.MINUTE -> 38.dp
                }.toPx()
                val from = mpv.position
                var dragging = false
                var pinching = false
                var steps = 0
                var current: Scrub? = null
                while (true) {
                    val ev = awaitPointerEvent()
                    val pressed = ev.changes.count { it.pressed }
                    if (pressed == 0) break
                    if (pressed >= 2 && !dragging) pinching = true
                    if (pinching) {
                        val z = ev.calculateZoom()
                        val pan = ev.calculatePan()
                        val focus = ev.calculateCentroid()
                        session.zoomBy(z.toDouble(), pan.x.toDouble(), pan.y.toDouble(), focus.x.toDouble(), focus.y.toDouble())
                        ev.changes.forEach { it.consume() }
                        continue
                    }
                    val c = ev.changes.firstOrNull { it.id == down.id } ?: continue
                    val dx = c.position.x - down.position.x
                    val dy = c.position.y - down.position.y
                    if (!dragging && abs(dx) > viewConfiguration.touchSlop && abs(dx) > abs(dy) * 1.2f) dragging = true
                    if (!dragging) continue
                    c.consume()
                    val s = (dx / stepPx).toInt()
                    if (s == steps && current != null) continue
                    if (s != steps) haptic()
                    steps = s
                    current = preview(session, zone, steps, from)
                    onScrub(current)
                    // Aperçu immédiat sur l'image clé la plus proche ; saut exact au lâcher.
                    if (steps != 0 && (zone != Zone.LINE || current.cue != null)) mpv.seek(current.target, exact = false)
                }
                when {
                    dragging -> {
                        val s = current
                        if (s != null && s.steps != 0) {
                            if (s.zone == Zone.LINE && s.cue == null) session.stepSub(s.steps, from) else mpv.seek(s.target)
                        } else if (s != null) mpv.seek(from)
                        onScrub(null)
                    }
                    pinching -> {}
                    else -> {
                        val now = System.currentTimeMillis()
                        val (t, at) = lastTap
                        if (now - t < 280 && (at - down.position).getDistance() < 80.dp.toPx()) {
                            lastTap = 0L to Offset.Zero
                            onDoubleTap()
                        } else {
                            lastTap = now to down.position
                            onTap()
                        }
                    }
                }
            }
        }
    )
}

private fun preview(session: Session, zone: Zone, steps: Int, from: Double): Scrub {
    val mpv = session.mpv
    val max = if (mpv.duration > 0) mpv.duration else Double.MAX_VALUE
    return when (zone) {
        Zone.LINE -> {
            val cue = session.cueAfter(steps, from)
            Scrub(zone, steps, cue?.let { session.cueTime(it) } ?: from, cue)
        }
        Zone.TEN -> (from + steps * 10.0).coerceIn(0.0, max).let { Scrub(zone, steps, it, session.cueAt(it)) }
        Zone.MINUTE -> (from + steps * 60.0).coerceIn(0.0, max).let { Scrub(zone, steps, it, session.cueAt(it)) }
    }
}

@Composable
private fun ScrubOsd(s: Scrub, session: Session, modifier: Modifier) {
    val sign = if (s.steps > 0) "+" else if (s.steps < 0) "−" else ""
    val n = abs(s.steps)
    val head = when (s.zone) {
        Zone.LINE -> if (n == 0) t("Sous-titres", "Subtitles") else "$sign$n ${if (n > 1) t("lignes", "lines") else t("ligne", "line")}"
        Zone.TEN -> if (n == 0) "10 s" else "$sign${n * 10} s"
        Zone.MINUTE -> if (n == 0) "1 min" else "$sign$n min"
    }
    Column(
        modifier.widthIn(min = 220.dp, max = 560.dp).glass(RoundedCornerShape(28.dp), tint = 0.42f).padding(horizontal = 26.dp, vertical = 18.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        BasicText(head, style = text(30, FontWeight.SemiBold))
        val total = session.mpv.duration
        BasicText(
            if (s.zone == Zone.LINE && s.cue == null && session.subIndex == null) t("saut par mpv", "jump via mpv")
            else "${clock(s.target)}${if (total > 0) "  /  ${clock(total)}" else ""}",
            style = text(15, color = Ink.secondary),
            modifier = Modifier.padding(top = 2.dp),
        )
        s.cue?.let {
            BasicText(
                it.text, maxLines = 3, overflow = TextOverflow.Ellipsis,
                style = text(17, color = Ink.accent).copy(textAlign = TextAlign.Center),
                modifier = Modifier.padding(top = 10.dp),
            )
        }
    }
}

@Composable
private fun Message(session: Session, modifier: Modifier) {
    var shown by remember { mutableStateOf(false) }
    LaunchedEffect(session.messageAt) {
        if (session.messageAt == 0L) return@LaunchedEffect
        shown = true
        delay(1500)
        shown = false
    }
    AnimatedVisibility(
        shown, modifier,
        enter = fadeIn(tween(140)) + scaleIn(initialScale = 0.9f),
        exit = fadeOut(tween(220)) + scaleOut(targetScale = 0.95f),
    ) {
        BasicText(
            session.message, style = text(15, FontWeight.SemiBold),
            modifier = Modifier.glass(Capsule, tint = 0.4f).padding(horizontal = 18.dp, vertical = 10.dp),
        )
    }
}

@Composable
private fun Controls(session: Session, onBack: () -> Unit, onPanel: (Panel) -> Unit, onPoke: () -> Unit) {
    val mpv = session.mpv
    Box(Modifier.fillMaxSize()) {
        // Voiles en haut et en bas : le verre reste lisible même sur une image claire.
        Box(Modifier.fillMaxWidth().height(130.dp).background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.55f), Color.Transparent))))
        Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(190.dp).background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.6f)))))

        Row(
            Modifier.windowInsetsPadding(WindowInsets.displayCutout).padding(horizontal = 22.dp, vertical = 18.dp).fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GlassButton(onBack, size = 44.dp) { Icon(Icons.back) }
            Column(Modifier.padding(start = 14.dp).weight(1f)) {
                BasicText(session.title.substringBeforeLast('.'), maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(17, FontWeight.SemiBold))
                val info = listOfNotNull(
                    mpv.hwdec.takeIf { it.isNotBlank() && it != "no" }?.let { t("décodage matériel", "hardware decoding") } ?: t("décodage logiciel", "software decoding"),
                    if (session.indexing) t("lecture des sous-titres…", "reading subtitles…") else null,
                ).joinToString(" · ")
                BasicText(info, style = text(12, color = Ink.secondary))
            }
        }

        // Une seule carte de verre sur toute la largeur : progression en haut, lecture à gauche,
        // réglages à droite, tout à l'intérieur avec de l'air autour.
        Column(
            Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.displayCutout)
                .padding(horizontal = 14.dp, vertical = 12.dp).fillMaxWidth()
                .glass(RoundedCornerShape(30.dp), tint = 0.34f).padding(horizontal = 16.dp, vertical = 10.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Clock(session, Modifier.width(72.dp)) { it }
                SeekBar(session, Modifier.weight(1f), onPoke)
                Clock(session, Modifier.width(72.dp), TextAlign.End) { mpv.duration - it }
            }
            Row(Modifier.padding(top = 4.dp).fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                GlassButton({ session.stepSub(-1); onPoke() }, plain = true, size = 50.dp) { Icon(Icons.prevLine, size = 24.dp) }
                GlassButton({ mpv.togglePause(); onPoke() }, plain = true, size = 58.dp) {
                    Icon(if (mpv.paused) Icons.play else Icons.pause, size = 32.dp)
                }
                GlassButton({ session.stepSub(1); onPoke() }, plain = true, size = 50.dp) { Icon(Icons.nextLine, size = 24.dp) }
                Spacer(Modifier.weight(1f))
                TracksButton(session) { onPanel(Panel.TRACKS) }
                Spacer(Modifier.width(10.dp))
                GlassButton({ onPanel(Panel.SPEED) }, plain = true, shape = Capsule, size = 50.dp) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.speed, size = 22.dp, tint = if (mpv.speed != 1.0) Ink.accent else Ink.primary)
                        BasicText(speedLabel(mpv.speed), style = text(15, FontWeight.SemiBold, if (mpv.speed != 1.0) Ink.accent else Ink.primary), modifier = Modifier.padding(start = 7.dp))
                    }
                }
                GlassButton({ onPanel(Panel.ZOOM) }, plain = true, size = 50.dp) {
                    Icon(Icons.zoom, size = 24.dp, tint = if (session.zoom > 1.01 || session.fill) Ink.accent else Ink.primary)
                }
                GlassButton({ onPanel(Panel.LIST) }, plain = true, size = 50.dp) { Icon(Icons.list, size = 24.dp) }
            }
        }
    }
}

fun speedLabel(s: Double) = if (abs(s - 1.0) < 0.001) "1×" else "%.2f".format(s).trimEnd('0').trimEnd('.', ',') + "×"

/** Langue d'une piste pour le bouton : « Français », sinon son titre, raccourci. */
private fun shortName(t: app.bioplay.Track): String {
    val n = app.bioplay.languageName(t.lang) ?: t.label
    return if (n.length > 14) n.take(13) + "…" else n
}

/**
 * Bouton des pistes : haut-parleur + langue audio, sous-titres + leur langue, liseré d'accent.
 */
@Composable
private fun TracksButton(session: Session, onClick: () -> Unit) {
    val mpv = session.mpv
    val audio = mpv.tracks.firstOrNull { it.type == "audio" && it.id == mpv.aid }
    val sub = session.subTrack
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.95f else 1f, spring(dampingRatio = 0.55f, stiffness = 900f), label = "press")
    Row(
        Modifier
            .graphicsLayer { scaleX = scale; scaleY = scale }
            .clip(Capsule)
            .background(Color.White.copy(alpha = if (pressed) 0.16f else 0.07f))
            .border(1.dp, Brush.horizontalGradient(listOf(Ink.accent.copy(alpha = 0.55f), Color.White.copy(alpha = 0.18f), Ink.accent.copy(alpha = 0.4f))), Capsule)
            .clickable(source, indication = null, onClick = onClick)
            .height(44.dp)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.speaker, size = 20.dp)
        BasicText(audio?.let(::shortName) ?: "—", style = text(13, FontWeight.SemiBold), modifier = Modifier.padding(start = 7.dp))
        Box(Modifier.padding(horizontal = 12.dp).width(1.dp).height(20.dp).background(Color.White.copy(alpha = 0.22f)))
        Icon(if (sub != null) Icons.subtitles else Icons.subtitlesOff, size = 20.dp, tint = if (sub != null) Ink.accent else Ink.secondary)
        BasicText(
            sub?.let(::shortName) ?: t("Aucun", "Off"),
            style = text(13, FontWeight.SemiBold, if (sub != null) Ink.accent else Ink.secondary),
            modifier = Modifier.padding(start = 7.dp),
        )
    }
}

@Composable
private fun Clock(session: Session, modifier: Modifier, align: TextAlign = TextAlign.Start, value: (Double) -> Double) {
    // Recompose une fois par seconde, pas à chaque image.
    val second by remember { derivedStateOf { session.mpv.position.toLong() } }
    val shown = value(second.toDouble())
    BasicText(clock(if (align == TextAlign.End) -shown else shown), modifier, style = text(13, FontWeight.SemiBold, Ink.secondary).copy(textAlign = align))
}

@Composable
private fun SeekBar(session: Session, modifier: Modifier, onPoke: () -> Unit) {
    val mpv = session.mpv
    var drag by remember { mutableStateOf<Float?>(null) }
    val thick by animateDpAsState(if (drag != null) 9.dp else 5.dp, label = "seek")
    Canvas(
        modifier.height(36.dp).pointerInput(session) {
            awaitEachGesture {
                val down = awaitFirstDown()
                down.consume()
                fun frac(x: Float) = (x / size.width).coerceIn(0f, 1f)
                drag = frac(down.position.x)
                while (true) {
                    val ev = awaitPointerEvent()
                    val c = ev.changes.firstOrNull { it.id == down.id } ?: break
                    if (!c.pressed) break
                    c.consume()
                    drag = frac(c.position.x)
                    mpv.seek(drag!! * mpv.duration, exact = false)
                    onPoke()
                }
                drag?.let { mpv.seek(it * mpv.duration) }
                drag = null
                onPoke()
            }
        }
    ) {
        val dur = mpv.duration
        val f = drag ?: if (dur > 0) (mpv.position / dur).toFloat() else 0f
        val cache = if (dur > 0) (mpv.cached / dur).toFloat().coerceIn(0f, 1f) else 0f
        val h = thick.toPx()
        val top = (size.height - h) / 2
        val r = CornerRadius(h / 2)
        drawRoundRect(Color.White.copy(alpha = 0.22f), Offset(0f, top), Size(size.width, h), r)
        if (cache > f) drawRoundRect(Color.White.copy(alpha = 0.18f), Offset(0f, top), Size(size.width * cache, h), r)
        drawRoundRect(
            Brush.horizontalGradient(listOf(Ink.accentDeep, Ink.accent, Color.White), endX = size.width * f.coerceAtLeast(0.01f)),
            Offset(0f, top), Size(size.width * f, h), r,
        )
        val x = size.width * f
        if (drag != null) drawCircle(Ink.accent.copy(alpha = 0.25f), h * 2.2f, Offset(x, size.height / 2))
        drawCircle(Color.White, h * 1.15f, Offset(x, size.height / 2))
    }
}

/** Pourcentage arrondi, utile aux panneaux. */
fun pct(v: Double) = (v * 100).roundToInt()
