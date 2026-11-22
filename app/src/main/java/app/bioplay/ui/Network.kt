package app.bioplay.ui

import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
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
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.bioplay.Library
import app.bioplay.Smb
import app.bioplay.t
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private sealed interface Step {
    data object Login : Step
    class Shares(val host: String, val names: List<String>) : Step
    class Browse(val dir: Uri, val dirs: List<String>) : Step
}

/**
 * Ajout d'un dossier réseau (disque de la box, NAS) : adresse et compte, puis le partage, puis
 * le dossier à surveiller. Tout se fait à la télécommande comme au doigt.
 */
@Composable
fun NetworkSheet(library: Library, onDone: () -> Unit, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences("bioplay", android.content.Context.MODE_PRIVATE) }
    val scope = rememberCoroutineScope()
    var step by remember { mutableStateOf<Step>(Step.Login) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var host by remember { mutableStateOf(prefs.getString("smb-host", null) ?: Smb.gateway(context) ?: "") }
    val saved = remember(host) { Smb.login(context, host) }
    var user by remember { mutableStateOf(saved?.user ?: "") }
    var password by remember { mutableStateOf(saved?.password ?: "") }
    var shareName by remember { mutableStateOf("") }

    fun work(block: suspend () -> Unit) {
        if (busy) return
        busy = true
        error = null
        scope.launch {
            try {
                block()
            } catch (e: Exception) {
                error = e.message ?: e.javaClass.simpleName
            } finally {
                busy = false
            }
        }
    }

    fun browse(dir: Uri) = work {
        val dirs = withContext(Dispatchers.IO) { Smb.list(context, dir).filter { it.dir && !it.name.startsWith('.') }.map { it.name }.sortedBy { it.lowercase() } }
        step = Step.Browse(dir, dirs)
    }

    fun connect() = work {
        val h = host.trim()
        val failed = withContext(Dispatchers.IO) { Smb.connect(context, h, Smb.Login(user.trim(), password)) }
        if (failed != null) {
            error = "${t("Connexion impossible", "Cannot connect")} : $failed"
            return@work
        }
        prefs.edit().putString("smb-host", h).apply()
        // Liste des partages ; si le serveur ne la donne pas, on tape le nom.
        val names = withContext(Dispatchers.IO) { runCatching { Smb.shares(context, h) }.getOrDefault(emptyList()) }
        step = Step.Shares(h, names)
    }

    Box(
        Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f))
            .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
    ) {
        Column(
            Modifier.align(Alignment.Center).padding(16.dp).widthIn(max = 560.dp).fillMaxWidth()
                .glass(RoundedCornerShape(28.dp), tint = 0.75f)
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .padding(horizontal = 20.dp, vertical = 16.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                BasicText(t("Partage réseau", "Network share"), style = text(17, FontWeight.SemiBold), modifier = Modifier.weight(1f))
                GlassButton(onDismiss, plain = true, size = 36.dp) { Icon(Icons.close, size = 18.dp, tint = Ink.secondary) }
            }
            when (val s = step) {
                Step.Login -> {
                    BasicText(
                        t("Disque de la box ou NAS (SMB). Freebox : activez le partage Windows dans Freebox OS et donnez son compte.",
                            "Box disk or NAS (SMB). Freebox: enable Windows sharing in Freebox OS and enter its account."),
                        style = text(12, color = Ink.secondary), modifier = Modifier.padding(top = 2.dp, bottom = 12.dp),
                    )
                    val first = remember { FocusRequester() }
                    Field(t("Adresse", "Address"), host, { host = it }, Modifier.focusRequester(first), KeyboardType.Uri)
                    Field(t("Utilisateur (vide : invité)", "User (empty: guest)"), user, { user = it })
                    Field(t("Mot de passe", "Password"), password, { password = it }, password = true, onDone = ::connect)
                    Spacer(Modifier.height(6.dp))
                    Action(if (busy) t("Connexion…", "Connecting…") else t("Se connecter", "Connect"), Icons.check, ::connect)
                    LaunchedEffect(Unit) { runCatching { first.requestFocus() } }
                }
                is Step.Shares -> {
                    Caption("${s.host} · ${t("choisissez un partage", "pick a share")}")
                    if (s.names.isEmpty()) {
                        Field(t("Nom du partage", "Share name"), shareName, { shareName = it }, onDone = { browse(Smb.uri(s.host, shareName.trim())) })
                        Action(t("Ouvrir", "Open"), Icons.check) { browse(Smb.uri(s.host, shareName.trim())) }
                    } else {
                        LazyColumn(Modifier.heightIn(max = 300.dp)) {
                            items(s.names) { n -> Entry(n, Icons.folder) { browse(Smb.uri(s.host, n)) } }
                        }
                    }
                    Back { step = Step.Login }
                }
                is Step.Browse -> {
                    val segs = s.dir.pathSegments
                    Caption("${s.dir.encodedAuthority}/${segs.joinToString("/")}")
                    LazyColumn(Modifier.heightIn(max = 280.dp)) {
                        if (s.dirs.isEmpty()) item { BasicText(t("Aucun sous-dossier", "No subfolder"), style = text(13, color = Ink.faint), modifier = Modifier.padding(12.dp)) }
                        items(s.dirs) { n -> Entry(n, Icons.folder) { browse(s.dir.buildUpon().appendPath(n).build()) } }
                    }
                    Spacer(Modifier.height(8.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                        Action(t("Surveiller ce dossier", "Watch this folder"), Icons.check) {
                            library.addFolder(s.dir)
                            onDone()
                        }
                        Back {
                            val up = segs.dropLast(1)
                            if (up.isEmpty()) work { step = Step.Shares(s.dir.encodedAuthority.orEmpty(), withContext(Dispatchers.IO) { runCatching { Smb.shares(context, s.dir.encodedAuthority.orEmpty()) }.getOrDefault(emptyList()) }) }
                            else browse(Smb.uri(s.dir.encodedAuthority.orEmpty(), up.first(), up.drop(1).joinToString("/")))
                        }
                    }
                }
            }
            error?.let { BasicText(it, maxLines = 3, overflow = TextOverflow.Ellipsis, style = text(12, color = Color(0xFFFF9A8F)), modifier = Modifier.padding(top = 10.dp)) }
            if (busy && step !is Step.Login) BasicText("…", style = text(14, color = Ink.secondary), modifier = Modifier.padding(top = 6.dp))
        }
    }
}

