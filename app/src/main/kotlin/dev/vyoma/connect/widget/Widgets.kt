package dev.vyoma.connect.widget

import android.app.Application
import android.content.Context
import dev.vyoma.connect.protocol.VLog
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.os.Build
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
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
import androidx.glance.LocalGlanceId
import androidx.glance.LocalSize
import androidx.glance.action.Action
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetManager
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.LocalAppWidgetOptions
import androidx.glance.appwidget.appWidgetBackground
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
        if (sampleMode) return
        val snap = read(context)
        if (snap.pcBattery != state.value?.pcBattery) VLog.d("Widgets", "PC battery ${snap.pcBattery}% (connected=${snap.connected})")
        state.value = snap
        withContext(Dispatchers.IO) { runCatching { save(context, snap) } }
        listOf(StatusWidget(), ActionsWidget(), MediaWidget(), ClipboardWidget(), DropWidget()).forEach {
            runCatching { it.updateAll(context) }
        }
    }

    @Volatile private var sampleMode = false

    /** Debug widget lab: show sample content (live refreshes paused) and redraw. */
    suspend fun showSample(context: Context, snap: WidgetSnapshot) {
        sampleMode = true
        state.value = snap
        listOf(StatusWidget(), ActionsWidget(), MediaWidget(), ClipboardWidget(), DropWidget()).forEach {
            runCatching { it.updateAll(context) }
        }
    }

    /**
     * Pushes the generated widget-picker previews (API 35+). The platform rate-limits this, so it
     * runs once per install/update and retries on a later start if any provider was refused.
     */
    suspend fun publishPreviews(context: Context) {
        if (Build.VERSION.SDK_INT < 35) return
        val prefs = context.getSharedPreferences("widgets", Context.MODE_PRIVATE)
        val installed = context.packageManager.getPackageInfo(context.packageName, 0).lastUpdateTime
        if (prefs.getLong("previews_for", 0) == installed) return
        val manager = GlanceAppWidgetManager(context)
        val receivers = listOf(
            StatusWidgetReceiver::class, ActionsWidgetReceiver::class, MediaWidgetReceiver::class,
            ClipboardWidgetReceiver::class, DropWidgetReceiver::class,
        )
        val ok = receivers.all { receiver ->
            runCatching { manager.setWidgetPreviews(receiver) }
                .onFailure { VLog.w("Widgets", "preview for ${receiver.simpleName} failed: ${it.message}") }
                .getOrNull() == GlanceAppWidgetManager.SET_WIDGET_PREVIEWS_RESULT_SUCCESS
        }
        if (ok) prefs.edit().putLong("previews_for", installed).apply()
    }

    /** The latest snapshot: in memory if the service ran in this process, else from disk. */
    internal fun flow(context: Context): StateFlow<WidgetSnapshot?> {
        if (state.value == null) state.compareAndSet(null, runCatching { load(context) }.getOrNull() ?: WidgetSnapshot())
        return state
    }

    @Composable
    internal fun snapshot(context: Context): WidgetSnapshot {
        LocalPreview.current?.let { return it }
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
// One UI widgets: launcher-blurred glass, high-contrast type, round glass buttons, accent used
// sparingly. Measured One UI Home cells (5x6 grid, S24 FE): width 58/129/200/271/342dp for 1..5
// columns, height 58/157/257/356dp for 1..4 rows. Every layout below is checked at those sizes.

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

/** Size classes every widget lays out against (SizeMode.Exact gives the real size). */
private class Dims(val w: Dp, val h: Dp) {
    val oneRow = h < 100.dp          // 58dp
    val oneCol = w < 100.dp          // 58dp
    val narrow = w < 160.dp          // 1-2 columns
    val tall = h >= 240.dp           // 3+ rows
    val pad = when {
        oneCol -> 6.dp
        oneRow -> 12.dp
        narrow -> 14.dp
        else -> 16.dp
    }

    /** One-row widgets are only 58dp tall: keep vertical padding tight so two text lines fit. */
    val padV = if (oneRow || oneCol) 6.dp else pad
    val innerW = w - pad * 2
    val innerH = h - padV * 2

    /** Largest button that fits a one-row widget. */
    val rowButton = minOf(innerH.value, 40f).dp
}

@Composable
private fun dims() = LocalSize.current.let { Dims(it.width, it.height) }

/**
 * The widget's root, tagged as @android:id/background with a translucent plain colour: One UI Home
 * then blurs the wallpaper behind it (with widgetStyle/widgetSize in the provider XML). The root
 * must stay a plain view, so the tap target lives on the content box inside it. The launcher tells
 * us per widget whether it will blur; elsewhere the glass is more opaque so text stays readable.
 */
@Composable
private fun Glass(onClick: Action? = null, content: @Composable () -> Unit) {
    val d = dims()
    val context = LocalContext.current
    val blur = LocalAppWidgetOptions.current.getBoolean("semIsWallpaperBlurSupported", false)
    val night = (context.resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
        android.content.res.Configuration.UI_MODE_NIGHT_YES
    val tint = when {
        night && blur -> Color(0x8C141416)
        night -> Color(0xE6141416)
        blur -> Color(0xB3F5F5F7)
        else -> Color(0xEBF5F5F7)
    }
    Box(
        GlanceModifier.fillMaxSize().appWidgetBackground().cornerRadius(android.R.dimen.system_app_widget_background_radius)
            .background(ColorProvider(tint)),
    ) {
        Box(GlanceModifier.fillMaxSize().background(ImageProvider(R.drawable.widget_sheen))) {}
        val inner = GlanceModifier.fillMaxSize().padding(horizontal = d.pad, vertical = d.padV)
        Box(if (onClick != null) inner.clickable(onClick) else inner) { content() }
    }
}

private fun style(size: TextUnit, color: ColorProvider, weight: FontWeight = FontWeight.Normal) =
    TextStyle(fontSize = size, color = color, fontWeight = weight)

@Composable
private fun HGap(w: Dp) = Spacer(GlanceModifier.width(w))

@Composable
private fun VGap(h: Dp) = Spacer(GlanceModifier.height(h))

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

internal class Act(val icon: Int, val label: String, val action: Action, val accent: Boolean = false)

/** Buttons spread evenly across the full width, labels optional; null slots keep columns aligned. */
@Composable
private fun ButtonRow(items: List<Act?>, size: Dp, labels: Boolean) {
    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        items.forEach { a ->
            if (a == null) {
                Spacer(GlanceModifier.defaultWeight())
                return@forEach
            }
            Column(GlanceModifier.defaultWeight(), horizontalAlignment = Alignment.CenterHorizontally) {
                RoundButton(a.icon, a.label, a.action, size = size, accent = a.accent)
                if (labels) {
                    VGap(6.dp)
                    Text(a.label, maxLines = 1, style = style(11.sp, Ink.secondary, FontWeight.Medium))
                }
            }
        }
    }
}

/** A battery/progress ring as a bitmap (RemoteViews can't draw arcs). Cached by its inputs. */
private object Rings {
    private val cache = object : LinkedHashMap<String, Bitmap>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>) = size > 12
    }

    fun ring(context: Context, sizeDp: Int, fraction: Float, color: Int, track: Int): Bitmap = synchronized(cache) {
        val px = (sizeDp * context.resources.displayMetrics.density).toInt().coerceAtLeast(8)
        val key = "$px:${(fraction * 100).toInt()}:$color:$track"
        cache.getOrPut(key) {
            createBitmap(px, px).also { bmp ->
                val stroke = px * 0.1f
                val r = RectF(stroke / 2, stroke / 2, px - stroke / 2, px - stroke / 2)
                val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE; strokeWidth = stroke; strokeCap = Paint.Cap.ROUND }
                Canvas(bmp).apply {
                    drawArc(r, 0f, 360f, false, p.apply { this.color = track })
                    if (fraction > 0f) drawArc(r, -90f, 360f * fraction.coerceIn(0.005f, 1f), false, p.apply { this.color = color })
                }
            }
        }
    }

    fun color(context: Context, percent: Int?, charging: Boolean) = when {
        charging -> Color(0xFF30D158).toArgb()
        percent != null && percent <= 15 -> Color(0xFFFF453A).toArgb()
        else -> context.getColor(android.R.color.system_accent1_400)
    }

    val track = Color(0x33808080).toArgb()
}

