package dev.vyoma.connect.designsystem

import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics

/**
 * [pressable] with a second job on a long press, usually opening a menu ([MenuHost]) that pours out of
 * the element. The element itself only dips like any press: a lens over the button (tried) didn't
 * match its shape and the user rejected it; the liquid glass belongs to the menu. Without
 * [onLongClick] it's a plain [pressable].
 */
fun Modifier.liquidPress(
    enabled: Boolean = true,
    haptic: Haptic? = Haptic.Tap,
    /** As [pressable]'s: 1f for glass that flexes by itself (interactive chrome) instead of dipping. */
    pressedScale: Float = PressAuto,
    onLongClickLabel: String? = null,
    onLongClick: (() -> Unit)? = null,
    onClick: () -> Unit,
): Modifier = if (onLongClick == null) pressable(enabled = enabled, pressedScale = pressedScale, haptic = haptic, onClick = onClick) else composed {
    val haptics = LocalHapticFeedback.current
    val click by rememberUpdatedState(onClick)
    val long by rememberUpdatedState(onLongClick)
    this
        .semantics {
            role = Role.Button
            onClick { click(); true }
            onLongClick(onLongClickLabel) { long?.invoke(); true }
        }
        .pressScale(pressedScale, enabled)
        .pointerInput(enabled) {
            if (!enabled) return@pointerInput
            detectTapGestures(
                onTap = {
                    haptic?.let(haptics::perform)
                    click()
                },
                // The menu plays the Hold haptic as it forms.
                onLongPress = { long?.invoke() },
            )
        }
}
