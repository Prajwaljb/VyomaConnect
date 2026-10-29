package dev.vyoma.connect.widget

import android.app.Application
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.graphics.createBitmap
import androidx.core.graphics.scale
import androidx.glance.ColorFilter
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.Image
import androidx.glance.ImageProvider
import androidx.glance.LocalContext
import androidx.glance.LocalSize
import androidx.glance.action.Action
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.actionSendBroadcast
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.appwidget.updateAll
import androidx.glance.background
import androidx.glance.color.ColorProvider
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Everything a widget shows. Written by the service, cached on disk; widgets never touch the network. */
data class WidgetSnapshot(
    val pcName: String? = null,
    val connected: Boolean = false,
    val pcBattery: Int? = null,
    val pcCharging: Boolean = false,
    val phoneBattery: Int? = null,
    val phoneCharging: Boolean = false,
    val title: String? = null,
    val artist: String? = null,
    val playing: Boolean = false,
    val art: Bitmap? = null,
    val clips: List<String> = emptyList(),
    val transferFraction: Float? = null,
    val transferLabel: String? = null,
)

object Widgets {
    private val state = MutableStateFlow<WidgetSnapshot?>(null)

    /**
     * Called by the service when anything visible changes. Saves the snapshot first: Android may
     * later start our process just to draw a widget, and the live state won't exist then.
     */
    suspend fun refresh(context: Context) {
        val snap = read(context)
        state.value = snap
        withContext(Dispatchers.IO) { runCatching { save(context, snap) } }
        listOf(StatusWidget(), ActionsWidget(), MediaWidget(), ClipboardWidget(), DropWidget()).forEach {
            runCatching { it.updateAll(context) }
        }
    }

    /** The latest snapshot: in memory if the service ran in this process, else from disk. */
    internal fun flow(context: Context): StateFlow<WidgetSnapshot?> {
        if (state.value == null) state.compareAndSet(null, runCatching { load(context) }.getOrNull() ?: WidgetSnapshot())
        return state
    }

    @Composable
    internal fun snapshot(context: Context): WidgetSnapshot {
        val s by flow(context).collectAsState()
        return s ?: WidgetSnapshot()
    }

    private fun read(context: Context): WidgetSnapshot {
        val c = (context.applicationContext as Application).container
        val primary = Commands.primary(c)
        val known = primary ?: c.hub.value?.devices?.value?.values?.firstOrNull { it.state.value.isPaired }
        val pcBattery = primary?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote?.value
        val phone = c.battery.state.value
        val np = c.media.nowPlaying.value
        val running = Transfers.all.value.filter { it.state == Transfer.State.RUNNING }
        return WidgetSnapshot(
            pcName = known?.name ?: state.value?.pcName,
            connected = primary != null,
            pcBattery = pcBattery?.charge,
            pcCharging = pcBattery?.charging == true,
            phoneBattery = phone?.charge,
            phoneCharging = phone?.charging == true,
            title = np?.player?.title?.takeIf { it.isNotBlank() },
            artist = np?.player?.artist,
            playing = np?.player?.isPlaying == true,
            // RemoteViews have a hard bitmap budget; 480px is still sharp full-bleed on a 4x2 widget.
            art = np?.art?.let { if (it.width > 480) it.scale(480, 480 * it.height / it.width) else it },
            clips = c.clipboard.history.value.take(4).map { it.text.replace('\n', ' ').take(80) },
            transferFraction = running.takeIf { it.isNotEmpty() }?.let { list ->
                val total = list.sumOf { it.totalBytes.coerceAtLeast(0) }
                if (total > 0) list.sumOf { it.doneBytes }.toFloat() / total else 0f
            },
            transferLabel = running.firstOrNull()?.let { if (running.size == 1) it.fileName else "${running.size} files" },
        )
    }

    // ---- disk cache ---------------------------------------------------------------------------

    private fun dir(context: Context) = File(context.filesDir, "widgets").apply { mkdirs() }

