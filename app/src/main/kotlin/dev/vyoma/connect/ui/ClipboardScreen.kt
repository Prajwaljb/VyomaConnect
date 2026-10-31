package dev.vyoma.connect.ui

import androidx.compose.runtime.getValue
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.glassCard
import androidx.compose.foundation.layout.Box
import dev.vyoma.connect.designsystem.SystemColor
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
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
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
import dev.vyoma.connect.designsystem.GlassDialog
import androidx.compose.material3.TextButton
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.material.icons.rounded.Sync

/** Shared clipboard: what's on it now, recent history from both sides, and how sync is running. */
@Composable
fun ClipboardScreen(vm: AppViewModel, onBack: (() -> Unit)? = null) {
    val history by vm.clipHistory.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val background = vm.backgroundClipboardSync()
    var setup by rememberSaveable { mutableStateOf(false) }
    var clearing by rememberSaveable { mutableStateOf(false) }

    CollapsingHeaderScaffold(
        title = "Clipboard",
        subtitle = if (background) "Syncing automatically" else "Syncs while Vyoma is open",
        navigation = onBack?.let { back -> { BounceIconButton(onClick = back) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } } },
        // Clearing can't be undone, so it asks first.
        actions = { if (history.isNotEmpty()) BounceIconButton(onClick = { clearing = true }) { Icon(Icons.Rounded.DeleteSweep, "Clear history") } },
    ) {
        item(key = "send") {
            SectionCard(contentPadding = PaddingValues(16.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text("Send what's copied", style = MaterialTheme.typography.titleMedium)
                        Text("Pushes this phone's clipboard to your PC", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    PillButton("Send", onClick = { context.startActivity(ClipboardActivity.intent(context)) })
                }
            }
        }
        if (!background) {
            item(key = "bg") {
                SectionCard {
                    // Opens the setup right here (it used to point at a Settings section by a wrong name).
                    ListRow(
                        title = "Sync when closed",
                        subtitle = "Android only lets the open app read the clipboard. A one-time grant from your PC lifts that, no root.",
                        icon = Icons.Rounded.Sync,
                        iconTint = SystemColor.ORANGE.color,
                        trailing = { PillButton("Set up", onClick = { setup = true }, prominent = false) },
                        onClick = { setup = true },
                    )
                }
            }
        }
        if (history.isNotEmpty()) item(key = "label") { SectionLabel("Recent") }
        else item(key = "empty") {
            Text(
                "Nothing yet. What you copy on this phone or your PC shows up here; tap one to copy it again.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 32.dp),
            )
        }
        items(history, key = { it.timestamp.toString() + it.text.hashCode() }) { clip ->
            Box(
                Modifier.fillMaxWidth().animateItem().pressable(pressedScale = 0.98f) {
                    vm.copyClip(clip.text)
                    Toast.makeText(context, "Copied, and sent to your PC", Toast.LENGTH_SHORT).show()
                }.glassCard(MaterialTheme.shapes.large),
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
                                ago(clip.timestamp),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
    if (setup) ClipboardAdbDialog { setup = false }
    if (clearing) {
        GlassDialog(
            onDismissRequest = { clearing = false },
            title = { Text("Clear clipboard history?") },
            text = { Text("Removes the list on this phone. What's copied right now stays.") },
            confirmButton = { TextButton(onClick = { vm.clearClipHistory(); clearing = false }) { Text("Clear") } },
            dismissButton = { TextButton(onClick = { clearing = false }) { Text("Cancel") } },
        )
    }
}
