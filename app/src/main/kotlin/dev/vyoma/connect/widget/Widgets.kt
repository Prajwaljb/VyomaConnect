package dev.vyoma.connect.widget

import android.app.Application
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.graphics.scale
import androidx.glance.ColorFilter
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.Image
import androidx.glance.ImageProvider
import androidx.glance.LocalSize
import androidx.glance.action.Action
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.LinearProgressIndicator
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.actionSendBroadcast
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.appwidget.updateAll
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.ContentScale
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxHeight
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.size
import androidx.glance.layout.width
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import androidx.glance.unit.ColorProvider
import dev.vyoma.connect.R
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.plugin.builtin.Transfer
import dev.vyoma.connect.protocol.plugin.builtin.Transfers
import dev.vyoma.connect.service.CommandReceiver
import dev.vyoma.connect.service.Commands
import dev.vyoma.connect.ui.ClipboardActivity
import dev.vyoma.connect.ui.MainActivity
import dev.vyoma.connect.ui.ShareActivity
import kotlinx.coroutines.flow.MutableStateFlow

/** Everything a widget shows, read from in-memory state. Widgets never touch the network. */
data class WidgetSnapshot(
    val pcName: String?,
    val connected: Boolean,
    val pcBattery: Int?,
    val pcCharging: Boolean,
    val phoneBattery: Int?,
    val title: String?,
    val artist: String?,
    val playing: Boolean,
    val art: Bitmap?,
    val clips: List<String>,
    val transferFraction: Float?,
    val transferLabel: String?,
)

object Widgets {
    private val version = MutableStateFlow(0)

    /** Called by the service when anything visible changes. */
    suspend fun refresh(context: Context) {
        version.value++
        listOf(StatusWidget(), ActionsWidget(), MediaWidget(), ClipboardWidget(), DropWidget()).forEach {
            runCatching { it.updateAll(context) }
        }
    }

    @Composable
    internal fun snapshot(context: Context): WidgetSnapshot {
        val tick by version.collectAsState()
        return remember(tick) { read(context) }
    }

    private fun read(context: Context): WidgetSnapshot {
        val c = (context.applicationContext as Application).container
        val primary = Commands.primary(c)
        val known = primary ?: c.hub.value?.devices?.value?.values?.firstOrNull { it.state.value.isPaired }
        val pcBattery = primary?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote?.value
        val np = c.media.nowPlaying.value
        val running = Transfers.all.value.filter { it.state == Transfer.State.RUNNING }
        return WidgetSnapshot(
            pcName = known?.name,
            connected = primary != null,
            pcBattery = pcBattery?.charge,
            pcCharging = pcBattery?.charging == true,
            phoneBattery = c.battery.state.value?.charge,
            title = np?.player?.title?.takeIf { it.isNotBlank() },
            artist = np?.player?.artist,
            playing = np?.player?.isPlaying == true,
            // RemoteViews have a hard bitmap budget; 360px is still sharp at widget size.
            art = np?.art?.let { if (it.width > 360) it.scale(360, 360 * it.height / it.width) else it },
            clips = c.clipboard.history.value.take(4).map { it.text.replace('\n', ' ').take(80) },
            transferFraction = running.takeIf { it.isNotEmpty() }?.let { list ->
                val total = list.sumOf { it.totalBytes.coerceAtLeast(0) }
                if (total > 0) list.sumOf { it.doneBytes }.toFloat() / total else 0f
            },
            transferLabel = running.firstOrNull()?.let { if (running.size == 1) it.fileName else "${running.size} files" },
        )
    }

    // ---- actions ------------------------------------------------------------------------------

    internal fun command(context: Context, action: String): Action =
        actionSendBroadcast(Intent(context, CommandReceiver::class.java).setAction(action))