    private fun save(context: Context, s: WidgetSnapshot) {
        val json = JSONObject()
            .put("pcName", s.pcName).put("connected", s.connected)
            .put("pcBattery", s.pcBattery).put("pcCharging", s.pcCharging)
            .put("phoneBattery", s.phoneBattery).put("phoneCharging", s.phoneCharging)
            .put("title", s.title).put("artist", s.artist).put("playing", s.playing)
            .put("clips", JSONArray(s.clips))
        File(dir(context), "snapshot.json").writeText(json.toString())
        val art = File(dir(context), "art.png")
        if (s.art != null) art.outputStream().use { s.art.compress(Bitmap.CompressFormat.PNG, 100, it) } else art.delete()
    }

    private fun load(context: Context): WidgetSnapshot? {
        val file = File(dir(context), "snapshot.json").takeIf { it.exists() } ?: return null
        val j = JSONObject(file.readText())
        fun JSONObject.intOrNull(k: String) = if (isNull(k)) null else optInt(k)
        fun JSONObject.strOrNull(k: String) = if (isNull(k)) null else optString(k)
        val clips = j.optJSONArray("clips")?.let { a -> List(a.length()) { a.getString(it) } }.orEmpty()
        return WidgetSnapshot(
            pcName = j.strOrNull("pcName"),
            // Whatever we cached, the link isn't up until the service says so.
            connected = false,
            pcBattery = j.intOrNull("pcBattery"),
            pcCharging = j.optBoolean("pcCharging"),
            phoneBattery = j.intOrNull("phoneBattery"),
            phoneCharging = j.optBoolean("phoneCharging"),
            title = j.strOrNull("title"),
            artist = j.strOrNull("artist"),
            playing = false,
            art = File(dir(context), "art.png").takeIf { it.exists() }?.let { BitmapFactory.decodeFile(it.path) },
            clips = clips,
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

// ---- look -------------------------------------------------------------------------------------
// One UI widgets: frosted card, high-contrast type, round glass buttons, accent used sparingly.

private object Ink {
    val primary = ColorProvider(day = Color(0xFF0B0B0F), night = Color(0xFFF5F5F7))
    val secondary = ColorProvider(day = Color(0x99000000), night = Color(0x99FFFFFF))
    val glassButton = ColorProvider(day = Color(0x0F000000), night = Color(0x1AFFFFFF))
    val hairline = ColorProvider(day = Color(0x14000000), night = Color(0x1FFFFFFF))
    val white = ColorProvider(day = Color.White, night = Color.White)
    val whiteSoft = ColorProvider(day = Color(0xB3FFFFFF), night = Color(0xB3FFFFFF))
    val whiteGlass = ColorProvider(day = Color(0x33FFFFFF), night = Color(0x33FFFFFF))
    val online = ColorProvider(day = Color(0xFF1FA855), night = Color(0xFF30D158))
}

@Composable
private fun Glass(modifier: GlanceModifier = GlanceModifier, padding: Dp = 16.dp, content: @Composable () -> Unit) {
    Box(modifier.fillMaxSize().background(ImageProvider(R.drawable.widget_glass)).padding(padding)) { content() }
}

private fun style(size: TextUnit, color: ColorProvider, weight: FontWeight = FontWeight.Normal) =
    TextStyle(fontSize = size, color = color, fontWeight = weight)

/** Round glass button; [accent] fills it with the wallpaper accent for the main action. */
@Composable
private fun RoundButton(icon: Int, desc: String, action: Action, size: Dp = 44.dp, accent: Boolean = false, onArt: Boolean = false) {
    Box(
        GlanceModifier.size(size).cornerRadius(size / 2)
            .background(
                when {
                    accent && onArt -> Ink.white
                    accent -> GlanceTheme.colors.primary
                    onArt -> Ink.whiteGlass
                    else -> Ink.glassButton
                },
            )
            .clickable(action),
        contentAlignment = Alignment.Center,
    ) {
        Image(
            ImageProvider(icon), desc,
            colorFilter = ColorFilter.tint(
                when {
                    accent && onArt -> ColorProvider(day = Color.Black, night = Color.Black)
                    accent -> GlanceTheme.colors.onPrimary
                    onArt -> Ink.white
                    else -> Ink.primary
                },
            ),
            modifier = GlanceModifier.size(size * 0.46f),
        )
    }
}

@Composable
private fun LabeledButton(icon: Int, label: String, action: Action, modifier: GlanceModifier, accent: Boolean = false, size: Dp = 48.dp) {
    Column(modifier.clickable(action), horizontalAlignment = Alignment.CenterHorizontally, verticalAlignment = Alignment.CenterVertically) {
        RoundButton(icon, label, action, size = size, accent = accent)
        Spacer(GlanceModifier.height(6.dp))
        Text(label, maxLines = 1, style = style(11.sp, Ink.secondary, FontWeight.Medium))
    }
}

/** A battery/progress ring as a bitmap (RemoteViews can't draw arcs). Cached by its inputs. */
private object Rings {
    private val cache = object : LinkedHashMap<String, Bitmap>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>) = size > 8
    }

    fun ring(context: Context, sizeDp: Int, fraction: Float, color: Int, track: Int): Bitmap = synchronized(cache) {
        val px = (sizeDp * context.resources.displayMetrics.density).toInt()
        val key = "$px:${(fraction * 100).toInt()}:$color:$track"
        cache.getOrPut(key) {
            createBitmap(px, px).also { bmp ->
                val stroke = px * 0.11f
                val r = RectF(stroke / 2, stroke / 2, px - stroke / 2, px - stroke / 2)
                val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE; strokeWidth = stroke; strokeCap = Paint.Cap.ROUND }
                Canvas(bmp).apply {
                    drawArc(r, 0f, 360f, false, p.apply { this.color = track })
                    if (fraction > 0f) drawArc(r, -90f, 360f * fraction.coerceIn(0.005f, 1f), false, p.apply { this.color = color })
                }
            }
        }
    }