/** Ring with the percentage inside; [label] underneath when given. */
@Composable
private fun BatteryRing(label: String?, percent: Int?, charging: Boolean, sizeDp: Int) {
    val context = LocalContext.current
    val bmp = Rings.ring(context, sizeDp, (percent ?: 0) / 100f, Rings.color(context, percent, charging), Rings.track)
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(GlanceModifier.size(sizeDp.dp), contentAlignment = Alignment.Center) {
            Image(ImageProvider(bmp), null, modifier = GlanceModifier.fillMaxSize())
            Text(percent?.let { "$it" } ?: "–", style = style((sizeDp / 3.2f).coerceAtMost(26f).sp, Ink.primary, FontWeight.Bold))
        }
        if (label != null) {
            VGap(4.dp)
            Text(if (charging) "$label ⚡" else label, maxLines = 1, style = style(11.sp, Ink.secondary, FontWeight.Medium))
        }
    }
}

/** Compact battery readout: small icon, bold percent. */
@Composable
private fun BatteryChip(icon: Int, percent: Int?, charging: Boolean) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Image(ImageProvider(icon), null, colorFilter = ColorFilter.tint(Ink.secondary), modifier = GlanceModifier.size(16.dp))
        HGap(4.dp)
        Text((percent?.let { "$it%" } ?: "–") + if (charging) "⚡" else "", maxLines = 1, style = style(13.sp, Ink.primary, FontWeight.Bold))
    }
}

