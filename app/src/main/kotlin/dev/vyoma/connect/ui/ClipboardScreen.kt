package dev.vyoma.connect.ui

import android.text.format.DateUtils
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.DeleteSweep
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.pressable
import android.widget.Toast
import androidx.compose.material.icons.rounded.Sync

/** Shared clipboard: what's on it now, recent history from both sides, and how sync is running. */
@Composable
fun ClipboardScreen(vm: AppViewModel, onBack: () -> Unit) {
    val history by vm.clipHistory.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val background = vm.backgroundClipboardSync()

    CollapsingHeaderScaffold(
        title = "Clipboard",
        subtitle = if (background) "Syncing automatically" else "Syncs while Vyoma is open",
        navigation = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
        actions = { if (history.isNotEmpty()) IconButton(onClick = vm::clearClipHistory) { Icon(Icons.Rounded.DeleteSweep, "Clear history") } },
    ) {
        item(key = "send") {
            SectionCard(contentPadding = PaddingValues(16.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text("Send what's copied", style = MaterialTheme.typography.titleMedium)
                        Text("Pushes this phone's clipboard to your PC", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    FilledTonalButton(onClick = { context.startActivity(ClipboardActivity.intent(context)) }) {
                        Icon(Icons.AutoMirrored.Rounded.Send, null, Modifier.padding(end = 6.dp))
                        Text("Send")
                    }
                }
            }
        }
        if (!background) {
            item(key = "bg") {
                SectionCard {
                    ListRow(
                        title = "Turn on automatic sync",
                        subtitle = "Android blocks background clipboard reading. A one-time ADB grant lifts that, no root needed. See Settings → Clipboard.",
                        icon = Icons.Rounded.Sync,
                    )
                }
            }
        }
        if (history.isNotEmpty()) item(key = "label") { SectionLabel("Recent") }
        items(history, key = { it.timestamp.toString() + it.text.hashCode() }) { clip ->
            Surface(
                Modifier.fillMaxWidth().animateItem().pressable(pressedScale = 0.98f) {
                    vm.copyClip(clip.text)
                    Toast.makeText(context, "Copied, and sent to your PC", Toast.LENGTH_SHORT).show()
                },
                shape = MaterialTheme.shapes.large,
                color = MaterialTheme.colorScheme.surfaceContainerLowest,
            ) {
                Column(Modifier.padding(16.dp)) {
                    Text(clip.text, style = MaterialTheme.typography.bodyMedium, maxLines = 4, overflow = TextOverflow.Ellipsis)
                    Spacer(Modifier.height(8.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(
                            if (clip.origin == null) Icons.Rounded.PhoneAndroid else Icons.Rounded.Laptop, null,
                            tint = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(end = 6.dp).width(16.dp),
                        )
                        Text(
                            (if (clip.origin == null) "This phone" else vm.deviceName(clip.origin!!)) + " · " +
                                DateUtils.getRelativeTimeSpanString(clip.timestamp, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}