    internal fun openApp(context: Context, screen: String? = null): Action =
        actionStartActivity(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).apply {
            screen?.let { putExtra(MainActivity.EXTRA_SCREEN, it) }
        })

    internal fun sendClipboard(context: Context): Action = actionStartActivity(ClipboardActivity.intent(context))

    internal fun pickFiles(context: Context): Action =
        actionStartActivity(Intent(context, ShareActivity::class.java).setAction(ShareActivity.ACTION_PICK).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

// ---- shared pieces ----------------------------------------------------------------------------

@Composable
private fun Card(modifier: GlanceModifier = GlanceModifier, content: @Composable () -> Unit) {
    Box(
        modifier.fillMaxSize().cornerRadius(26.dp).background(GlanceTheme.colors.widgetBackground).padding(14.dp),
    ) { content() }
}

@Composable
private fun ActionButton(icon: Int, label: String?, action: Action, modifier: GlanceModifier = GlanceModifier, accent: Boolean = false) {
    Column(
        modifier.cornerRadius(20.dp)
            .background(if (accent) GlanceTheme.colors.primary else GlanceTheme.colors.secondaryContainer)
            .clickable(action)
            .padding(vertical = 10.dp, horizontal = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Image(
            ImageProvider(icon), label,
            colorFilter = ColorFilter.tint(if (accent) GlanceTheme.colors.onPrimary else GlanceTheme.colors.onSecondaryContainer),
            modifier = GlanceModifier.size(22.dp),
        )
        if (label != null) {
            Spacer(GlanceModifier.height(4.dp))
            Text(label, maxLines = 1, style = TextStyle(fontSize = 11.sp, color = GlanceTheme.colors.onSecondaryContainer))
        }
    }
}

private fun style(size: TextUnit, color: ColorProvider, bold: Boolean = false) =
    TextStyle(fontSize = size, color = color, fontWeight = if (bold) FontWeight.Medium else FontWeight.Normal)

// ---- 1. device status -------------------------------------------------------------------------

class StatusWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Responsive(setOf(DpSize(140.dp, 120.dp), DpSize(260.dp, 120.dp), DpSize(260.dp, 200.dp)))

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme {
            val s = Widgets.snapshot(context)
            val wide = LocalSize.current.width >= 250.dp
            Card(GlanceModifier.clickable(Widgets.openApp(context))) {
                Column(GlanceModifier.fillMaxSize()) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = GlanceModifier.fillMaxWidth()) {
                        Box(
                            GlanceModifier.size(40.dp).cornerRadius(14.dp).background(GlanceTheme.colors.primaryContainer),
                            contentAlignment = Alignment.Center,
                        ) {
                            Image(ImageProvider(R.drawable.ic_w_laptop), null, colorFilter = ColorFilter.tint(GlanceTheme.colors.primary), modifier = GlanceModifier.size(22.dp))
                        }
                        Spacer(GlanceModifier.width(10.dp))
                        Column(GlanceModifier.defaultWeight()) {
                            Text(s.pcName ?: "No PC paired", maxLines = 1, style = style(15.sp, GlanceTheme.colors.onSurface, bold = true))
                            Text(
                                when {
                                    s.pcName == null -> "Open Vyoma to pair"
                                    !s.connected -> "Not connected"
                                    s.pcBattery != null -> "Connected · ${s.pcBattery}%${if (s.pcCharging) " ⚡" else ""}"
                                    else -> "Connected"
                                },
                                maxLines = 1,
                                style = style(12.sp, if (s.connected) GlanceTheme.colors.primary else GlanceTheme.colors.onSurfaceVariant),
                            )
                        }
                    }
                    Spacer(GlanceModifier.defaultWeight())
                    if (s.title != null && LocalSize.current.height >= 190.dp) {
                        Row(
                            GlanceModifier.fillMaxWidth().cornerRadius(16.dp).background(GlanceTheme.colors.secondaryContainer).padding(8.dp)
                                .clickable(Widgets.command(context, Commands.PLAY_PAUSE)),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Image(ImageProvider(if (s.playing) R.drawable.ic_pause else R.drawable.ic_play), null, colorFilter = ColorFilter.tint(GlanceTheme.colors.onSecondaryContainer), modifier = GlanceModifier.size(20.dp))
                            Spacer(GlanceModifier.width(8.dp))
                            Text(s.title, maxLines = 1, style = style(12.sp, GlanceTheme.colors.onSecondaryContainer))
                        }
                        Spacer(GlanceModifier.height(8.dp))
                    }
                    Row(GlanceModifier.fillMaxWidth()) {
                        ActionButton(R.drawable.ic_w_clipboard, null, Widgets.sendClipboard(context), GlanceModifier.defaultWeight())
                        Spacer(GlanceModifier.width(6.dp))
                        ActionButton(R.drawable.ic_w_upload, null, Widgets.pickFiles(context), GlanceModifier.defaultWeight())
                        if (wide) {
                            Spacer(GlanceModifier.width(6.dp))
                            ActionButton(R.drawable.ic_w_mouse, null, Widgets.openApp(context, MainActivity.SCREEN_TOUCHPAD), GlanceModifier.defaultWeight())
                            Spacer(GlanceModifier.width(6.dp))
                            ActionButton(R.drawable.ic_w_ring, null, Widgets.command(context, Commands.RING_PC), GlanceModifier.defaultWeight())
                        }
                    }
                }
            }
        }
    }
}

