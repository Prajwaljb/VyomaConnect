package dev.vyoma.connect.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.text.format.Formatter
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.Glass
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.RowChevron
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.cardSegment
import dev.vyoma.connect.platform.AndroidShareSink
import dev.vyoma.connect.protocol.plugin.builtin.Transfer
import dev.vyoma.connect.protocol.plugin.builtin.Transfers
import dev.vyoma.connect.service.FileRecord
import dev.vyoma.connect.service.TransferLog

/**
 * Everything that moves between phone and PC, in one tab: send and browse, what's on its way and
 * what came and went (it used to live only in notifications), and the latest clips with the full
 * history a tap away. Clipboard sync runs by itself, so it no longer needs a tab of its own.
 */
@Composable
fun FilesHubScreen(vm: AppViewModel, onBrowse: (String) -> Unit, onClipboard: () -> Unit) {
    val context = LocalContext.current
    val status = LocalStatus.current
    val devices by vm.devices.collectAsStateWithLifecycle()
    val pc = devices.firstOrNull { it.identity.companionOf == null && it.isPaired && it.isReachable }
    val companion = pc?.let { p -> devices.any { it.identity.companionOf == p.identity.deviceId && it.isPaired && it.isReachable } } == true
    val running by Transfers.all.collectAsStateWithLifecycle()
    val recent by remember { TransferLog.recent(context) }.collectAsStateWithLifecycle()
    val clips by vm.clipHistory.collectAsStateWithLifecycle()
    val moving = running.filter { it.state == Transfer.State.RUNNING }
    val done = recent.orEmpty().take(RECENT_SHOWN)

    CollapsingHeaderScaffold(
        title = "Files",
        subtitle = pc?.let { "With ${it.identity.deviceName}" } ?: "No PC connected",
    ) {
        item(key = "pc") {
            SectionCard(Modifier.animateItem()) {
                ListRow(
                    title = "Send files",
                    subtitle = "To the PC's Downloads",
                    subtitleMaxLines = 1,
                    icon = Icons.Rounded.UploadFile,
                    iconTint = SystemColor.BLUE.color,
                    showDivider = true,
                    enabled = pc != null,
                    trailing = { RowChevron() },
                    onClick = { vm.pickFiles(context) },
                )
                ListRow(
                    title = "Browse PC files",
                    subtitle = if (companion || pc == null) "Its home folder, to open or upload into" else "Needs the Vyoma companion on the PC",
                    subtitleMaxLines = 1,
                    icon = Icons.Rounded.Folder,
                    iconTint = SystemColor.BLUE.color,
                    enabled = companion,
                    trailing = { RowChevron() },
                    onClick = { pc?.let { onBrowse(it.identity.deviceId) } },
                )
            }
        }

        if (moving.isNotEmpty() || done.isNotEmpty()) {
            item(key = "transfers-label") { SectionLabel("Transfers", Modifier.animateItem()) }
            transferRows(moving, done, context, keyPrefix = "t")
        }

        item(key = "clips-label") { SectionLabel("Clipboard", Modifier.animateItem()) }
        item(key = "clips") {
            SectionCard(Modifier.animateItem()) {
                clips.take(CLIPS_SHOWN).forEach { clip ->
                    ListRow(
                        title = clip.text.replace('\n', ' '),
                        subtitle = (if (clip.origin == null) "This phone" else vm.deviceName(clip.origin!!)) + " · " + ago(clip.timestamp),
                        subtitleMaxLines = 1,
                        icon = Icons.Rounded.ContentPaste,
                        iconTint = SystemColor.ORANGE.color,
                        showDivider = true,
                        trailing = { Icon(Icons.Rounded.ContentCopy, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp)) },
                        onClick = {
                            vm.copyClip(clip.text)
                            status.show("Copied, and sent to your PC")
                        },
                    )
                }
                ListRow(
                    title = if (clips.isEmpty()) "Nothing copied yet" else "All clipboard history",
                    subtitle = if (clips.isEmpty()) "What you copy here or on the PC shows up here" else null,
                    subtitleMaxLines = 1,
                    icon = if (clips.isEmpty()) Icons.Rounded.ContentPaste else null,
                    iconTint = SystemColor.ORANGE.color,
                    trailing = { RowChevron() },
                    onClick = onClipboard,
                )
            }
        }
    }
}

