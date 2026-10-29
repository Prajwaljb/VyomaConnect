package dev.vyoma.connect.ui

import android.app.Activity
import android.app.PendingIntent
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.KeyEvent
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import dev.vyoma.connect.container
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.designsystem.VyomaTheme
import dev.vyoma.connect.platform.Ringer
import dev.vyoma.connect.protocol.plugin.builtin.ClipboardPlugin
import dev.vyoma.connect.service.Commands
import dev.vyoma.connect.service.LinkService
import dev.vyoma.connect.service.Notifications
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Invisible: takes focus for a moment so Android lets us read the clipboard, sends it to every
 * connected PC, and closes. Used by the tile, widget, shortcut and notification button.
 */
class ClipboardActivity : Activity() {
    private var done = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || done) return
        done = true
        val c = application.container
        c.clipboard.readNow()
        val text = c.clipboard.current.value?.text
        if (text.isNullOrBlank()) {
            Toast.makeText(this, "Clipboard is empty", Toast.LENGTH_SHORT).show()
            finish()
            return
        }
        val targets = c.hub.value?.devices?.value?.values?.filter { it.state.value.isPaired && it.state.value.isReachable }.orEmpty()
        if (targets.isEmpty()) {
            Toast.makeText(this, "No PC connected", Toast.LENGTH_SHORT).show()
        } else {
            c.appScope.launch { targets.forEach { it.plugin<ClipboardPlugin>(ClipboardPlugin.ID)?.push(text) } }
            Toast.makeText(this, "Clipboard sent to ${targets.joinToString { it.name }}", Toast.LENGTH_SHORT).show()
        }
        finish()
    }

    companion object {
        fun intent(context: Context) = Intent(context, ClipboardActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION or Intent.FLAG_ACTIVITY_EXCLUDE_FROM_RECENTS)

        fun pending(context: Context): PendingIntent =
            PendingIntent.getActivity(context, 41, intent(context), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
}

/**
 * Share target ("Send to PC" in the share sheet) and the file picker for "Send files".
 * Work is handed to [LinkService] together with the URI grants, so it outlives this screen.
 */
class ShareActivity : ComponentActivity() {
    private val picker = registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) forward(uris, null)
        finish()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (savedInstanceState != null) return
        when (intent.action) {
            ACTION_PICK -> picker.launch(arrayOf("*/*"))
            Intent.ACTION_SEND -> {
                @Suppress("DEPRECATION")
                val stream = intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
                val text = intent.getStringExtra(Intent.EXTRA_TEXT)
                if (stream != null) forward(listOf(stream), null) else if (text != null) forward(emptyList(), text)
                finish()
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                @Suppress("DEPRECATION")
                val streams = intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM).orEmpty()
                forward(streams, null)
                finish()
            }
            else -> finish()
        }
    }

    private fun forward(uris: List<Uri>, text: String?) {
        val c = application.container
        val target = Commands.primary(c)
        if (target == null) {
            Toast.makeText(this, "No PC connected", Toast.LENGTH_SHORT).show()
            return
        }
        val service = Intent(this, LinkService::class.java).setAction(LinkService.ACTION_SEND)
            .putExtra(LinkService.EXTRA_DEVICE, target.id)
        if (text != null) service.putExtra(Intent.EXTRA_TEXT, text)
        if (uris.isNotEmpty()) {
            val clip = ClipData.newRawUri(null, uris.first())
            uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) }
            service.clipData = clip
            service.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        startForegroundService(service)
        val what = when {
            uris.size == 1 -> "file"
            uris.size > 1 -> "${uris.size} files"
            text != null && LinkService.looksLikeUrl(text) -> "link"
            else -> "text"
        }
        Toast.makeText(this, "Sending $what to ${target.name}", Toast.LENGTH_SHORT).show()
    }

    companion object {
        const val ACTION_PICK = "dev.vyoma.connect.PICK_FILES"

        fun pickFilesIntent(context: Context): PendingIntent = PendingIntent.getActivity(
            context, 42,
            Intent(context, ShareActivity::class.java).setAction(ACTION_PICK).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }
}

/** Full-screen, over the lock screen, while the phone rings. Any button or tap stops it. */
class FindMyPhoneActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setShowWhenLocked(true)
        setTurnScreenOn(true)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        lifecycleScope.launch {
            Ringer.ringing.first { it == null }
            finish()
        }
        setContent {
            val theme by application.container.settings.theme.collectAsStateWithLifecycle(ThemeSettings())
            val from by Ringer.ringing.collectAsStateWithLifecycle()
            VyomaTheme(theme) { RingingScreen(from ?: "Your PC", ::stop) }
        }
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        if (keyCode == KeyEvent.KEYCODE_VOLUME_DOWN || keyCode == KeyEvent.KEYCODE_VOLUME_UP || keyCode == KeyEvent.KEYCODE_POWER) {
            stop()
            return true
        }
        return super.onKeyDown(keyCode, event)
    }

    private fun stop() {
        Ringer.stop(this)
        Notifications.cancelRinging(this)
        finish()
    }
}

@androidx.compose.runtime.Composable
private fun RingingScreen(from: String, onStop: () -> Unit) {
    val pulse = rememberInfiniteTransition(label = "pulse")
    val scale by pulse.animateFloat(1f, 1.35f, infiniteRepeatable(tween(900), RepeatMode.Restart), label = "scale")
    val alpha by pulse.animateFloat(0.5f, 0f, infiniteRepeatable(tween(900), RepeatMode.Restart), label = "alpha")
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
            Box(contentAlignment = Alignment.Center) {
                Box(
                    Modifier.size(180.dp).graphicsLayer { scaleX = scale; scaleY = scale; this.alpha = alpha }
                        .clip(CircleShape).background(MaterialTheme.colorScheme.primary),
                )
                Box(Modifier.size(132.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primaryContainer), contentAlignment = Alignment.Center) {
                    Icon(Icons.Rounded.PhoneAndroid, null, Modifier.size(56.dp), tint = MaterialTheme.colorScheme.primary)
                }
            }
            Spacer(Modifier.height(40.dp))
            Text("Here I am!", style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.height(8.dp))
            Text("$from is looking for this phone", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Button(
            onClick = onStop,
            modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(24.dp).fillMaxWidth().height(60.dp),
            shape = MaterialTheme.shapes.extraLarge,
            colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.primary),
        ) { Text("Found it", style = MaterialTheme.typography.titleMedium) }
    }
}
