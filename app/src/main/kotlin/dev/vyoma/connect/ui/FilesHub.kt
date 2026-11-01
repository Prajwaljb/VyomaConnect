package dev.vyoma.connect.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.text.format.Formatter
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.material.icons.rounded.PhotoLibrary
import androidx.compose.material.icons.rounded.ContentPasteGo
import androidx.compose.material.icons.rounded.FolderOff
import androidx.compose.material.icons.rounded.DesktopWindows
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.MusicNote
import androidx.compose.material.icons.rounded.Movie
import dev.vyoma.connect.designsystem.isDarkUi
import androidx.compose.ui.graphics.lerp
import dev.vyoma.connect.designsystem.glassCard
import dev.vyoma.connect.designsystem.glassRaised
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.designsystem.toned
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
fun FilesHubScreen(vm: AppViewModel, onBrowse: (String, String) -> Unit, onClipboard: () -> Unit, onCaptures: () -> Unit = {}) {
    val context = LocalContext.current
    val status = LocalStatus.current
    val devices by vm.devices.collectAsStateWithLifecycle()
    val pc = devices.firstOrNull { it.identity.companionOf == null && it.isPaired && it.isReachable }
    val companion = pc?.let { p -> devices.any { it.identity.companionOf == p.identity.deviceId && it.isPaired && it.isReachable } } == true
    val running by Transfers.all.collectAsStateWithLifecycle()
    val recent by remember { TransferLog.recent(context) }.collectAsStateWithLifecycle()
    val clips by vm.clipHistory.collectAsStateWithLifecycle()
    RefreshCaptures(null)
    val moving = running.filter { it.state == Transfer.State.RUNNING }
    val done = recent.orEmpty().take(RECENT_SHOWN)

    CollapsingHeaderScaffold(
        title = "Files",
        subtitle = pc?.let { "With ${it.identity.deviceName}" } ?: "No PC connected",
    ) {
        // Sending is the job people come here for: three big glass tiles, one per kind of thing.
        item(key = "send-label") { SectionLabel(pc?.let { "Send to ${it.identity.deviceName}" } ?: "Send to your PC", Modifier.animateItem()) }
        item(key = "send") {
            Row(Modifier.animateItem().fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SendTile(Icons.Rounded.UploadFile, "Files", SystemColor.BLUE, pc != null, Modifier.weight(1f)) { vm.pickFiles(context) }
                SendTile(Icons.Rounded.PhotoLibrary, "Photos", SystemColor.PINK, pc != null, Modifier.weight(1f)) { vm.pickPhotos(context) }
                SendTile(Icons.Rounded.ContentPasteGo, "Clipboard", SystemColor.ORANGE, pc != null, Modifier.weight(1f)) {
                    if (vm.sendClipboard() != null) status.show("Clipboard sent") else status.show("Nothing copied to send", failure = true)
                }
            }
        }

        // The PC's usual folders, a tap from here (the companion lists and fetches them).
        item(key = "pc-label") { SectionLabel("On the PC", Modifier.animateItem()) }
        item(key = "pc") {
            if (companion && pc != null) {
                Column(Modifier.animateItem().glassCard(MaterialTheme.shapes.large).padding(10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    PC_FOLDERS.chunked(3).forEach { row ->
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            row.forEach { f -> FolderTile(f, Modifier.weight(1f)) { onBrowse(pc.identity.deviceId, f.path) } }
                        }
                    }
                    ListRow(
                        title = "Browse everything",
                        subtitle = "Its home folder, to open or upload into",
                        icon = Icons.Rounded.Folder,
                        iconTint = SystemColor.BLUE.color,
                        horizontalPadding = 10.dp,
                        trailing = { RowChevron() },
                        onClick = { onBrowse(pc.identity.deviceId, "") },
                    )
                }
            } else {
                SectionCard(Modifier.animateItem()) {
                    ListRow(
                        title = "Browse PC files",
                        subtitle = if (pc == null) "Connect to your PC first" else "Needs the Vyoma companion on the PC",
                        icon = Icons.Rounded.FolderOff,
                        iconTint = SystemColor.GRAY.color,
                    )
                }
            }
        }

        item(key = "captures-label") { SectionLabel("Captures", Modifier.animateItem()) }
        item(key = "captures") {
            val plugin = remember(companion, pc?.identity?.deviceId, devices) { pc?.let { vm.companionOf(it.identity.deviceId) } }
            CapturesStrip(onTake = plugin?.let { c -> { vm.capture(c.screenshot) } }, onSeeAll = onCaptures, modifier = Modifier.animateItem())
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
                    icon = if (clips.isEmpty()) Icons.Rounded.ContentPaste else null,
                    iconTint = SystemColor.ORANGE.color,
                    trailing = { RowChevron() },
                    onClick = onClipboard,
                )
            }
        }
    }
}

/** One of the Send tiles: a filled Shortcuts tile, wider than Home's, glyph over its name. */
@Composable
private fun SendTile(icon: ImageVector, label: String, color: SystemColor, enabled: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val tone = color.color
    val fill = if (tone.luminance() in 0.2f..0.5f) tone.toned(lightness = 0.38f) else tone
    Column(
        modifier
            .height(96.dp)
            .graphicsLayer { if (!enabled) alpha = 0.4f }
            .pressable(enabled = enabled, onClick = onClick)
            .glassRaised(RoundedCornerShape(20.dp), fill.copy(alpha = 0.92f))
            .padding(14.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Icon(icon, null, tint = Color.White, modifier = Modifier.size(26.dp))
        Text(label, style = MaterialTheme.typography.titleSmall, color = Color.White, maxLines = 1)
    }
}

private class PcFolderLink(val name: String, val path: String, val icon: ImageVector, val color: SystemColor)

private val PC_FOLDERS = listOf(
    PcFolderLink("Desktop", "Desktop", Icons.Rounded.DesktopWindows, SystemColor.BLUE),
    PcFolderLink("Documents", "Documents", Icons.Rounded.Description, SystemColor.CYAN),
    PcFolderLink("Downloads", "Downloads", Icons.Rounded.Download, SystemColor.GREEN),
    PcFolderLink("Pictures", "Pictures", Icons.Rounded.Image, SystemColor.PINK),
    PcFolderLink("Music", "Music", Icons.Rounded.MusicNote, SystemColor.RED),
    PcFolderLink("Videos", "Videos", Icons.Rounded.Movie, SystemColor.ORANGE),
)

/**
 * A PC folder as a clear glass tile: its glyph bare in its colour, its name under. (A coloured
 * badge inside the tile inside the card was three nested boxes.) 16 dp corners sit concentric in
 * the 26 dp card's 10 dp padding.
 */
@Composable
private fun FolderTile(f: PcFolderLink, modifier: Modifier, onClick: () -> Unit) {
    val tone = f.color.color
    Column(
        modifier
            .height(88.dp)
            .pressable(onClick = onClick)
            .glassRaised(RoundedCornerShape(16.dp))
            .padding(12.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Icon(f.icon, null, tint = if (isDarkUi) tone else lerp(tone, Color.Black, 0.35f), modifier = Modifier.size(28.dp))
        Text(f.name, style = MaterialTheme.typography.labelLarge, maxLines = 1)
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

private const val RECENT_SHOWN = 12
private const val CLIPS_SHOWN = 5
