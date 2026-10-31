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
    onLongClickLabel: String? = null,
    onLongClick: (() -> Unit)? = null,
    onClick: () -> Unit,
): Modifier = if (onLongClick == null) pressable(enabled = enabled, haptic = haptic, onClick = onClick) else composed {
    val haptics = LocalHapticFeedback.current
    val click by rememberUpdatedState(onClick)
    val long by rememberUpdatedState(onLongClick)
    this
        .semantics {
            role = Role.Button
            onClick { click(); true }
            onLongClick(onLongClickLabel) { long?.invoke(); true }
        }
        .pressScale(enabled = enabled)
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