// ---- 2. quick actions grid --------------------------------------------------------------------

class ActionsWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Responsive(setOf(DpSize(260.dp, 90.dp), DpSize(260.dp, 180.dp)))

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme {
            val s = Widgets.snapshot(context)
            val tall = LocalSize.current.height >= 170.dp
            Card {
                Column(GlanceModifier.fillMaxSize()) {
                    val first = listOf(
                        Triple(R.drawable.ic_w_clipboard, "Clipboard", Widgets.sendClipboard(context)),
                        Triple(R.drawable.ic_w_upload, "Files", Widgets.pickFiles(context)),
                        Triple(R.drawable.ic_w_mouse, "Touchpad", Widgets.openApp(context, MainActivity.SCREEN_TOUCHPAD)),
                        Triple(if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, if (s.playing) "Pause" else "Play", Widgets.command(context, Commands.PLAY_PAUSE)),
                    )
                    val second = listOf(
                        Triple(R.drawable.ic_w_ring, "Ring PC", Widgets.command(context, Commands.RING_PC)),
                        Triple(R.drawable.ic_w_lock, "Lock PC", Widgets.command(context, Commands.LOCK_PC)),
                        Triple(R.drawable.ic_w_keyboard, "Keyboard", Widgets.openApp(context, MainActivity.SCREEN_KEYBOARD)),
                        Triple(R.drawable.ic_w_bell, "Ping", Widgets.command(context, Commands.PING)),
                    )
                    GridRow(first, GlanceModifier.defaultWeight())
                    if (tall) {
                        Spacer(GlanceModifier.height(8.dp))
                        GridRow(second, GlanceModifier.defaultWeight())
                    }
                }
            }
        }
    }

    @Composable
    private fun GridRow(items: List<Triple<Int, String, Action>>, modifier: GlanceModifier) {
        Row(modifier.fillMaxWidth()) {
            items.forEachIndexed { i, (icon, label, action) ->
                if (i > 0) Spacer(GlanceModifier.width(8.dp))
                ActionButton(icon, label, action, GlanceModifier.defaultWeight().fillMaxHeight())
            }
        }
    }
}

// ---- 3. PC media ------------------------------------------------------------------------------

class MediaWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Responsive(setOf(DpSize(260.dp, 110.dp), DpSize(260.dp, 200.dp)))

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme {
            val s = Widgets.snapshot(context)
            val tall = LocalSize.current.height >= 190.dp
            Card(GlanceModifier.clickable(Widgets.openApp(context))) {
                Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    val artSize = if (tall) 140.dp else 82.dp
                    Box(GlanceModifier.size(artSize).cornerRadius(18.dp).background(GlanceTheme.colors.secondaryContainer), contentAlignment = Alignment.Center) {
                        if (s.art != null) {
                            Image(ImageProvider(s.art), s.title, contentScale = ContentScale.Crop, modifier = GlanceModifier.fillMaxSize().cornerRadius(18.dp))
                        } else {
                            Image(ImageProvider(R.drawable.ic_w_music), null, colorFilter = ColorFilter.tint(GlanceTheme.colors.onSecondaryContainer), modifier = GlanceModifier.size(32.dp))
                        }
                    }
                    Spacer(GlanceModifier.width(14.dp))
                    Column(GlanceModifier.defaultWeight().fillMaxHeight(), verticalAlignment = Alignment.CenterVertically) {
                        Text(s.title ?: "Nothing playing", maxLines = 2, style = style(16.sp, GlanceTheme.colors.onSurface, bold = true))
                        Text(
                            s.artist?.takeIf { it.isNotBlank() } ?: s.pcName?.let { "on $it" } ?: "Play something on your PC",
                            maxLines = 1, style = style(13.sp, GlanceTheme.colors.onSurfaceVariant),
                        )
                        Spacer(GlanceModifier.height(10.dp))
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            MediaKey(R.drawable.ic_skip_previous, Widgets.command(context, Commands.PREVIOUS))
                            Spacer(GlanceModifier.width(8.dp))
                            MediaKey(if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, Widgets.command(context, Commands.PLAY_PAUSE), accent = true)
                            Spacer(GlanceModifier.width(8.dp))
                            MediaKey(R.drawable.ic_skip_next, Widgets.command(context, Commands.NEXT))
                        }
                    }
                }
            }
        }
    }

    @Composable
    private fun MediaKey(icon: Int, action: Action, accent: Boolean = false) {
        Box(
            GlanceModifier.size(if (accent) 48.dp else 40.dp).cornerRadius(24.dp)
                .background(if (accent) GlanceTheme.colors.primary else GlanceTheme.colors.secondaryContainer)
                .clickable(action),
            contentAlignment = Alignment.Center,
        ) {
            Image(ImageProvider(icon), null, colorFilter = ColorFilter.tint(if (accent) GlanceTheme.colors.onPrimary else GlanceTheme.colors.onSecondaryContainer), modifier = GlanceModifier.size(22.dp))
        }
    }
}

