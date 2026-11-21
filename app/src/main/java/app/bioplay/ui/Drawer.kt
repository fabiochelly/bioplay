package app.bioplay.ui

import android.app.Dialog
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionContext
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCompositionContext
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import app.bioplay.R
import java.util.function.Consumer

private const val RADIUS_DP = 30f
private const val BLUR_DP = 34f

/**
 * Tiroir de verre : une fenêtre à part, posée en bas de l'écran, que le compositeur floute en
 * temps réel sur la vidéo (Android 12+, flou entre fenêtres). Si le flou est coupé (économie
 * d'énergie, appareil qui ne le gère pas), la teinte s'épaissit pour rester lisible.
 */
private class DrawerWindow(
    activity: ComponentActivity,
    parent: CompositionContext,
    width: Dp,
    content: @Composable () -> Unit,
) : Dialog(activity, R.style.Theme_Bioplay_Drawer) {
    private val density = activity.resources.displayMetrics.density
    private val background = GradientDrawable().apply { cornerRadius = RADIUS_DP * density }
    private val blurListener = Consumer<Boolean> { applyBlur(it) }

    init {
        val view = ComposeView(activity).apply {
            setParentCompositionContext(parent)
            setContent(content)
        }
        setContentView(view)
        setCanceledOnTouchOutside(true)
        window!!.apply {
            decorView.setViewTreeLifecycleOwner(activity)
            decorView.setViewTreeSavedStateRegistryOwner(activity)
            setBackgroundDrawable(background)
            val screen = activity.resources.displayMetrics.widthPixels
            setLayout(minOf((width.value * density).toInt(), screen - (32 * density).toInt()), ViewGroup.LayoutParams.WRAP_CONTENT)
            setGravity(Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL)
            attributes = attributes.apply { y = (14 * density).toInt() }
            clearFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND)
            addFlags(WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN)
            attributes.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
        }
    }

    private fun applyBlur(enabled: Boolean) {
        window?.setBackgroundBlurRadius(if (enabled) (BLUR_DP * density).toInt() else 0)
        background.setColor(if (enabled) 0x8C0E1116.toInt() else 0xEB0E1116.toInt())
    }

    override fun onStart() {
        super.onStart()
        val wm = context.getSystemService(WindowManager::class.java)
        applyBlur(wm.isCrossWindowBlurEnabled)
        wm.addCrossWindowBlurEnabledListener(blurListener)
        window?.insetsController?.apply {
            hide(WindowInsets.Type.systemBars())
            systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }

    override fun onStop() {
        context.getSystemService(WindowManager::class.java).removeCrossWindowBlurEnabledListener(blurListener)
        super.onStop()
    }
}

@Composable
fun GlassDrawer(onDismiss: () -> Unit, width: Dp = 620.dp, content: @Composable ColumnScope.() -> Unit) {
    val activity = LocalContext.current as ComponentActivity
    val parent = rememberCompositionContext()
    val body by rememberUpdatedState(content)
    val dismiss by rememberUpdatedState(onDismiss)
    DisposableEffect(Unit) {
        val window = DrawerWindow(activity, parent, width) {
            Column(Modifier.fillMaxWidth().glass(androidx.compose.foundation.shape.RoundedCornerShape(RADIUS_DP.dp), tint = 0f).padding(20.dp)) { body() }
        }
        window.setOnCancelListener { dismiss() }
        window.show()
        onDispose { window.dismiss() }
    }
}
