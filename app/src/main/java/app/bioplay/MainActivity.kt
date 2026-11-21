package app.bioplay

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
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
import androidx.compose.runtime.LaunchedEffect
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
    private val pickSubtitle = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri?.let { session.addSubtitle(it) }
    }

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
        session = Session(this)

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
            if (playing) {
                PlayerScreen(session, onBack = ::leavePlayer, onAddSubtitle = { pickSubtitle.launch(arrayOf("*/*")) })
            } else {
                val recents = remember(homeVersion) { session.recents() }
                HomeScreen(
                    recents,
                    onOpen = { pickVideo.launch(arrayOf("video/*", "application/x-matroska")) },
                    onResume = { r -> fromOutside = false; play(Uri.parse(r.uri)) },
                    onForget = { r -> session.forget(r.uri); homeVersion++ },
                )
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
        val uri = intent?.data ?: return
        if (intent.action != Intent.ACTION_VIEW) return
        fromOutside = true
        play(uri)
    }

    private fun play(uri: Uri) {
        playing = true
        session.open(uri)
    }

    private fun leavePlayer() {
        if (!playing) return finish()
        session.save()
        session.mpv.stop()
        audio.abandonAudioFocusRequest(focus)
        if (fromOutside) finish() else {
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
        super.onDestroy()
    }
}