// ---- 4. clipboard -----------------------------------------------------------------------------

class ClipboardWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Exact

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme {
            val s = Widgets.snapshot(context)
            Card {
                Column(GlanceModifier.fillMaxSize()) {
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("Clipboard", style = style(15.sp, GlanceTheme.colors.onSurface, bold = true), modifier = GlanceModifier.defaultWeight())
                        Box(
                            GlanceModifier.cornerRadius(16.dp).background(GlanceTheme.colors.primary).padding(horizontal = 12.dp, vertical = 6.dp)
                                .clickable(Widgets.sendClipboard(context)),
                        ) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Image(ImageProvider(R.drawable.ic_w_send), null, colorFilter = ColorFilter.tint(GlanceTheme.colors.onPrimary), modifier = GlanceModifier.size(14.dp))
                                Spacer(GlanceModifier.width(6.dp))
                                Text("Send", style = style(12.sp, GlanceTheme.colors.onPrimary, bold = true))
                            }
                        }
                    }
                    Spacer(GlanceModifier.height(8.dp))
                    if (s.clips.isEmpty()) {
                        Text("Copied text shows up here", style = style(12.sp, GlanceTheme.colors.onSurfaceVariant))
                    }
                    s.clips.forEach { clip ->
                        Box(
                            GlanceModifier.fillMaxWidth().cornerRadius(14.dp).background(GlanceTheme.colors.secondaryContainer).padding(horizontal = 10.dp, vertical = 8.dp)
                                .clickable(Widgets.openApp(context, MainActivity.SCREEN_CLIPBOARD)),
                        ) {
                            Text(clip, maxLines = 1, style = style(12.sp, GlanceTheme.colors.onSecondaryContainer))
                        }
                        Spacer(GlanceModifier.height(6.dp))
                    }
                }
            }
        }
    }
}

// ---- 5. file drop -----------------------------------------------------------------------------

class DropWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Exact

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme {
            val s = Widgets.snapshot(context)
            Box(
                GlanceModifier.fillMaxSize().cornerRadius(26.dp).background(GlanceTheme.colors.primaryContainer).clickable(Widgets.pickFiles(context)).padding(12.dp),
                contentAlignment = Alignment.Center,
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally, verticalAlignment = Alignment.CenterVertically) {
                    Image(ImageProvider(R.drawable.ic_w_upload), "Send files", colorFilter = ColorFilter.tint(GlanceTheme.colors.primary), modifier = GlanceModifier.size(30.dp))
                    Spacer(GlanceModifier.height(6.dp))
                    if (s.transferFraction != null) {
                        LinearProgressIndicator(
                            s.transferFraction, GlanceModifier.fillMaxWidth().height(4.dp),
                            color = GlanceTheme.colors.primary, backgroundColor = ColorProvider(Color(0x22000000)),
                        )
                        Spacer(GlanceModifier.height(4.dp))
                        Text(s.transferLabel ?: "", maxLines = 1, style = style(11.sp, GlanceTheme.colors.onPrimaryContainer))
                    } else {
                        Text(if (s.connected) "Send to ${s.pcName}" else "Send files", maxLines = 1, style = style(12.sp, GlanceTheme.colors.onPrimaryContainer, bold = true))
                    }
                }
            }
        }
    }
}

class StatusWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = StatusWidget() }
class ActionsWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = ActionsWidget() }
class MediaWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = MediaWidget() }
class ClipboardWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = ClipboardWidget() }
class DropWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = DropWidget() }
