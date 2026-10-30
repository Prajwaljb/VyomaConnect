package dev.vyoma.connect.ui

import android.content.Intent
import android.net.Uri
import android.text.format.DateUtils
import android.text.format.Formatter
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.ui.layout.layout
import dev.vyoma.connect.platform.outgoingFile
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.InsertDriveFile
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.rounded.Archive
import androidx.compose.material.icons.rounded.Audiotrack
import androidx.compose.material.icons.rounded.Code
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.FolderOff
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Movie
import androidx.compose.material.icons.rounded.PictureAsPdf
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.cardSegment
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.platform.AndroidShareSink
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PcFile
import dev.vyoma.connect.protocol.plugin.builtin.PcFileEvent

/**
 * The PC's home folder, browsed from the phone through the companion. Folders open in place
 * (back goes up a level before leaving); tapping a file downloads it to Downloads/Vyoma, with the
 * usual transfer notification, and opens it when it lands. "Upload here" puts phone files into
 * the folder on screen.
 */
@Composable
fun FilesScreen(vm: AppViewModel, pcId: String, onBack: () -> Unit) {
    val context = LocalContext.current
    val haptics = LocalHapticFeedback.current
    val devices by vm.devices.collectAsStateWithLifecycle()
    val pcName = devices.firstOrNull { it.identity.deviceId == pcId }?.identity?.deviceName ?: "PC"
    val companion = devices.firstOrNull { it.identity.companionOf == pcId && it.isPaired && it.isReachable }
    val plugin = remember(companion?.identity?.deviceId, companion?.plugins) {
        companion?.let { vm.device(it.identity.deviceId)?.plugin<CompanionPlugin>(CompanionPlugin.ID) }
    }

    var path by rememberSaveable { mutableStateOf("") }
    BackHandler(enabled = path.isNotEmpty()) { path = path.substringBeforeLast('/', "") }
    LaunchedEffect(plugin, path) { plugin?.listFiles(path) }

    val folder by (plugin?.folder ?: remember { kotlinx.coroutines.flow.MutableStateFlow(null) }).collectAsStateWithLifecycle()
    val fetching by (plugin?.fetching ?: remember { kotlinx.coroutines.flow.MutableStateFlow(emptySet()) }).collectAsStateWithLifecycle()
    LaunchedEffect(plugin) {
        plugin?.fileEvents?.collect { e ->
            when (e) {
                is PcFileEvent.Saved -> {
                    haptics.perform(Haptic.Success)
                    val open = Intent(Intent.ACTION_VIEW)
                        .setDataAndType(Uri.parse(e.location), AndroidShareSink.mimeOf(e.name))
                        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    runCatching { context.startActivity(open) }
                        .onFailure { toast(context, "${e.name} is in Downloads/Vyoma") }
                }
                is PcFileEvent.Uploaded -> {
                    haptics.perform(Haptic.Success)
                    toast(context, "Uploaded ${e.name}")
                }
                is PcFileEvent.Failed -> {
                    haptics.perform(Haptic.Failure)
                    toast(context, "Couldn't get ${e.name}: ${e.reason}")
                }
            }
        }
    }

    val upload = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (plugin != null && uris.isNotEmpty()) vm.uploadPcFiles(plugin, path, uris.map(context.contentResolver::outgoingFile))
    }

    val shown = folder?.takeIf { it.path == path }
    val listState = rememberLazyListState()
    CollapsingHeaderScaffold(
        listState = listState,
        title = if (path.isEmpty()) "Home" else path.substringAfterLast('/'),
        subtitle = when {
            plugin == null -> pcName
            shown == null || (shown.loading && shown.entries.isEmpty()) -> "$pcName · Loading…"
            shown.error != null -> pcName
            else -> "$pcName · ${itemCount(shown.entries.size)}"
        },
        navigation = { BounceIconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
        actions = {
            if (plugin != null) BounceIconButton(onClick = { vm.listPcFiles(plugin, path) }) { Icon(Icons.Rounded.Refresh, "Refresh") }
        },
    ) {
        if (plugin == null) {
            item(key = "offline") {
                SectionCard {
                    ListRow(
                        title = "The companion isn't connected",
                        subtitle = "Browsing PC files needs Vyoma's companion running on $pcName",
                        icon = Icons.Rounded.FolderOff,
                        iconTint = SystemColor.GRAY.color,
                    )
                }
            }
            return@CollapsingHeaderScaffold
        }

        item(key = "crumbs") {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Breadcrumbs(path, onPick = { path = it }, modifier = Modifier.weight(1f))
                // Into the folder on screen; the dashboard's Send drops files in the PC's Downloads instead.
                PillButton("Upload here", prominent = false, onClick = { upload.launch(arrayOf("*/*")) })
            }
        }

        when {
            shown == null || (shown.loading && shown.entries.isEmpty()) -> item(key = "loading") {
                Box(Modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 3.dp)
                }
            }
            shown.error != null -> item(key = "error") {
                SectionCard(Modifier.animateItem()) {
                    ListRow(
                        title = "Couldn't open this folder",
                        subtitle = shown.error,
                        icon = Icons.Rounded.FolderOff,
                        iconTint = SystemColor.RED.color,
                    )
                    PillButton(
                        "Try again", prominent = false,
                        modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp).fillMaxWidth(),
                        onClick = { vm.listPcFiles(plugin, path) },
                    )
                }
            }
            shown.entries.isEmpty() -> item(key = "empty") {
                SectionCard(Modifier.animateItem()) {
                    ListRow(title = "This folder is empty", icon = Icons.Rounded.Folder, iconTint = SystemColor.GRAY.color)
                }
            }
            else -> {
                val last = shown.entries.lastIndex
                shown.entries.forEachIndexed { i, f ->
                    val child = if (path.isEmpty()) f.name else "$path/${f.name}"
                    item(key = child, contentType = ENTRY) {
                        FileRow(
                            f,
                            busy = child in fetching,
                            showDivider = i != last,
                            modifier = (if (i != last) Modifier.joinNext() else Modifier).cardSegment(first = i == 0, last = i == last),
                            onClick = { if (f.isDir) path = child else vm.downloadPcFile(plugin, child) },
                        )
                    }
                }
                if (shown.truncated) item(key = "truncated") {
                    Text(
                        "Showing the first ${shown.entries.size} items",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(start = 20.dp, top = 4.dp),
                    )
                }
            }
        }
    }
}