    fun accent(context: Context) = context.getColor(android.R.color.system_accent1_400)
    fun low() = Color(0xFFFF453A).toArgb()
    fun charging() = Color(0xFF30D158).toArgb()
    fun track() = Color(0x33808080).toArgb()
}

@Composable
private fun BatteryRing(label: String, percent: Int?, charging: Boolean, sizeDp: Int) {
    val context = LocalContext.current
    val color = when {
        charging -> Rings.charging()
        percent != null && percent <= 15 -> Rings.low()
        else -> Rings.accent(context)
    }
    val bmp = remember(percent, charging, sizeDp) { Rings.ring(context, sizeDp, (percent ?: 0) / 100f, color, Rings.track()) }
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(GlanceModifier.size(sizeDp.dp), contentAlignment = Alignment.Center) {
            Image(ImageProvider(bmp), null, modifier = GlanceModifier.fillMaxSize())
            Text(percent?.let { "$it" } ?: "–", style = style((sizeDp / 3.6f).sp, Ink.primary, FontWeight.Bold))
        }
        Spacer(GlanceModifier.height(4.dp))
        Text(if (charging) "$label ⚡" else label, maxLines = 1, style = style(11.sp, Ink.secondary, FontWeight.Medium))
    }
}

@Composable
private fun StatusLine(s: WidgetSnapshot) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        if (s.connected) {
            Box(GlanceModifier.size(7.dp).cornerRadius(4.dp).background(Ink.online)) {}
            Spacer(GlanceModifier.width(5.dp))
        }
        Text(
            when {
                s.pcName == null -> "Open Vyoma to pair"
                s.connected -> "Connected"
                else -> "Not connected"
            },
            maxLines = 1,
            style = style(12.sp, if (s.connected) Ink.online else Ink.secondary, FontWeight.Medium),
        )
    }
}

// ---- 1. device status -------------------------------------------------------------------------

class StatusWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Responsive(setOf(DpSize(140.dp, 140.dp), DpSize(260.dp, 120.dp), DpSize(260.dp, 200.dp)))

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme {
            val s = Widgets.snapshot(context)
            val size = LocalSize.current
            val wide = size.width >= 250.dp
            val tall = size.height >= 190.dp
            Glass(GlanceModifier.clickable(Widgets.openApp(context))) {
                Column(GlanceModifier.fillMaxSize()) {
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Column(GlanceModifier.defaultWeight()) {
                            Text(s.pcName ?: "No PC", maxLines = 1, style = style(if (wide) 18.sp else 16.sp, Ink.primary, FontWeight.Bold))
                            Spacer(GlanceModifier.height(2.dp))
                            StatusLine(s)
                        }
                        if (wide && !tall) {
                            BatteryRing("PC", s.pcBattery.takeIf { s.connected }, s.pcCharging && s.connected, 46)
                            Spacer(GlanceModifier.width(10.dp))
                            BatteryRing("Phone", s.phoneBattery, s.phoneCharging, 46)
                        }
                    }
                    Spacer(GlanceModifier.defaultWeight())
                    if (!wide) {
                        // 2x2: one big ring for the PC, the way One UI's battery widget leads with one device.
                        Row(GlanceModifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                            BatteryRing("PC", s.pcBattery.takeIf { s.connected }, s.pcCharging && s.connected, 58)
                        }
                    } else {
                        if (tall) {
                            Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                                BatteryRing("PC", s.pcBattery.takeIf { s.connected }, s.pcCharging && s.connected, 58)
                                Spacer(GlanceModifier.width(14.dp))
                                BatteryRing("Phone", s.phoneBattery, s.phoneCharging, 58)
                                Spacer(GlanceModifier.defaultWeight())
                                if (s.title != null) NowPlayingPill(context, s, GlanceModifier.width(130.dp))
                            }
                            Spacer(GlanceModifier.defaultWeight())
                        }
                        Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                            RoundButton(R.drawable.ic_w_clipboard, "Send clipboard", Widgets.sendClipboard(context))
                            Spacer(GlanceModifier.defaultWeight())
                            RoundButton(R.drawable.ic_w_upload, "Send files", Widgets.pickFiles(context))
                            Spacer(GlanceModifier.defaultWeight())
                            RoundButton(R.drawable.ic_w_mouse, "Touchpad", Widgets.openApp(context, MainActivity.SCREEN_TOUCHPAD))
                            Spacer(GlanceModifier.defaultWeight())
                            RoundButton(R.drawable.ic_w_ring, "Ring PC", Widgets.command(context, Commands.RING_PC))
                            Spacer(GlanceModifier.defaultWeight())
                            RoundButton(
                                if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, if (s.playing) "Pause" else "Play",
                                Widgets.command(context, Commands.PLAY_PAUSE), accent = true,
                            )
                        }
                    }
                }
            }
        }
    }

    @Composable
    private fun NowPlayingPill(context: Context, s: WidgetSnapshot, modifier: GlanceModifier) {
        Row(
            modifier.cornerRadius(18.dp).background(Ink.glassButton).padding(8.dp).clickable(Widgets.command(context, Commands.PLAY_PAUSE)),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (s.art != null) {
                Image(ImageProvider(s.art), null, contentScale = ContentScale.Crop, modifier = GlanceModifier.size(34.dp).cornerRadius(10.dp))
                Spacer(GlanceModifier.width(8.dp))
            }
            Column(GlanceModifier.defaultWeight()) {
                Text(s.title.orEmpty(), maxLines = 1, style = style(12.sp, Ink.primary, FontWeight.Medium))
                Text(if (s.playing) "Playing" else "Paused", maxLines = 1, style = style(10.sp, Ink.secondary))
            }
        }
    }
}

// ---- 2. quick actions -------------------------------------------------------------------------

class ActionsWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Responsive(setOf(DpSize(260.dp, 100.dp), DpSize(260.dp, 190.dp)))

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme {
            val s = Widgets.snapshot(context)
            val tall = LocalSize.current.height >= 180.dp
            Glass(padding = 14.dp) {
                Column(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    ButtonRow(
                        listOf(
                            Triple(R.drawable.ic_w_clipboard, "Clipboard", Widgets.sendClipboard(context)),
                            Triple(R.drawable.ic_w_upload, "Files", Widgets.pickFiles(context)),
                            Triple(R.drawable.ic_w_mouse, "Touchpad", Widgets.openApp(context, MainActivity.SCREEN_TOUCHPAD)),
                            Triple(if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, if (s.playing) "Pause" else "Play", Widgets.command(context, Commands.PLAY_PAUSE)),
                        ),
                        accentLast = true,
                    )
                    if (tall) {
                        Spacer(GlanceModifier.defaultWeight())
                        ButtonRow(
                            listOf(
                                Triple(R.drawable.ic_w_ring, "Ring PC", Widgets.command(context, Commands.RING_PC)),
                                Triple(R.drawable.ic_w_lock, "Lock PC", Widgets.command(context, Commands.LOCK_PC)),
                                Triple(R.drawable.ic_w_keyboard, "Keyboard", Widgets.openApp(context, MainActivity.SCREEN_KEYBOARD)),
                                Triple(R.drawable.ic_w_bell, "Ping", Widgets.command(context, Commands.PING)),
                            ),
                        )
                    }
                }
            }
        }
    }

    @Composable
    private fun ButtonRow(items: List<Triple<Int, String, Action>>, accentLast: Boolean = false) {
        Row(GlanceModifier.fillMaxWidth()) {
            items.forEachIndexed { i, (icon, label, action) ->
                LabeledButton(icon, label, action, GlanceModifier.defaultWeight(), accent = accentLast && i == items.lastIndex)
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
            if (s.art != null) ArtLayout(context, s, tall) else PlainLayout(context, s)
        }
    }

    /** Album art full-bleed under a scrim, white type: the Now Bar look. */
    @Composable
    private fun ArtLayout(context: Context, s: WidgetSnapshot, tall: Boolean) {
        Box(GlanceModifier.fillMaxSize().cornerRadius(28.dp).clickable(Widgets.openApp(context))) {
            Image(ImageProvider(s.art!!), s.title, contentScale = ContentScale.Crop, modifier = GlanceModifier.fillMaxSize())
            Box(GlanceModifier.fillMaxSize().background(ImageProvider(R.drawable.widget_scrim))) {}
            Column(GlanceModifier.fillMaxSize().padding(16.dp)) {
                Text(s.pcName?.let { "On $it" } ?: "PC", maxLines = 1, style = style(11.sp, Ink.whiteSoft, FontWeight.Medium))
                Spacer(GlanceModifier.defaultWeight())
                Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.Bottom) {
                    Column(GlanceModifier.defaultWeight()) {
                        Text(s.title.orEmpty(), maxLines = if (tall) 2 else 1, style = style(if (tall) 20.sp else 16.sp, Ink.white, FontWeight.Bold))
                        s.artist?.takeIf { it.isNotBlank() }?.let { Text(it, maxLines = 1, style = style(13.sp, Ink.whiteSoft)) }
                    }
                    Spacer(GlanceModifier.width(10.dp))
                    Controls(context, s, onArt = true)
                }
            }
        }
    }

    @Composable
    private fun PlainLayout(context: Context, s: WidgetSnapshot) {
        Glass(GlanceModifier.clickable(Widgets.openApp(context))) {
            Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                Box(GlanceModifier.size(64.dp).cornerRadius(18.dp).background(Ink.glassButton), contentAlignment = Alignment.Center) {
                    Image(ImageProvider(R.drawable.ic_w_music), null, colorFilter = ColorFilter.tint(Ink.secondary), modifier = GlanceModifier.size(28.dp))
                }
                Spacer(GlanceModifier.width(14.dp))
                Column(GlanceModifier.defaultWeight()) {
                    Text(s.title ?: "Nothing playing", maxLines = 2, style = style(16.sp, Ink.primary, FontWeight.Bold))
                    Text(
                        s.artist?.takeIf { it.isNotBlank() } ?: s.pcName?.let { "on $it" } ?: "Play something on your PC",
                        maxLines = 1, style = style(12.sp, Ink.secondary),
                    )
                }
                if (s.title != null) Controls(context, s, onArt = false)
            }
        }
    }

    @Composable
    private fun Controls(context: Context, s: WidgetSnapshot, onArt: Boolean) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            RoundButton(R.drawable.ic_skip_previous, "Previous", Widgets.command(context, Commands.PREVIOUS), size = 38.dp, onArt = onArt)
            Spacer(GlanceModifier.width(6.dp))
            RoundButton(
                if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, if (s.playing) "Pause" else "Play",
                Widgets.command(context, Commands.PLAY_PAUSE), size = 48.dp, accent = true, onArt = onArt,
            )
            Spacer(GlanceModifier.width(6.dp))
            RoundButton(R.drawable.ic_skip_next, "Next", Widgets.command(context, Commands.NEXT), size = 38.dp, onArt = onArt)
        }
    }
}

