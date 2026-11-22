package app.bioplay.ui

import androidx.compose.foundation.focusable
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.InputMode
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalInputModeManager

import android.os.Build
import android.view.HapticFeedbackConstants
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.ui.unit.IntOffset
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.unit.sp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.BoxWithConstraints
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
import androidx.compose.ui.platform.LocalContext
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

/**
 * Sous-titres du mode direct : texte fourni par mpv, dessiné ici en Inter SemiBold avec un liseré
 * noir. Placés dans l'image, et au-dessus de la carte de commandes quand elle est affichée.
 */
@Composable
private fun BoxScope.Subtitles(session: Session, plates: GlassPlates, sw: Float, sh: Float, videoH: Float) {
    val mpv = session.mpv
    val text = mpv.subText
    if (!mpv.direct || text.isBlank()) return
    val density = LocalDensity.current
    val shownH = minOf(videoH, sh)
    val fontPx = shownH * 0.072f
    val videoBottom = (sh + shownH) / 2
    var bottom = sh - videoBottom + shownH * 0.05f
    bottomUiTop(plates, sh)?.let { top -> bottom = maxOf(bottom, sh - top + 10 * density.density) }
    val style = text(16, FontWeight.SemiBold, Color(0xFFEBEBEB)).copy(
        fontSize = with(density) { fontPx.toSp() },
        lineHeight = with(density) { (fontPx * 1.18f).toSp() },
        textAlign = TextAlign.Center,
        letterSpacing = 0.sp,
    )
    val outline = style.copy(color = Color(0xF20A0A0A), drawStyle = androidx.compose.ui.graphics.drawscope.Stroke(width = fontPx * 0.11f, join = androidx.compose.ui.graphics.StrokeJoin.Round))
    Box(Modifier.align(Alignment.BottomCenter).padding(bottom = with(density) { bottom.toDp() }).widthIn(max = with(density) { (sw * 0.86f).toDp() })) {
        BasicText(text, style = outline)
        BasicText(text, style = style)
    }
}

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
            delay(15000)
            session.save()
        }
    }

    // Position et cache relus seulement quand quelque chose les affiche : commandes masquées,
    // rien ne réveille l'interface pendant la lecture.
    // En pause, rien ne bouge : une lecture, puis plus rien (la fin d'un saut relit aussi).
    LaunchedEffect(controls || panel != null, mpv.paused) {
        if (!(controls || panel != null)) return@LaunchedEffect
        mpv.refresh()
        while (!mpv.paused) {
            delay(250)
            mpv.refresh()
        }
    }

    // L'écran passe à la cadence du film (24p sans saccades sur un écran 60/120 Hz).
    LaunchedEffect(mpv.fps, surfaceHolder) {
        val s = surfaceHolder?.surface ?: return@LaunchedEffect
        if (mpv.fps > 1 && s.isValid && Build.VERSION.SDK_INT >= 31) {
            runCatching { s.setFrameRate(mpv.fps.toFloat(), Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE, Surface.CHANGE_FRAME_RATE_ONLY_IF_SEAMLESS) }
        }
    }

    // Télécommande (ou clavier) : le focus part de l'écran entier, puis va aux commandes.
    val keys = LocalInputModeManager.current.inputMode == InputMode.Keyboard
    val rootFocus = remember { FocusRequester() }
    val playFocus = remember { FocusRequester() }
    LaunchedEffect(controls, panel, keys) {
        if (panel != null) return@LaunchedEffect
        delay(40)
        runCatching { if (controls && keys) playFocus.requestFocus() else rootFocus.requestFocus() }
    }
    fun seekBy(d: Double) {
        // Toujours sur une image clé (immédiat) : la bulle donne le saut réel.
        val from = session.from()
        val target = session.seekSpan(from, from + d)
        val real = target - from
        session.say("${if (real >= 0) "+" else "−"}${delta(abs(real))}  ·  ${clock(target)}")
    }
    fun onKey(e: KeyEvent): Boolean {
        if (e.type != KeyEventType.KeyDown) return false
        poke++
        when (e.key) {
            Key.MediaPlayPause -> { mpv.togglePause(); return true }
            Key.MediaPlay -> { mpv.pauseTo(false); return true }
            Key.MediaPause -> { mpv.pauseTo(true); return true }
            Key.MediaFastForward -> { seekBy(60.0); return true }
            Key.MediaRewind -> { seekBy(-60.0); return true }
            Key.MediaNext -> { session.stepSub(1); return true }
            Key.MediaPrevious -> { session.stepSub(-1); return true }
            Key.Menu -> { panel = if (panel == null) Panel.TRACKS else null; return true }
        }
        if (panel != null || controls) return false
        // Commandes masquées : OK met en pause, ←/→ sautent de 10 s, ↑/↓ montrent les commandes.
        return when (e.key) {
            Key.DirectionCenter, Key.Enter, Key.NumPadEnter, Key.Spacebar -> { mpv.togglePause(); controls = true; true }
            Key.DirectionLeft -> { seekBy(-10.0); true }
            Key.DirectionRight -> { seekBy(10.0); true }
            Key.DirectionUp, Key.DirectionDown -> { controls = true; true }
            else -> false
        }
    }

    CompositionLocalProvider(LocalPlates provides plates) {
    Box(Modifier.fillMaxSize().background(Color.Black).onPreviewKeyEvent(::onKey).focusRequester(rootFocus).focusable()) {
        // Mode direct : la surface a le format exact de la vidéo (le décodeur y dessine tel quel).
        // Par défaut, la partie utile de l'image (sans ses bandes noires incrustées) remplit
        // l'écran sans être rognée : seules restent les bandes dues au format de l'écran.
        // Le zoom et « remplir » agrandissent la surface depuis le centre.
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val sw = constraints.maxWidth.toFloat()
            val sh = constraints.maxHeight.toFloat()
            val a = mpv.aspect.toFloat().takeIf { it > 0f } ?: (16f / 9f)
            val (vw, vh) = if (sw / sh > a) sh * a to sh else sw to sw / a
            val c = session.content
            val cw = vw * (c.right - c.left)
            val ch = vh * (c.bottom - c.top)
            val base = (if (session.fill) maxOf(sw / cw, sh / ch) else minOf(sw / cw, sh / ch)).coerceAtLeast(1f)
            val z = base * session.zoom.toFloat()
            // Centre de la partie utile ramené au centre de l'écran.
            val dx = -((c.left + c.right) / 2f - 0.5f) * vw * z
            val dy = -((c.top + c.bottom) / 2f - 0.5f) * vh * z
            val density = LocalDensity.current
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
                modifier = Modifier.align(Alignment.Center)
                    .offset { IntOffset(dx.roundToInt(), dy.roundToInt()) }
                    .requiredSize(with(density) { (vw * z).toDp() }, with(density) { (vh * z).toDp() }),
            )
            if (panel == null) Subtitles(session, plates, sw, sh, ch * z)
        }

        val context = LocalContext.current
        Gestures(
            session,
            onBrightnessStart = { Brightness.start(context) { session.say(it) } },
            onBrightness = { level ->
                Brightness.set(context, level)
                session.say("${t("Luminosité", "Brightness")} ${(level * 100).roundToInt()} %")
            },
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
            Controls(session, onBack, playFocus, onPanel = { panel = it; poke++ }, onPoke = { poke++ })
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
    // À la télécommande, Retour masque d'abord les commandes.
    BackHandler(panel == null && controls && keys) { controls = false }
    // Tiroir ouvert : la lecture s'arrête, tout va à l'interface ; elle reprend à la fermeture
    // si elle tournait avant.
    var resumeAfterPanel by remember { mutableStateOf(false) }
    LaunchedEffect(panel != null) {
        if (panel != null) {
            resumeAfterPanel = !mpv.paused && mpv.loaded
            if (resumeAfterPanel) mpv.pauseTo(true)
        } else if (resumeAfterPanel) {
            resumeAfterPanel = false
            mpv.pauseTo(false)
        }
    }
}

