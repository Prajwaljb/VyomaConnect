package dev.vyoma.connect.ui

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Mouse
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.container
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.designsystem.VyomaSwitch
import dev.vyoma.connect.designsystem.VyomaTheme
import dev.vyoma.connect.mirror.MirrorAccessibility
import dev.vyoma.connect.mirror.PhoneMirror
import dev.vyoma.connect.service.LinkService
import kotlinx.coroutines.flow.MutableStateFlow

/**
 * This phone's screen in a window on the PC, worked from there with the mouse and keyboard. Start
 * asks Android to share the screen (every time; Android's rule), then it runs with this screen
 * closed until Stop, the notification's Stop sharing, or the PC's window is closed. Opened from the
 * Screen tile's long-press menu on Home.
 */
class MirrorActivity : ComponentActivity() {
    /** Whether the PC can work the phone (the accessibility service), checked on every return here. */
    private val control = MutableStateFlow(false)

    override fun onResume() {
        super.onResume()
        control.value = MirrorAccessibility.enabled(this)
    }

    private val prefs by lazy { getSharedPreferences("mirror", MODE_PRIVATE) }

    private fun granted(p: String) = checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    private fun setSound(on: Boolean) {
        prefs.edit().putBoolean("sound", on).apply()
        PhoneMirror.sound.value = on
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        PhoneMirror.sound.value = prefs.getBoolean("sound", true)
        setContent {
            val theme by application.container.settings.theme.collectAsStateWithLifecycle(ThemeSettings())
            val status by PhoneMirror.status.collectAsStateWithLifecycle()
            val canControl by control.collectAsStateWithLifecycle()
            val consent = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
                val data = r.data
                if (r.resultCode == Activity.RESULT_OK && data != null) LinkService.mirror(this, r.resultCode, data)
            }
            val sound by PhoneMirror.sound.collectAsStateWithLifecycle()
            // Sound needs the record-audio permission (Android counts capturing playback as recording).
            var startAfterAsking by remember { mutableStateOf(false) }
            val askAudio = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
                if (!ok) setSound(false)
                // Only when it was Start that asked, not the Sound switch.
                if (startAfterAsking && !status.on) consent.launch(PhoneMirror.consentIntent(this))
                startAfterAsking = false
            }
            fun start() {
                if (sound && !granted(Manifest.permission.RECORD_AUDIO)) {
                    startAfterAsking = true
                    askAudio.launch(Manifest.permission.RECORD_AUDIO)
                }
                else consent.launch(PhoneMirror.consentIntent(this))
            }
            fun openAccessibility() = startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            VyomaTheme(theme) {
                CollapsingHeaderScaffold(
                    title = "Phone screen on PC",
                    subtitle = subtitle(status),
                    navigation = { BounceIconButton(onClick = ::finish) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
                ) {
                    item(key = "go") {
                        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            PillButton(
                                if (status.on) "Stop" else "Show on the PC",
                                onClick = {
                                    if (status.on) PhoneMirror.stop(this@MirrorActivity)
                                    else start()
                                },
                                modifier = Modifier.fillMaxWidth(),
                                prominent = !status.on,
                                large = true,
                            )
                        }
                    }
                    status.error?.takeIf { !status.on }?.let { err ->
                        item(key = "error") {
                            SectionCard(Modifier.padding(top = 8.dp)) {
                                ListRow(title = "Couldn't show the screen", subtitle = plainReason(err) ?: "Try again, or restart Vyoma on the PC", icon = Icons.Rounded.ErrorOutline, iconTint = MaterialTheme.colorScheme.error)
                            }
                        }
                    }
                    item(key = "sound-label") { SectionLabel("Sound") }
                    item(key = "sound") {
                        SectionCard(Modifier.padding(top = 8.dp)) {
                            fun toggle(on: Boolean) {
                                if (on && !granted(Manifest.permission.RECORD_AUDIO)) {
                                    setSound(true)
                                    askAudio.launch(Manifest.permission.RECORD_AUDIO)
                                } else setSound(on)
                            }
                            ListRow(
                                title = "Play the phone's sound on the PC",
                                subtitle = "Videos, music and games; calls and apps that block recording stay on the phone",
                                icon = Icons.AutoMirrored.Rounded.VolumeUp,
                                trailing = { VyomaSwitch(sound, { toggle(it) }, contentDescription = "Phone sound on the PC") },
                                onClick = { toggle(!sound) },
                            )
                        }
                    }
                    item(key = "control-label") { SectionLabel("Control") }
                    item(key = "control") {
                        SectionCard(Modifier.padding(top = 8.dp)) {
                            ListRow(
                                title = "Mouse and keyboard from the PC",
                                subtitle = if (canControl) "On: click, drag, scroll and type in the PC's window"
                                else "Off: the PC can only watch. Turn on \"Vyoma PC control\" in Accessibility " +
                                    "(if it says restricted: App info > ⋮ > Allow restricted settings)",
                                icon = Icons.Rounded.Mouse,
                                trailing = { VyomaSwitch(canControl, { openAccessibility() }, contentDescription = "Control from the PC") },
                                onClick = ::openAccessibility,
                            )
                        }
                    }
                    item(key = "tip") {
                        Text(
                            "In the PC's window, click to tap and drag to swipe; the wheel scrolls. Typing goes into the " +
                                "field you tapped, and Ctrl+V pastes the PC's clipboard there. Right-click or Esc is Back, middle-click " +
                                "is Home. Close the window to stop.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp),
                        )
                    }
                }
            }
        }
    }

    companion object {
        fun open(context: Context) = context.startActivity(Intent(context, MirrorActivity::class.java))
    }
}

private fun subtitle(s: PhoneMirror.Status): String = when {
    !s.on -> "Off"
    s.pc == null -> "Waiting for Vyoma on your PC"
    s.open -> "Showing on ${s.pc}"
    else -> "Opening on ${s.pc}…"
}