@Composable
private fun Header(s: WidgetSnapshot, titleSize: TextUnit = 17.sp, modifier: GlanceModifier = GlanceModifier, compact: Boolean = false) {
    Column(modifier) {
        Text(s.pcName ?: "No PC", maxLines = 1, style = style(titleSize, Ink.primary, FontWeight.Bold))
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (s.connected) {
                Box(GlanceModifier.size(6.dp).cornerRadius(3.dp).background(Ink.online)) {}
                HGap(5.dp)
            }
            Text(
                when {
                    s.pcName == null -> "Open Vyoma"
                    s.connected -> "Online"
                    else -> "Offline"
                },
                maxLines = 1,
                style = style(if (compact) 11.sp else 12.sp, if (s.connected) Ink.online else Ink.secondary, FontWeight.Medium),
            )
        }
    }
}

/**
 * Common shape of our widgets: exact sizes on the home screen, and on API 35+ a generated
 * widget-picker preview drawn by the same code with sample data at each grid size, so the
 * picker shows the real layout for whatever size it offers (the XML previewLayout is the fallback).
 */
abstract class VyomaWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Exact
    override val previewSizeMode = SizeMode.Responsive(PREVIEW_SIZES)

    @Composable
    abstract fun Content(context: Context)

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        GlanceTheme { Content(context) }
    }

    override suspend fun providePreview(context: Context, widgetCategory: Int) = provideContent {
        CompositionLocalProvider(LocalPreview provides PREVIEW_SAMPLE) {
            GlanceTheme { Content(context) }
        }
    }

    private companion object {
        // One UI 5x6 home grid cells a picker offers. RemoteViews caps a size map at 16 entries.
        val PREVIEW_SIZES = listOf(1 to 1, 2 to 1, 4 to 1, 2 to 2, 3 to 2, 4 to 2, 5 to 2, 2 to 3, 4 to 3, 4 to 4)
            .mapTo(mutableSetOf()) { (c, r) -> DpSize((71.1f * c - 13f).dp, (99.2f * r - 41f).dp) }
    }
}

private val LocalPreview = compositionLocalOf<WidgetSnapshot?> { null }

