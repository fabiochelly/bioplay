package app.bioplay.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import app.bioplay.t

/**
 * Luminosité du système (le vrai réglage du téléphone, pas un réglage propre à l'app).
 * Android demande pour cela l'autorisation « Modifier les paramètres système », accordée une
 * fois depuis la page des réglages que l'on ouvre au premier glissé.
 */
object Brightness {
    /** Niveau actuel (0..1), ou -1 si l'autorisation manque (la page des réglages s'ouvre). */
    fun start(context: Context, say: (String) -> Unit): Float {
        if (!Settings.System.canWrite(context)) {
            say(t("Autorisez bioplay à régler la luminosité", "Allow bioplay to change brightness"))
            context.startActivity(
                Intent(Settings.ACTION_MANAGE_WRITE_SETTINGS, Uri.parse("package:" + context.packageName))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            )
            return -1f
        }
        val r = context.contentResolver
        // En mode automatique, Android remplacerait aussitôt la valeur : passage en manuel.
        if (Settings.System.getInt(r, Settings.System.SCREEN_BRIGHTNESS_MODE, 0) == Settings.System.SCREEN_BRIGHTNESS_MODE_AUTOMATIC) {
            Settings.System.putInt(r, Settings.System.SCREEN_BRIGHTNESS_MODE, Settings.System.SCREEN_BRIGHTNESS_MODE_MANUAL)
            say(t("Luminosité automatique désactivée", "Adaptive brightness turned off"))
        }
        return Settings.System.getInt(r, Settings.System.SCREEN_BRIGHTNESS, 128) / 255f
    }

    fun set(context: Context, level: Float) {
        if (!Settings.System.canWrite(context)) return
        Settings.System.putInt(context.contentResolver, Settings.System.SCREEN_BRIGHTNESS, (level * 254 + 1).toInt().coerceIn(1, 255))
    }
}
