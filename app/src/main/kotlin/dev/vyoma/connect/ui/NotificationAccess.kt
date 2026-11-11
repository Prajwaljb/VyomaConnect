package dev.vyoma.connect.ui

import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LifecycleResumeEffect
import dev.vyoma.connect.designsystem.DialogButton
import dev.vyoma.connect.designsystem.GlassDialog
import dev.vyoma.connect.platform.NotificationBridge
import dev.vyoma.connect.platform.RestrictedSettings

/**
 * Opens Android's notification access page for Vyoma. Coming back with it still off, on a phone
 * where Android holds it back from an app installed from a file ([RestrictedSettings]), says how
 * to allow it; once allowed in App info, the access page opens again by itself.
 */
@Composable
fun rememberNotificationAccess(): () -> Unit {
    val context = LocalContext.current
    var asked by rememberSaveable { mutableStateOf(false) }
    var allowing by rememberSaveable { mutableStateOf(false) }
    var help by rememberSaveable { mutableStateOf(false) }
    fun open() {
        // Turning it off from there isn't something to help with.
        asked = !NotificationBridge.isEnabled(context)
        runCatching { context.startActivity(NotificationBridge.settingsIntent(context)) }
    }
    LifecycleResumeEffect(Unit) {
        val (wasAsked, wasAllowing) = asked to allowing
        asked = false
        allowing = false
        when {
            NotificationBridge.isEnabled(context) -> Unit
            // Back from App info: straight on to the switch (still blocked, Android says so and this asks again).
            wasAllowing -> open()
            wasAsked && RestrictedSettings.mayBlock(context) -> help = true
        }
        onPauseOrDispose { }
    }
    if (help) {
        GlassDialog(
            onDismissRequest = { help = false },
            title = { Text("Android is holding this back") },
            text = {
                Text(
                    "Android blocks this setting for apps installed from a file. To allow it:\n\n" +
                        "1. Tap Open App info.\n" +
                        "2. Tap ⋮ at the top right, then Allow restricted settings.\n" +
                        "3. Come back here: notification access opens again.",
                )
            },
            confirmButton = {
                DialogButton(onClick = {
                    help = false
                    allowing = true
                    runCatching { context.startActivity(RestrictedSettings.appInfo(context)) }
                }) { Text("Open App info") }
            },
            dismissButton = { DialogButton(onClick = { help = false }) { Text("Not now") } },
        )
    }
    return ::open
}