/** Running transfers with a progress bar, then finished ones (incoming open on tap), as one grouped card. */
internal fun LazyListScope.transferRows(moving: List<Transfer>, done: List<FileRecord>, context: Context, keyPrefix: String) {
    val rows: List<Any> = moving + done
    itemsIndexed(rows, key = { _, r -> keyPrefix + if (r is Transfer) "run${r.id}" else (r as FileRecord).let { "${it.at}${it.name}" } }) { i, r ->
        val last = i == rows.lastIndex
        val m = Modifier.animateItem().then(if (!last) Modifier.joinNext() else Modifier).cardSegment(first = i == 0, last = last)
        when (r) {
            is Transfer -> RunningRow(r, !last, m)
            is FileRecord -> DoneRow(r, !last, m, context)
        }
    }
}

@Composable
private fun RunningRow(t: Transfer, divider: Boolean, modifier: Modifier) {
    val context = LocalContext.current
    val (icon, color) = kindOf(t.fileName)
    Column(modifier) {
        ListRow(
            title = t.fileName,
            subtitle = (if (t.incoming) "From ${t.deviceName} · " else "To ${t.deviceName} · ") +
                Formatter.formatShortFileSize(context, t.doneBytes) + " of " + Formatter.formatShortFileSize(context, t.totalBytes),
            subtitleMaxLines = 1,
            icon = icon,
            iconTint = color.color,
            showDivider = false,
        )
        // The house capsule bar (same as Home's meters), under the text it belongs to.
        val fill = Glass.accent
        val track = Glass.raised
        Canvas(Modifier.fillMaxWidth().padding(start = 76.dp, end = 20.dp, bottom = if (divider) 12.dp else 16.dp).height(6.dp)) {
            val r = CornerRadius(size.height / 2)
            drawRoundRect(track, cornerRadius = r)
            drawRoundRect(fill, size = Size(maxOf(size.height, size.width * t.fraction), size.height), cornerRadius = r)
        }
    }
}

@Composable
private fun DoneRow(f: FileRecord, divider: Boolean, modifier: Modifier, context: Context) {
    val status = LocalStatus.current
    val (icon, color) = kindOf(f.name)
    val openable = f.ok && f.incoming && f.location != null
    ListRow(
        title = f.name,
        subtitle = when {
            !f.ok -> "Didn't arrive · ${ago(f.at)}"
            f.incoming -> "From ${f.device} · ${ago(f.at)}"
            else -> "Sent to ${f.device} · ${ago(f.at)}"
        },
        subtitleMaxLines = 1,
        icon = if (f.ok) icon else Icons.Rounded.ErrorOutline,
        iconTint = if (f.ok) color.color else SystemColor.RED.color,
        showDivider = divider,
        modifier = modifier,
        trailing = when {
            openable -> { { Icon(Icons.AutoMirrored.Rounded.OpenInNew, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp)) } }
            !f.incoming && f.ok -> { { Icon(Icons.AutoMirrored.Rounded.Send, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp)) } }
            else -> null
        },
        onClick = if (openable) { {
            val open = Intent(Intent.ACTION_VIEW)
                .setDataAndType(Uri.parse(f.location), AndroidShareSink.mimeOf(f.name))
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            runCatching { context.startActivity(open) }.onFailure { status.show("No app here opens this file", failure = true) }
        } } else null,
    )
}

private const val RECENT_SHOWN = 8
private const val CLIPS_SHOWN = 3