// ---- 4. clipboard -----------------------------------------------------------------------------

class ClipboardWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Exact

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme {
            val s = Widgets.snapshot(context)
            val rows = ((LocalSize.current.height - 70.dp) / 36.dp).toInt().coerceIn(1, 4)
            Glass {
                Column(GlanceModifier.fillMaxSize()) {
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("Clipboard", style = style(16.sp, Ink.primary, FontWeight.Bold), modifier = GlanceModifier.defaultWeight())
                        Row(
                            GlanceModifier.cornerRadius(18.dp).background(GlanceTheme.colors.primary).padding(horizontal = 14.dp, vertical = 8.dp)
                                .clickable(Widgets.sendClipboard(context)),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Image(ImageProvider(R.drawable.ic_w_send), null, colorFilter = ColorFilter.tint(GlanceTheme.colors.onPrimary), modifier = GlanceModifier.size(14.dp))
                            Spacer(GlanceModifier.width(6.dp))
                            Text("Send", style = style(12.sp, GlanceTheme.colors.onPrimary, FontWeight.Bold))
                        }
                    }
                    Spacer(GlanceModifier.height(10.dp))
                    if (s.clips.isEmpty()) {
                        Text("Copied text shows up here", style = style(12.sp, Ink.secondary))
                    }
                    s.clips.take(rows).forEachIndexed { i, clip ->
                        if (i > 0) Box(GlanceModifier.fillMaxWidth().height(1.dp).background(Ink.hairline)) {}
                        Text(
                            clip, maxLines = 1,
                            style = style(13.sp, if (i == 0) Ink.primary else Ink.secondary, if (i == 0) FontWeight.Medium else FontWeight.Normal),
                            modifier = GlanceModifier.fillMaxWidth().padding(vertical = 9.dp).clickable(Widgets.openApp(context, MainActivity.SCREEN_CLIPBOARD)),
                        )
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
                GlanceModifier.fillMaxSize().background(ImageProvider(R.drawable.widget_accent)).clickable(Widgets.pickFiles(context)).padding(12.dp),
                contentAlignment = Alignment.Center,
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally, verticalAlignment = Alignment.CenterVertically) {
                    val fraction = s.transferFraction
                    Box(GlanceModifier.size(52.dp), contentAlignment = Alignment.Center) {
                        if (fraction != null) {
                            val ring = Rings.ring(context, 52, fraction, android.graphics.Color.WHITE, 0x40FFFFFF)
                            Image(ImageProvider(ring), null, modifier = GlanceModifier.fillMaxSize())
                        } else {
                            Box(GlanceModifier.fillMaxSize().cornerRadius(26.dp).background(Ink.whiteGlass)) {}
                        }
                        Image(ImageProvider(R.drawable.ic_w_upload), "Send files", colorFilter = ColorFilter.tint(Ink.white), modifier = GlanceModifier.size(24.dp))
                    }
                    Spacer(GlanceModifier.height(8.dp))
                    Text(
                        when {
                            fraction != null -> "${(fraction * 100).toInt()}%  ${s.transferLabel.orEmpty()}"
                            s.connected && s.pcName != null -> "Send to ${s.pcName}"
                            else -> "Send files"
                        },
                        maxLines = 1, style = style(12.sp, Ink.white, FontWeight.Bold),
                    )
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