/** Home › Documents › Work: every level is a chip you can jump back to. */
@Composable
private fun Breadcrumbs(path: String, onPick: (String) -> Unit, modifier: Modifier) {
    val parts = if (path.isEmpty()) emptyList() else path.split('/')
    val scroll = rememberScrollState()
    // Deep paths keep the folder you're in visible.
    LaunchedEffect(path) { scroll.animateScrollTo(scroll.maxValue) }
    Row(
        modifier.horizontalScroll(scroll).padding(bottom = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Crumb("Home", current = parts.isEmpty()) { onPick("") }
        parts.forEachIndexed { i, name ->
            Icon(
                Icons.AutoMirrored.Rounded.KeyboardArrowRight, null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp),
            )
            Crumb(name, current = i == parts.lastIndex) { onPick(parts.take(i + 1).joinToString("/")) }
        }
    }
}

@Composable
private fun Crumb(name: String, current: Boolean, onClick: () -> Unit) {
    Text(
        name,
        style = MaterialTheme.typography.labelLarge,
        color = if (current) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
        maxLines = 1,
        modifier = Modifier
            .then(if (current) Modifier else Modifier.pressable(pressedScale = 0.92f, haptic = Haptic.Select, onClick = onClick))
            .padding(horizontal = 10.dp, vertical = 8.dp),
    )
}

@Composable
private fun FileRow(f: PcFile, busy: Boolean, showDivider: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val context = LocalContext.current
    val (icon, color) = if (f.isDir) Icons.Rounded.Folder to SystemColor.BLUE else kindOf(f.name)
    val date = if (f.modified > 0) {
        DateUtils.getRelativeTimeSpanString(f.modified, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS, DateUtils.FORMAT_ABBREV_ALL).toString()
    } else null
    ListRow(
        title = f.name,
        subtitle = listOfNotNull(if (f.isDir) null else Formatter.formatShortFileSize(context, f.size), date).joinToString(" · ").ifEmpty { null },
        icon = icon,
        iconTint = color.color,
        showDivider = showDivider,
        modifier = modifier,
        trailing = when {
            busy -> { { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp) } }
            f.isDir -> { { Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant) } }
            else -> null
        },
        onClick = onClick,
    )
}

/** A glyph and a stable color per kind of file, so a folder reads at a glance. */
private fun kindOf(name: String): Pair<ImageVector, SystemColor> = when (name.substringAfterLast('.', "").lowercase()) {
    "png", "jpg", "jpeg", "webp", "gif", "heic", "avif", "svg", "bmp" -> Icons.Rounded.Image to SystemColor.PINK
    "mp4", "mkv", "webm", "mov", "avi" -> Icons.Rounded.Movie to SystemColor.RED
    "mp3", "flac", "ogg", "opus", "wav", "m4a" -> Icons.Rounded.Audiotrack to SystemColor.ORANGE
    "pdf" -> Icons.Rounded.PictureAsPdf to SystemColor.RED
    "zip", "tar", "gz", "xz", "zst", "7z", "rar", "apk" -> Icons.Rounded.Archive to SystemColor.BROWN
    "txt", "md", "doc", "docx", "odt", "rtf" -> Icons.Rounded.Description to SystemColor.BLUE
    "kt", "rs", "py", "js", "ts", "c", "h", "cpp", "java", "sh", "json", "toml", "yaml", "yml", "html", "css" ->
        Icons.Rounded.Code to SystemColor.TEAL
    else -> Icons.AutoMirrored.Rounded.InsertDriveFile to SystemColor.GRAY
}

private const val ENTRY = "entry"

/** File rows are list items of their own; this cancels the list's gap so they sit edge to edge on one card. */
private fun Modifier.joinNext() = layout { measurable, constraints ->
    val p = measurable.measure(constraints)
    layout(p.width, p.height - 12.dp.roundToPx()) { p.place(0, 0) }
}

private fun itemCount(n: Int) = if (n == 1) "1 item" else "$n items"

private fun toast(context: android.content.Context, text: String) = Toast.makeText(context, text, Toast.LENGTH_SHORT).show()