private val PREVIEW_SAMPLE = WidgetSnapshot(
    pcName = "My PC", connected = true, pcBattery = 82, phoneBattery = 64, phoneCharging = true,
    title = "Song title", artist = "Artist", playing = true,
    clips = listOf("https://kde.org/connect", "Meeting at 4:30", "Wi-Fi password", "Grocery list"),
)

// ---- 1. device status -------------------------------------------------------------------------

class StatusWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context) {
        val s = Widgets.snapshot(context)
        val d = dims()
        val pc = s.pcBattery.takeIf { s.connected }
        val pcCharging = s.pcCharging && s.connected
        Glass(onClick = Widgets.openApp(context)) {
            when {
                // 1x1: just the PC's battery ring.
                d.oneCol -> Box(GlanceModifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    BatteryRing(null, pc, pcCharging, minOf(d.innerW, d.innerH).value.toInt())
                }
                // n x 1: ring, name and status; batteries as chips when wide.
                d.oneRow -> Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    BatteryRing(null, pc, pcCharging, d.innerH.value.toInt())
                    HGap(10.dp)
                    Header(s, 14.sp, GlanceModifier.defaultWeight(), compact = true)
                    if (d.innerW >= 160.dp) {
                        HGap(8.dp)
                        BatteryChip(R.drawable.ic_w_phone, s.phoneBattery, s.phoneCharging)
                    }
                    if (d.innerW >= 300.dp) {
                        HGap(12.dp)
                        RoundButton(
                            if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, if (s.playing) "Pause" else "Play",
                            Widgets.command(context, Commands.PLAY_PAUSE), size = d.rowButton, accent = true,
                        )
                    }
                }
                // 2 columns: name, then a big centred ring (and the phone's when tall).
                d.narrow -> Column(GlanceModifier.fillMaxSize()) {
                    Header(s, 15.sp)
                    Spacer(GlanceModifier.defaultWeight())
                    // Header ~40dp; each ring carries a ~20dp label; 8dp between two rings.
                    val ring = minOf(d.innerW.value, if (d.tall) (d.innerH.value - 40 - 40 - 8 - 8) / 2f else d.innerH.value - 40 - 20 - 4).toInt().coerceIn(32, 84)
                    Row(GlanceModifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                        BatteryRing("PC", pc, pcCharging, ring)
                    }
                    if (d.tall) {
                        VGap(8.dp)
                        Row(GlanceModifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                            BatteryRing("Phone", s.phoneBattery, s.phoneCharging, ring)
                        }
                    }
                    Spacer(GlanceModifier.defaultWeight())
                }
                // 3-5 x 2: header with battery chips, action row.
                !d.tall -> Column(GlanceModifier.fillMaxSize()) {
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Header(s, modifier = GlanceModifier.defaultWeight())
                        BatteryChip(R.drawable.ic_w_laptop, pc, pcCharging)
                        if (d.innerW >= 220.dp) {
                            HGap(12.dp)
                            BatteryChip(R.drawable.ic_w_phone, s.phoneBattery, s.phoneCharging)
                        }
                    }
                    Spacer(GlanceModifier.defaultWeight())
                    val n = (d.innerW.value / 56).toInt().coerceIn(3, 5)
                    ButtonRow(statusActions(context, s, n), size = 44.dp, labels = false)
                }
                // 3+ x 3+: big rings, now playing, labelled actions.
                else -> Column(GlanceModifier.fillMaxSize()) {
                    Header(s, 18.sp)
                    Spacer(GlanceModifier.defaultWeight())
                    val ring = ((d.innerH.value - 40 - 68 - 32) * 0.9f).toInt().coerceIn(44, 84)
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        BatteryRing("PC", pc, pcCharging, ring)
                        HGap(16.dp)
                        BatteryRing("Phone", s.phoneBattery, s.phoneCharging, ring)
                        if (s.title != null && d.innerW.value - ring * 2 - 32 >= 110) {
                            HGap(16.dp)
                            NowPlaying(context, s, GlanceModifier.defaultWeight())
                        }
                    }
                    Spacer(GlanceModifier.defaultWeight())
                    val n = (d.innerW.value / 56).toInt().coerceIn(3, 5)
                    ButtonRow(statusActions(context, s, n), size = 44.dp, labels = true)
                }
            }
        }
    }

    /** [n] buttons: play/pause always (last, so the accent ends the row), then clipboard, files, touchpad, lock. */
    private fun statusActions(context: Context, s: WidgetSnapshot, n: Int) =
        (listOf(QuickAction.CLIPBOARD, QuickAction.FILES, QuickAction.TOUCHPAD, QuickAction.LOCK).take(n - 1) + QuickAction.PLAY_PAUSE)
            .map { it.button(context, s) }

    @Composable
    private fun NowPlaying(context: Context, s: WidgetSnapshot, modifier: GlanceModifier) {
        Row(
            modifier.cornerRadius(18.dp).background(Ink.glassButton).padding(8.dp).clickable(Widgets.command(context, Commands.PLAY_PAUSE)),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (s.art != null) {
                Image(ImageProvider(s.art), null, contentScale = ContentScale.Crop, modifier = GlanceModifier.size(36.dp).cornerRadius(10.dp))
                HGap(8.dp)
            }
            Column(GlanceModifier.defaultWeight()) {
                Text(s.title.orEmpty(), maxLines = 1, style = style(12.sp, Ink.primary, FontWeight.Medium))
                Text(if (s.playing) "Playing" else "Paused", maxLines = 1, style = style(10.sp, Ink.secondary))
            }
        }
    }
}

