package dev.vyoma.connect.designsystem

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Error
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** One outcome to confirm: "Running Suspend", "Copied", "Couldn't reach the PC". */
class StatusMessage internal constructor(val id: Long, val text: String, val icon: ImageVector?, val failure: Boolean, val haptic: Boolean)

/** Shows outcomes in the status bubble; get it from [LocalStatus]. */
class StatusController {
    internal var current by mutableStateOf<StatusMessage?>(null)
    private var seq = 0L

    /** [haptic] false when the caller already played one (one haptic per action). */
    fun show(text: String, icon: ImageVector? = null, failure: Boolean = false, haptic: Boolean = true) {
        current = StatusMessage(++seq, text, icon, failure, haptic)
    }
}

val LocalStatus = staticCompositionLocalOf { StatusController() }

/**
 * Confirms outcomes in place, inside the design: a glass capsule drops from the top where the
 * header's bubbles live, swells out like the title bubble (height first, width on jelly), holds
 * 1.6 s and lifts away. Replaces toasts, which sat outside the app's look. A new message replaces
 * the one showing.
 */
@Composable
fun StatusHost(content: @Composable () -> Unit) {
    val controller = remember { StatusController() }
    // The app is recorded as the bubble's backdrop (replaying a recorded list costs nothing extra),
    // so it's the same liquid glass as the tab bar.
    val backdrop = rememberBackdrop()
    CompositionLocalProvider(LocalStatus provides controller) {
        Box(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize().backdropSource(backdrop)) { content() }
            StatusBubble(controller, backdrop)
        }
    }
}

@Composable
private fun BoxScope.StatusBubble(c: StatusController, backdrop: Backdrop) {
    val haptics = LocalHapticFeedback.current
    val drop = remember { Animatable(0f) }
    val spread = remember { Animatable(0f) }
    var shown by remember { mutableStateOf<StatusMessage?>(null) }
    val msg = c.current
    LaunchedEffect(msg?.id) {
        msg ?: return@LaunchedEffect
        shown = msg
        if (msg.haptic) haptics.perform(if (msg.failure) Haptic.Failure else Haptic.Success)
        launch {
            spread.snapTo(0.4f)
            spread.animateTo(1f, VyomaMotion.jelly())
        }
        drop.animateTo(1f, VyomaMotion.spatial())
        // Long enough to read: about 15 characters a second past what fits on one line.
        val extra = (msg.text.length - ONE_LINE_CHARS).coerceAtLeast(0) * 66L
        delay(((if (msg.failure) 2600L else 1600L) + extra).coerceAtMost(6000L))
        drop.animateTo(0f, VyomaMotion.page())
        shown = null
        if (c.current?.id == msg.id) c.current = null
    }
    val m = shown ?: return
    Row(
        Modifier
            .align(Alignment.TopCenter)
            .windowInsetsPadding(WindowInsets.statusBars)
            // Between the header's side buttons (16 dp gutter + 48 dp circle + 8), never over them.
            .padding(top = 6.dp, start = 72.dp, end = 72.dp)
            .graphicsLayer {
                val p = drop.value
                translationY = (p - 1f) * 72.dp.toPx()
                alpha = p.coerceIn(0f, 1f)
                scaleX = spread.value
                scaleY = 0.8f + 0.2f * p.coerceIn(0f, 1f)
            }
            .semantics { liveRegion = LiveRegionMode.Polite }
            // Clear liquid glass like the tab bar. Not `legible`: its toned face read as a grey slab
            // (user, 2026-11-07). The corners make one line a capsule; longer messages grow downwards.
            .glassChrome(backdrop, RoundedCornerShape(24.dp), blur = 2.5.dp, bevel = 16.dp)
            .heightIn(min = 48.dp)
            .widthIn(min = 120.dp)
            .padding(start = 14.dp, end = 20.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val tone = if (m.failure) SystemColor.RED.color else SystemColor.GREEN.color
        Icon(m.icon ?: if (m.failure) Icons.Rounded.Error else Icons.Rounded.CheckCircle, null, tint = tone, modifier = Modifier.size(22.dp))
        Spacer(Modifier.width(10.dp))
        Text(m.text, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface, maxLines = 4, overflow = TextOverflow.Ellipsis)
    }
}

private const val ONE_LINE_CHARS = 24
