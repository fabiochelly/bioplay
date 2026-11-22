package app.bioplay

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ActivityInfo
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.net.Uri
import android.os.Bundle
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import app.bioplay.ui.HomeScreen
import app.bioplay.ui.PlayerScreen
import app.bioplay.ui.loadFonts

class MainActivity : ComponentActivity() {
    private lateinit var session: Session
    private lateinit var library: Library
    private var playing by mutableStateOf(false)
    private var homeVersion by mutableIntStateOf(0)
    /** Ouverte par « Ouvrir avec » : le retour rend la main à l'app d'origine. */
    private var fromOutside = false

    private val pickVideo = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri ?: return@registerForActivityResult
        runCatching { contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION) }
        fromOutside = false
        play(uri)
    }
    private val pickFolder = registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        uri?.let { library.addFolder(it) }
    }
    private val pickSubtitle = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri?.let { session.addSubtitle(it) }
    }

    /** Sélecteurs de fichiers du système : absents sur la plupart des TV. */
    private val canPickFile by lazy {
        Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("video/*").resolveActivity(packageManager) != null
    }
    private val canPickFolder by lazy { Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).resolveActivity(packageManager) != null }

    private val audio by lazy { getSystemService(AudioManager::class.java) }
    private val focus by lazy {
        AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MOVIE).build())
            .setOnAudioFocusChangeListener { change ->
                if (change == AudioManager.AUDIOFOCUS_LOSS || change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) session.mpv.pauseTo(true)
            }
            .build()
    }
    /** Casque débranché : pause, comme partout ailleurs. */
    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) = session.mpv.pauseTo(true)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.setDecorFitsSystemWindows(false)
        window.attributes.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
        loadFonts(assets)
        Lang.init(this)
        app.bioplay.ui.Flags.load(this)
        session = Session(this)
        library = Library(this)
        library.start()

        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() = leavePlayer()
        })

        setContent {
            val mpv = session.mpv
            LaunchedEffect(mpv.paused, playing) {
                val on = playing && !mpv.paused
                if (on) {
                    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    audio.requestAudioFocus(focus)
                } else {
                    window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    if (playing) session.save()
                }
            }
            // Défilement gardé pendant la lecture : on revient là où l'on était.
            // Un défilement par dossier : on retrouve sa place en remontant.
            var place by remember { mutableStateOf(Place()) }
            val grids = remember { HashMap<Place, LazyGridState>() }
            val lists = remember { HashMap<Place, LazyListState>() }
            if (playing) {
                PlayerScreen(session, onBack = ::leavePlayer, onAddSubtitle = { pickSubtitle.launch(arrayOf("*/*")) })
            } else {
                // Relu au retour du lecteur : barres d'avancée à jour.
                key(homeVersion) {
                    HomeScreen(
                        library, place, { place = it },
                        { p -> grids.getOrPut(p) { LazyGridState() } },
                        { p -> lists.getOrPut(p) { LazyListState() } },
                        onOpen = if (canPickFile) ({ pickVideo.launch(arrayOf("video/*", "application/x-matroska")) }) else null,
                        onAddFolder = if (canPickFolder) ({ pickFolder.launch(null) }) else null,
                        onPlay = { v -> fromOutside = false; play(Uri.parse(v.uri)) },
                    )
                }
            }
        }
        immersive()
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    private fun handle(intent: Intent?) {
        // Essai d'un partage réseau sans interface (versions de développement seulement) :
        // am start -n app.bioplay/.MainActivity --es smbtest "hôte:port|utilisateur|mot de passe|smb://hôte:port/partage/dossier"
        if (BuildConfig.DEBUG) intent?.getStringExtra("smbtest")?.split('|')?.takeIf { it.size == 4 }?.let { (host, user, pass, folder) ->
            Thread {
                val err = Smb.connect(this, host, Smb.Login(user, pass))
                android.util.Log.i("bioplay", "smbtest connect=${err ?: "ok"} shares=${runCatching { Smb.shares(this, host) }.getOrElse { it.toString() }}")
                runOnUiThread { library.addFolder(Uri.parse(folder)) }
            }.start()
            return
        }
        val uri = intent?.data ?: return
        if (intent.action != Intent.ACTION_VIEW) return
        fromOutside = true
        play(uri)
    }

    private fun play(uri: Uri) {
        // Seule la lecture est en paysage forcé ; l'accueil suit le téléphone.
        requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        library.hold = true
        playing = true
        session.open(uri)
    }

    private fun leavePlayer() {
        if (!playing) return finish()
        session.save()
        session.mpv.stop()
        audio.abandonAudioFocusRequest(focus)
        library.hold = false
        if (fromOutside) finish() else {
            requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_FULL_USER
            playing = false
            homeVersion++
        }
    }

    private fun immersive() {
        window.insetsController?.apply {
            hide(WindowInsets.Type.systemBars())
            systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) immersive()
    }

    override fun onStart() {
        super.onStart()
        registerReceiver(noisy, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY), RECEIVER_NOT_EXPORTED)
    }

    override fun onStop() {
        unregisterReceiver(noisy)
        if (playing) {
            session.mpv.pauseTo(true)
            session.save()
        }
        super.onStop()
    }

    override fun onDestroy() {
        audio.abandonAudioFocusRequest(focus)
        session.close()
        library.close()
        super.onDestroy()
    }
}
