package dev.vyoma.connect.ui

import dev.vyoma.connect.designsystem.LocalStatus

import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.rounded.ContentCopy
import dev.vyoma.connect.designsystem.DialogButton
import dev.vyoma.connect.designsystem.RowChevron
import androidx.compose.material.icons.rounded.ContentPaste
import dev.vyoma.connect.designsystem.cardSegment
import androidx.compose.material3.HorizontalDivider
import androidx.compose.foundation.lazy.itemsIndexed
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.material.icons.rounded.Sync
import dev.vyoma.connect.platform.AppLock

/** Shared clipboard: what's on it now, recent history from both sides, and how sync is running. */
@Composable
fun ClipboardScreen(vm: AppViewModel, onBack: (() -> Unit)? = null) {
    val clips by vm.clipHistory.collectAsStateWithLifecycle()
    val context = LocalContext.current
    // The page leaves with a slide when the lock closes: nothing to read while it does.
    val history = if (AppLock.isOpen(context)) clips else emptyList()
    val status = LocalStatus.current
    val sync by vm.clipboardSync.collectAsStateWithLifecycle()
    val background = sync == dev.vyoma.connect.platform.AndroidClipboard.SyncState.ON
    var setup by rememberSaveable { mutableStateOf(false) }
    var clearing by rememberSaveable { mutableStateOf(false) }

    CollapsingHeaderScaffold(
        title = "Clipboard",
        subtitle = if (background) "Copies go to your PC by themselves" else "Syncs while Vyoma is open",
        navigation = onBack?.let { back -> { BounceIconButton(onClick = back) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } } },
        // Sending is the one action, so it sits up top and the history comes first on the page.
        // Clearing can't be undone, so it asks first.
        actions = {
            // Sending by hand stays even with background sync on, in case a copy didn't go.
            BounceIconButton(onClick = { context.startActivity(ClipboardActivity.intent(context)) }) { Icon(Icons.AutoMirrored.Rounded.Send, "Send what's copied to PC") }
            if (history.isNotEmpty()) BounceIconButton(onClick = { clearing = true }) { Icon(Icons.Rounded.DeleteSweep, "Clear history") }
        },
    ) {
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
        // One grouped card, a slice per clip (the list's own items, so long histories stay lazy).
        val last = history.lastIndex
        itemsIndexed(history, key = { _, it -> it.timestamp.toString() + it.text.hashCode() }) { i, clip ->
            Column(
                Modifier.fillMaxWidth().animateItem()
                    .then(if (i != last) Modifier.joinNext() else Modifier)
                    .cardSegment(first = i == 0, last = i == last)
                    .pressable {
                        vm.copyClip(clip.text)
                        status.show("Copied, and sent to your PC")
                    },
            ) {
                Row(Modifier.padding(horizontal = 20.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) { Column(Modifier.weight(1f)) {
                    Text(clip.text, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Spacer(Modifier.height(8.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(
                            if (clip.origin == null) Icons.Rounded.PhoneAndroid else Icons.Rounded.Laptop, null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(end = 6.dp).width(16.dp),
                        )
                        Text(
                            (if (clip.origin == null) "This phone" else vm.deviceName(clip.origin!!)) + " · " +
                                ago(clip.timestamp),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                    // What a tap does, said by the row itself (copies it again).
                    Spacer(Modifier.width(12.dp))
                    Icon(Icons.Rounded.ContentCopy, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp))
                }
                if (i != last) HorizontalDivider(Modifier.padding(horizontal = 20.dp), color = MaterialTheme.colorScheme.outlineVariant)
            }
        }
        if (!background) {
            item(key = "bg") {
                SectionCard(Modifier.padding(top = 8.dp)) {
                    // Set up already: only Android's log question, asked again on each restart of the app, is missing.
                    if (sync == dev.vyoma.connect.platform.AndroidClipboard.SyncState.WAITING) ListRow(
                        title = "Allow log access",
                        subtitle = "Tap Ask again, then Allow one-time access, and copies go to your PC by themselves. Android didn't ask? Restart the phone, then open Vyoma",
                        icon = Icons.Rounded.Sync,
                        iconTint = SystemColor.ORANGE.color,
                        trailing = { RowChevron("Ask again") },
                        onClick = { vm.retryClipboardSync() },
                    ) else ListRow(
                        title = "Sync when closed",
                        subtitle = "One-time setup from your PC",
                        icon = Icons.Rounded.Sync,
                        iconTint = SystemColor.ORANGE.color,
                        trailing = { RowChevron("Set up") },
                        onClick = { setup = true },
                    )
                }
            }
        }
    }
    if (setup) ClipboardSetupDialog(vm) { setup = false }
    if (clearing) {
        GlassDialog(
            onDismissRequest = { clearing = false },
            title = { Text("Clear clipboard history?") },
            text = { Text("Removes the list on this phone. What's copied right now stays.") },
            confirmButton = { DialogButton(onClick = { vm.clearClipHistory(); clearing = false }, destructive = true) { Text("Clear") } },
            dismissButton = { DialogButton(onClick = { clearing = false }) { Text("Cancel") } },
        )
    }
}