// ---- 2. quick actions -------------------------------------------------------------------------

class ActionsWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context) {
        val s = Widgets.snapshot(context)
        val d = dims()
        val picked = if (LocalPreview.current != null) QuickAction.DEFAULT else {
            val glanceId = LocalGlanceId.current
            QuickActionStore.get(context, remember(glanceId) { GlanceAppWidgetManager(context).getAppWidgetId(glanceId) })
        }
        Glass {
            // Two candidate grids: labelled (58dp columns, 76dp rows) and icon-only. Labels win
            // only when they don't cost buttons, so a 2x2 shows four icons, not one labelled button.
            val iconSize = d.rowButton.coerceAtMost(44.dp)
            val iconCols = ((d.innerW.value + 8) / (iconSize.value + 8)).toInt().coerceIn(1, 6)
            val iconRows = ((d.innerH.value + 8) / (iconSize.value + 8)).toInt().coerceIn(1, 4)
            val labelCols = (d.innerW.value / 58).toInt().coerceIn(1, 6)
            val labelRows = ((d.innerH.value + 12) / 76).toInt().coerceIn(0, 4)
            val count = picked.size.coerceAtLeast(1)
            val labels = !d.oneRow && labelRows > 0 && labelCols * labelRows >= minOf(iconCols * iconRows, count)
            val size = if (labels) 44.dp else iconSize
            val maxCols = if (labels) labelCols else iconCols
            // No more rows than the actions need, then full rows (3 + 3 rather than 4 + 2).
            val rows = minOf(if (labels) labelRows else iconRows, (count + maxCols - 1) / maxCols)
            val cols = minOf(maxCols, (count + rows - 1) / rows)
            // Play/pause leads when space is tight: it's the one people reach for most.
            val all = (if (cols * rows < 4 && QuickAction.PLAY_PAUSE in picked) listOf(QuickAction.PLAY_PAUSE) + (picked - QuickAction.PLAY_PAUSE) else picked)
                .map { it.button(context, s) }
            val items = all.take(cols * rows)
            Column(GlanceModifier.fillMaxSize()) {
                // Rows spaced evenly, including above the first and below the last.
                items.chunked(cols).forEach { row ->
                    Spacer(GlanceModifier.defaultWeight())
                    ButtonRow(row + List(cols - row.size) { null }, size, labels)
                }
                Spacer(GlanceModifier.defaultWeight())
            }
        }
    }
}

// ---- 3. PC media ------------------------------------------------------------------------------

class MediaWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context) {
        val s = Widgets.snapshot(context)
        val d = dims()
        when {
            d.oneRow || d.oneCol -> Compact(context, s, d)
            s.art != null -> ArtLayout(context, s, d)
            else -> PlainLayout(context, s, d)
        }
    }

    /** Album art full-bleed under a scrim, white type: the Now Bar look. */
    @Composable
    private fun ArtLayout(context: Context, s: WidgetSnapshot, d: Dims) {
        Box(GlanceModifier.fillMaxSize().appWidgetBackground().cornerRadius(android.R.dimen.system_app_widget_background_radius)) {
            Image(ImageProvider(s.art!!), s.title, contentScale = ContentScale.Crop, modifier = GlanceModifier.fillMaxSize())
            Box(GlanceModifier.fillMaxSize().background(ImageProvider(R.drawable.widget_scrim))) {}
            Column(GlanceModifier.fillMaxSize().padding(horizontal = d.pad, vertical = d.padV).clickable(Widgets.openApp(context))) {
                Text(s.pcName?.let { "On $it" } ?: "PC", maxLines = 1, style = style(11.sp, Ink.whiteSoft, FontWeight.Medium))
                Spacer(GlanceModifier.defaultWeight())
                if (d.narrow) {
                    Text(s.title.orEmpty(), maxLines = 2, style = style(14.sp, Ink.white, FontWeight.Bold))
                    VGap(8.dp)
                    Row(GlanceModifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                        Controls(context, s, onArt = true, full = d.innerW >= 130.dp, size = 40.dp)
                    }
                } else {
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.Bottom) {
                        Column(GlanceModifier.defaultWeight()) {
                            Text(s.title.orEmpty(), maxLines = if (d.tall) 2 else 1, style = style(if (d.tall) 20.sp else 16.sp, Ink.white, FontWeight.Bold))
                            s.artist?.takeIf { it.isNotBlank() }?.let { Text(it, maxLines = 1, style = style(13.sp, Ink.whiteSoft)) }
                        }
                        HGap(12.dp)
                        Controls(context, s, onArt = true, full = d.innerW >= 220.dp)
                    }
                }
            }
        }
    }

    @Composable
    private fun PlainLayout(context: Context, s: WidgetSnapshot, d: Dims) {
        Glass(onClick = Widgets.openApp(context)) {
            if (d.narrow) {
                Column(GlanceModifier.fillMaxSize()) {
                    Thumb(s, 40.dp)
                    Spacer(GlanceModifier.defaultWeight())
                    Text(s.title ?: "Nothing playing", maxLines = 2, style = style(14.sp, Ink.primary, FontWeight.Bold))
                    if (s.title != null) {
                        VGap(8.dp)
                        Controls(context, s, onArt = false, full = d.innerW >= 130.dp, size = 40.dp)
                    }
                }
            } else {
                Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    Thumb(s, 64.dp)
                    HGap(14.dp)
                    Column(GlanceModifier.defaultWeight()) {
                        Text(s.title ?: "Nothing playing", maxLines = 2, style = style(16.sp, Ink.primary, FontWeight.Bold))
                        Text(
                            s.artist?.takeIf { it.isNotBlank() } ?: s.pcName?.let { "on $it" } ?: "Play something on your PC",
                            maxLines = 1, style = style(12.sp, Ink.secondary),
                        )
                    }
                    if (s.title != null) {
                        HGap(8.dp)
                        Controls(context, s, onArt = false, full = d.innerW >= 230.dp)
                    }
                }
            }
        }
    }

    /** One row (or one column): thumbnail, title, play; trimmed to what fits. */
    @Composable
    private fun Compact(context: Context, s: WidgetSnapshot, d: Dims) {
        Glass(onClick = Widgets.openApp(context)) {
            if (d.oneCol) {
                Box(GlanceModifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    RoundButton(
                        if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, if (s.playing) "Pause" else "Play",
                        Widgets.command(context, Commands.PLAY_PAUSE), size = minOf(d.innerW, d.innerH), accent = true,
                    )
                }
                return@Glass
            }
            Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                if (d.innerW >= 160.dp) {
                    Thumb(s, d.innerH.coerceAtMost(44.dp))
                    HGap(10.dp)
                }
                Column(GlanceModifier.defaultWeight()) {
                    Text(s.title ?: "Nothing playing", maxLines = 1, style = style(13.sp, Ink.primary, FontWeight.Bold))
                    s.artist?.takeIf { it.isNotBlank() && d.innerW >= 160.dp }?.let { Text(it, maxLines = 1, style = style(11.sp, Ink.secondary)) }
                }
                if (s.title != null) {
                    HGap(6.dp)
                    Controls(context, s, onArt = false, full = d.innerW >= 240.dp, size = d.rowButton)
                }
            }
        }
    }

    @Composable
    private fun Thumb(s: WidgetSnapshot, size: Dp) {
        if (s.art != null) {
            Image(ImageProvider(s.art), null, contentScale = ContentScale.Crop, modifier = GlanceModifier.size(size).cornerRadius(size * 0.28f))
        } else {
            Box(GlanceModifier.size(size).cornerRadius(size * 0.28f).background(Ink.glassButton), contentAlignment = Alignment.Center) {
                Image(ImageProvider(R.drawable.ic_w_music), null, colorFilter = ColorFilter.tint(Ink.secondary), modifier = GlanceModifier.size(size * 0.45f))
            }
        }
    }

    @Composable
    private fun Controls(context: Context, s: WidgetSnapshot, onArt: Boolean, full: Boolean, size: Dp = 48.dp) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            val side = size * 0.8f
            if (full) {
                RoundButton(R.drawable.ic_skip_previous, "Previous", Widgets.command(context, Commands.PREVIOUS), size = side, onArt = onArt)
                HGap(6.dp)
            }
            RoundButton(
                if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, if (s.playing) "Pause" else "Play",
                Widgets.command(context, Commands.PLAY_PAUSE), size = size, accent = true, onArt = onArt,
            )
            if (full) {
                HGap(6.dp)
                RoundButton(R.drawable.ic_skip_next, "Next", Widgets.command(context, Commands.NEXT), size = side, onArt = onArt)
            }
        }
    }
}

