package app.bioplay

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.util.Locale

/** Interface en français ou en anglais : langue du système par défaut, bascule d'un toucher. */
object Lang {
    var en by mutableStateOf(false)
        private set

    fun init(context: Context) {
        val saved = context.getSharedPreferences("bioplay", Context.MODE_PRIVATE).getString("lang", null)
        en = saved?.let { it == "en" } ?: (Locale.getDefault().language != "fr")
    }

    fun toggle(context: Context) {
        en = !en
        context.getSharedPreferences("bioplay", Context.MODE_PRIVATE).edit().putString("lang", if (en) "en" else "fr").apply()
    }

    val locale: Locale get() = if (en) Locale.ENGLISH else Locale.FRENCH
}

/** Le texte dans la langue choisie. */
fun t(fr: String, en: String) = if (Lang.en) en else fr