@Composable
private fun Caption(s: String) =
    BasicText(s, maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(12, color = Ink.secondary), modifier = Modifier.padding(top = 2.dp, bottom = 10.dp))

@Composable
private fun Field(
    label: String,
    value: String,
    onChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    type: KeyboardType = KeyboardType.Text,
    password: Boolean = false,
    onDone: (() -> Unit)? = null,
) {
    Column(Modifier.padding(bottom = 10.dp)) {
        BasicText(label, style = text(12, FontWeight.SemiBold, Ink.secondary), modifier = Modifier.padding(start = 4.dp, bottom = 4.dp))
        val shape = RoundedCornerShape(14.dp)
        BasicTextField(
            value, onChange,
            modifier.focusRing(shape).fillMaxWidth().clip(shape).background(Color.White.copy(alpha = 0.08f))
                .border(1.dp, Color.White.copy(alpha = 0.18f), shape).padding(horizontal = 14.dp, vertical = 11.dp),
            singleLine = true,
            textStyle = text(15),
            cursorBrush = SolidColor(Ink.accent),
            visualTransformation = if (password) PasswordVisualTransformation() else VisualTransformation.None,
            keyboardOptions = KeyboardOptions(
                keyboardType = if (password) KeyboardType.Password else type,
                imeAction = if (onDone != null) ImeAction.Done else ImeAction.Next,
                autoCorrectEnabled = false,
            ),
            keyboardActions = androidx.compose.foundation.text.KeyboardActions(onDone = { onDone?.invoke() }),
        )
    }
}

@Composable
private fun Entry(name: String, icon: androidx.compose.ui.graphics.vector.ImageVector, onClick: () -> Unit) {
    val shape = RoundedCornerShape(14.dp)
    Row(
        Modifier.fillMaxWidth().padding(vertical = 2.dp).focusRing(shape).clip(shape).clickable(onClick = onClick).padding(horizontal = 12.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, size = 20.dp, tint = Ink.accent)
        BasicText(name, maxLines = 1, overflow = TextOverflow.Ellipsis, style = text(15, FontWeight.SemiBold), modifier = Modifier.padding(start = 12.dp))
    }
}

@Composable
private fun Action(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, onClick: () -> Unit) {
    GlassButton(onClick, shape = Capsule, size = 46.dp) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(icon, size = 19.dp)
            BasicText(label, style = text(14, FontWeight.SemiBold), modifier = Modifier.padding(start = 8.dp, end = 4.dp))
        }
    }
}

@Composable
private fun Back(onClick: () -> Unit) {
    GlassButton(onClick, plain = true, shape = Capsule, size = 46.dp) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.back, size = 19.dp, tint = Ink.secondary)
            BasicText(t("Retour", "Back"), style = text(14, FontWeight.SemiBold, Ink.secondary), modifier = Modifier.padding(start = 6.dp, end = 4.dp))
        }
    }
}
