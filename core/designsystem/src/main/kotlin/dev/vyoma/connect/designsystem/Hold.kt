package dev.vyoma.connect.designsystem

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * [pressable] with a second job on a long press, held like water: after a beat the element swells
 * and turns into a liquid lens of itself (its own glyph and label bend through the rim, the way
 * Control Center modules lift before they expand), the lens filling as the hold nears its
 * timeout. When it fires it overshoots with a [Haptic.Hold] and, let go, wobbles back on
 * [VyomaMotion.release]. Let go early and it just settles; a tap stays a tap. Without
 * [onLongClick] it's a plain [pressable].
 *
 * The lens appears only while held (the user rejected lenses on plain taps: refraction over a
 * button's own label on every press read as glitchy), and costs nothing at rest.
 */
fun Modifier.liquidPress(
    enabled: Boolean = true,
    haptic: Haptic? = Haptic.Tap,
    onLongClickLabel: String? = null,
    onLongClick: (() -> Unit)? = null,
    onClick: () -> Unit,
): Modifier = if (onLongClick == null) pressable(enabled = enabled, haptic = haptic, onClick = onClick) else composed {
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    // 0 at rest, 1 when the hold fires; overshoots past 1 as it does.
    val hold = remember { Animatable(0f) }
    val click by rememberUpdatedState(onClick)
    val long by rememberUpdatedState(onLongClick)
    this
        .semantics {
            role = Role.Button
            onClick { click(); true }
            onLongClick(onLongClickLabel) { long?.invoke(); true }
        }
        .pressScale(enabled = enabled)
        .graphicsLayer {
            val h = hold.value.coerceAtLeast(0f)
            // Grows towards the finger's hold like a drop gathering; the lens does the rest.
            val s = 1f + 0.07f * h
            scaleX = s
            scaleY = s
        }
        .liquidLens(
            null,
            strength = { hold.value.coerceIn(0f, 1f) },
            zoom = { 1f + 0.1f * hold.value.coerceIn(0f, 1.2f) },
            style = LensStyle.Lift,
            rect = { Rect(Offset.Zero, size) },
        )
        .pointerInput(enabled) {
            if (!enabled) return@pointerInput
            detectTapGestures(
                onPress = {
                    val gather = scope.launch {
                        delay(HOLD_BEAT_MS)
                        hold.animateTo(1f, tween((viewConfiguration.longPressTimeoutMillis - HOLD_BEAT_MS).toInt().coerceAtLeast(120), easing = LinearOutSlowInEasing))
                    }
                    tryAwaitRelease()
                    gather.cancel()
                    scope.launch { hold.animateTo(0f, VyomaMotion.release()) }
                },
                onTap = {
                    haptic?.let(haptics::perform)
                    click()
                },
                onLongPress = {
                    haptics.perform(Haptic.Hold)
                    scope.launch { hold.animateTo(1.3f, VyomaMotion.spatial()) }
                    long?.invoke()
                },
            )
        }
}

/** How long a touch rests before the hold starts to show (a tap never swells). */
private const val HOLD_BEAT_MS = 120L