// ---- 4. clipboard -----------------------------------------------------------------------------

class ClipboardWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context) {
        val s = Widgets.snapshot(context)
        val d = dims()
        val send = Widgets.sendClipboard(context)
        val open = Widgets.openApp(context, MainActivity.SCREEN_CLIPBOARD)
        Glass {
            when {
                d.oneCol -> Box(GlanceModifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    RoundButton(R.drawable.ic_w_send, "Send clipboard", send, size = minOf(d.innerW, d.innerH), accent = true)
                }
                // One row: the latest copy and a send button.
                d.oneRow -> Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    Column(GlanceModifier.defaultWeight().clickable(open)) {
                        // Two columns only fit a few characters of the clip: just name the widget.
                        if (d.narrow) {
                            Text("Send", maxLines = 1, style = style(13.sp, Ink.primary, FontWeight.Bold))
                            Text("clipboard", maxLines = 1, style = style(11.sp, Ink.secondary, FontWeight.Medium))
                        } else {
                            Text("Clipboard", maxLines = 1, style = style(11.sp, Ink.secondary, FontWeight.Medium))
                            Text(s.clips.firstOrNull() ?: "Nothing copied yet", maxLines = 1, style = style(13.sp, Ink.primary, FontWeight.Medium))
                        }
                    }
                    HGap(8.dp)
                    RoundButton(R.drawable.ic_w_send, "Send clipboard", send, size = d.rowButton, accent = true)
                }
                else -> Column(GlanceModifier.fillMaxSize()) {
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("Clipboard", maxLines = 1, style = style(if (d.narrow) 14.sp else 16.sp, Ink.primary, FontWeight.Bold), modifier = GlanceModifier.defaultWeight())
                        RoundButton(R.drawable.ic_w_send, "Send clipboard", send, size = if (d.narrow) 32.dp else 36.dp, accent = true)
                    }
                    VGap(6.dp)
                    if (s.clips.isEmpty()) {
                        Text("Copied text shows up here", style = style(12.sp, Ink.secondary))
                    }
                    // Each row is ~36dp: 9dp padding, one 13sp line, 1dp hairline.
                    val rows = ((d.innerH.value - 42) / 36).toInt().coerceIn(1, 6)
                    s.clips.take(rows).forEachIndexed { i, clip ->
                        if (i > 0) Box(GlanceModifier.fillMaxWidth().height(1.dp).background(Ink.hairline)) {}
                        Text(
                            clip, maxLines = 1,
                            style = style(13.sp, if (i == 0) Ink.primary else Ink.secondary, if (i == 0) FontWeight.Medium else FontWeight.Normal),
                            modifier = GlanceModifier.fillMaxWidth().padding(vertical = 8.dp).clickable(open),
                        )
                    }
                }
            }
        }
    }
}