/**
 * Un seul détecteur pour tout l'écran : toucher (contrôles, sans attendre un second toucher),
 * double toucher (lecture/pause), glissé horizontal par crans, pincement (zoom, déplacement).
 */
@Composable
private fun Gestures(
    session: Session,
    onBrightnessStart: () -> Float,
    onBrightness: (Float) -> Unit,
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
                val from = mpv.now()
                var dragging = false
                var pinching = false
                var bright = false
                var brightFrom = -1f
                var steps = 0
                var current: Scrub? = null
                while (true) {
                    val ev = awaitPointerEvent()
                    val pressed = ev.changes.count { it.pressed }
                    if (pressed == 0) break
                    if (pressed >= 2 && !dragging) pinching = true
                    if (pinching) {
                        // Pincement : zoom depuis le centre de l'écran, jamais décalé. Glissé à deux
                        // doigts sans pincer : déplacement de l'image zoomée.
                        // Pincement : zoom depuis le centre ; l'image reste toujours centrée.
                        val z = ev.calculateZoom()
                        if (abs(z - 1f) > 0.002f) session.zoomBy(z.toDouble())
                        ev.changes.forEach { it.consume() }
                        continue
                    }
                    val c = ev.changes.firstOrNull { it.id == down.id } ?: continue
                    val dx = c.position.x - down.position.x
                    val dy = c.position.y - down.position.y
                    // Glissé vertical sur le tiers gauche : luminosité du système.
                    if (!dragging && !bright && down.position.x < size.width / 3f && abs(dy) > viewConfiguration.touchSlop && abs(dy) > abs(dx) * 1.2f) {
                        bright = true
                        brightFrom = onBrightnessStart()
                    }
                    if (bright) {
                        c.consume()
                        if (brightFrom >= 0f) onBrightness((brightFrom - dy / (size.height * 0.8f)).coerceIn(0f, 1f))
                        continue
                    }
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
                    bright -> {}
                    dragging -> {
                        val s = current
                        if (s != null && s.steps != 0) {
                            when {
                                s.zone == Zone.LINE && s.cue == null -> session.stepSub(s.steps, from)
                                s.zone == Zone.LINE -> session.seekLine(s.target)
                                else -> session.seekSpan(from, s.target)
                            }
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
        modifier.widthIn(min = 220.dp, max = 560.dp).glass(RoundedCornerShape(28.dp), tint = 0.58f).padding(horizontal = 26.dp, vertical = 18.dp),
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
            modifier = Modifier.glass(Capsule, tint = 0.55f).padding(horizontal = 18.dp, vertical = 10.dp),
        )
    }
}

@Composable
private fun Controls(session: Session, onBack: () -> Unit, playFocus: FocusRequester, onPanel: (Panel) -> Unit, onPoke: () -> Unit) {
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
                .glass(RoundedCornerShape(30.dp), tint = 0.5f).padding(horizontal = 16.dp, vertical = 10.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Clock(session, Modifier.width(72.dp)) { it }
                SeekBar(session, Modifier.weight(1f), onPoke)
                Clock(session, Modifier.width(72.dp), TextAlign.End) { mpv.duration - it }
            }
            Row(Modifier.padding(top = 4.dp).fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                GlassButton({ session.stepSub(-1); onPoke() }, plain = true, size = 50.dp) { Icon(Icons.prevLine, size = 24.dp) }
                GlassButton({ mpv.togglePause(); onPoke() }, Modifier.focusRequester(playFocus), plain = true, size = 58.dp) {
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
            .focusRing(Capsule)
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
        modifier.height(36.dp)
            // Télécommande : barre sélectionnée, ←/→ avancent ou reculent de 10 s.
            .focusRing(RoundedCornerShape(12.dp))
            .onKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onKeyEvent false
                val d = when (e.key) { Key.DirectionLeft -> -10.0; Key.DirectionRight -> 10.0; else -> return@onKeyEvent false }
                val from = session.from()
                session.seekSpan(from, from + d)
                onPoke()
                true
            }
            .focusable()
            .pointerInput(session) {
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
        // Partie lue à plat, du bleu des contours de boutons ; le curseur seulement pendant le glissé.
        if (f > 0f) drawRoundRect(Ink.accent, Offset(0f, top), Size(maxOf(size.width * f, h), h), r)
        if (drag != null) drawCircle(Color.White, h * 1.3f, Offset(size.width * f, size.height / 2))
    }
}

/** Pourcentage arrondi, utile aux panneaux. */
fun pct(v: Double) = (v * 100).roundToInt()

/** « 10 s », « 1 min ». */
private fun delta(d: Double): String = if (d >= 60) "${(d / 60).roundToInt()} min" else "${d.roundToInt()} s"