// ---- 5. file drop -----------------------------------------------------------------------------

class DropWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context) {
        val s = Widgets.snapshot(context)
        val d = dims()
        val fraction = s.transferFraction
        val label = when {
            fraction != null -> "${(fraction * 100).toInt()}%"
            s.connected && s.pcName != null -> "Send to ${s.pcName}"
            else -> "Send files"
        }
        Box(
            GlanceModifier.fillMaxSize().appWidgetBackground().cornerRadius(android.R.dimen.system_app_widget_background_radius)
                .background(ImageProvider(R.drawable.widget_accent)),
        ) {
            Box(GlanceModifier.fillMaxSize().padding(horizontal = d.pad, vertical = d.padV).clickable(Widgets.pickFiles(context)), contentAlignment = Alignment.Center) {
                when {
                    d.oneCol -> Circle(context, fraction, minOf(d.innerW, d.innerH).value.toInt())
                    // One row: circle and label side by side.
                    d.oneRow -> Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                        Circle(context, fraction, d.innerH.value.toInt())
                        HGap(10.dp)
                        if (d.narrow && fraction == null) {
                            // Too narrow for "Send to <pc>": a short two-line label instead.
                            Column(GlanceModifier.defaultWeight()) {
                                Text("Send", maxLines = 1, style = style(13.sp, Ink.white, FontWeight.Bold))
                                Text("files", maxLines = 1, style = style(11.sp, Ink.whiteSoft, FontWeight.Medium))
                            }
                        } else {
                            Text(label, maxLines = 1, style = style(13.sp, Ink.white, FontWeight.Bold), modifier = GlanceModifier.defaultWeight())
                        }
                    }
                    else -> Column(horizontalAlignment = Alignment.CenterHorizontally, verticalAlignment = Alignment.CenterVertically) {
                        Circle(context, fraction, 52)
                        VGap(8.dp)
                        Text(label, maxLines = 1, style = style(12.sp, Ink.white, FontWeight.Bold))
                    }
                }
            }
        }
    }

    @Composable
    private fun Circle(context: Context, fraction: Float?, size: Int) {
        Box(GlanceModifier.size(size.dp), contentAlignment = Alignment.Center) {
            if (fraction != null) {
                Image(ImageProvider(Rings.ring(context, size, fraction, android.graphics.Color.WHITE, 0x40FFFFFF)), null, modifier = GlanceModifier.fillMaxSize())
            } else {
                Box(GlanceModifier.fillMaxSize().cornerRadius((size / 2).dp).background(Ink.whiteGlass)) {}
            }
            Image(ImageProvider(R.drawable.ic_w_upload), "Send files", colorFilter = ColorFilter.tint(Ink.white), modifier = GlanceModifier.size((size * 0.46f).dp))
        }
    }
}

class StatusWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = StatusWidget() }
class ActionsWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = ActionsWidget()

    override fun onDeleted(context: Context, appWidgetIds: IntArray) {
        super.onDeleted(context, appWidgetIds)
        QuickActionStore.remove(context, appWidgetIds)
    }
}
class MediaWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = MediaWidget() }
class ClipboardWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = ClipboardWidget() }
class DropWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = DropWidget() }
