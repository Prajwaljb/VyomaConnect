package dev.vyoma.connect.widget

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Shader
import android.text.format.DateFormat
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.graphics.ColorUtils
import androidx.core.graphics.createBitmap
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
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.appWidgetBackground
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
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
import androidx.palette.graphics.Palette
import dev.vyoma.connect.R
import dev.vyoma.connect.service.Commands
import dev.vyoma.connect.ui.MainActivity
import java.util.Date

// Vyoma's widgets, planned from scratch (2026-11): four, each with one job, in the app's own look.
//  - PC: is it on, and how charged (it and the phone), as the app's battery rings.
//  - Shortcuts: Home's coloured tiles, picked per widget; Send files rings with a transfer's progress.
//    At 1x1 the tile is the whole widget (what the old "File drop" widget did).
//  - Now playing: the media card: the cover fading into its own deep colour, white type.
//  - Clipboard: the newest copies, one tap to send; hidden while the app lock is on.
// Every widget says when the PC is away or off (and since when), in words, never a spinner, and
// draws only from the cached snapshot. Layouts follow the real size: one row is 58 dp on One UI
// Home (5x6, S24 FE: 58/157/257/356 dp for 1..4 rows), about 70 on 4x6 grids (ColorOS, HyperOS)
// and 102 on Pixel Launcher; each is checked at all three in the debug widget lab
// (WidgetLabActivity, --es grid oneui|pixel|compact), dark and light, and at a large font size.

// ---- look -------------------------------------------------------------------------------------

private fun tone(light: Long, dark: Long) = ColorProvider(day = Color(light), night = Color(dark))

private object Ink {
    val primary = tone(0xFF0B0B0F, 0xFFF5F5F7)
    val secondary = tone(0x8C000000, 0x99FFFFFF)
    val chip = tone(0x0F000000, 0x1AFFFFFF)
    val hairline = tone(0x14000000, 0x1FFFFFFF)
    val white = tone(0xFFFFFFFF, 0xFFFFFFFF)
    val whiteSoft = tone(0xB3FFFFFF, 0xB3FFFFFF)
    val dark = tone(0xFF1C1C1E, 0xFF1C1C1E)
    val green = tone(0xFF1FA855, 0xFF30D158)
}

/** The app's system colours (designsystem SystemColor), light and dark. */
internal enum class Hue(val light: Long, val dark: Long) {
    BLUE(0xFF007AFF, 0xFF0A84FF),
    TEAL(0xFF30B0C7, 0xFF40C8E0),
    ORANGE(0xFFFF9500, 0xFFFF9F0A),
    PINK(0xFFFF2D55, 0xFFFF375F),
    MINT(0xFF00C7BE, 0xFF63E6E2),
    CYAN(0xFF32ADE6, 0xFF64D2FF),
    GRAY(0xFF8E8E93, 0xFF8E8E93),
    GREEN(0xFF34C759, 0xFF30D158),
    RED(0xFFFF3B30, 0xFFFF453A),
    ;

    /**
     * As a solid tile with white ink: the Home tiles' rule (mid tones at 38% lightness), and darker
     * wherever white would still fall under 3.5:1, in both themes (dark mode's mint is bright).
     */
    val fill: ColorProvider get() = ColorProvider(day = Color(tileFill(light)), night = Color(tileFill(dark)))

    /** The same fill, faded: the PC isn't there to do it. */
    val faded: ColorProvider get() = ColorProvider(day = Color(tileFill(light)).copy(alpha = 0.45f), night = Color(tileFill(dark)).copy(alpha = 0.4f))
    val ink: ColorProvider get() = ColorProvider(day = Color(light), night = Color(dark))

    private fun tileFill(argb: Long): Int {
        val c = argb.toInt()
        if (ColorUtils.calculateLuminance(c) < 0.2) return c
        val hsl = FloatArray(3).also { ColorUtils.colorToHSL(c, it) }
        hsl[2] = minOf(hsl[2], 0.38f)
        var out = ColorUtils.HSLToColor(hsl)
        while (ColorUtils.calculateLuminance(out) > 0.25 && hsl[2] > 0.1f) {
            hsl[2] -= 0.02f
            out = ColorUtils.HSLToColor(hsl)
        }
        return out
    }
}

/** Size classes every widget lays out against (SizeMode.Exact gives the real size). */
private class Dims(val w: Dp, val h: Dp) {
    /** One row on any launcher (58 to 102 dp). */
    val oneRow = h < 120.dp
    val oneCol = w < 100.dp          // 57 to 70 dp
    val narrow = w < 180.dp          // 1-2 columns
    val tall = h >= 240.dp           // 3+ rows
    val pad = when {
        oneCol -> 6.dp
        h < 90.dp -> 10.dp
        narrow -> 14.dp
        else -> 16.dp
    }

    /** A 58 dp row keeps its vertical padding tight so two text lines fit. */
    val padV = when {
        h < 90.dp -> 8.dp
        oneCol -> 6.dp
        else -> pad
    }
    val innerW = w - pad * 2
    val innerH = h - padV * 2
}

@Composable
private fun dims() = LocalSize.current.let { Dims(it.width, it.height) }

/**
 * Text follows the phone's font size up to a point, then stops: a widget's box doesn't grow with
 * it, so past that a title would be cut off (iOS caps widget text the same way).
 */
private const val MAX_TEXT_SCALE = 1.15f
private val LocalTextScale = compositionLocalOf { 1f }

@Composable
private fun style(size: TextUnit, color: ColorProvider, weight: FontWeight = FontWeight.Normal) =
    TextStyle(fontSize = (size.value * LocalTextScale.current).sp, color = color, fontWeight = weight)

@Composable
private fun HGap(w: Dp) = Spacer(GlanceModifier.width(w))

@Composable
private fun VGap(h: Dp) = Spacer(GlanceModifier.height(h))

@Composable
private fun androidx.glance.layout.ColumnScope.Flex() = Spacer(GlanceModifier.defaultWeight())

/**
 * The root of a glass widget, tagged as @android:id/background with a translucent plain colour:
 * One UI Home then blurs the wallpaper behind it (widgetStyle colorful in the provider XML). The
 * launcher says per widget whether it will blur; elsewhere the glass is more opaque so text reads.
 * The root stays a plain view, so the tap target is the content box inside it.
 */
@Composable
private fun Glass(onClick: Action? = null, pad: Boolean = true, content: @Composable () -> Unit) {
    val d = dims()
    val blur = LocalAppWidgetOptions.current.getBoolean("semIsWallpaperBlurSupported", false)
    val tint = if (blur) tone(0xB3F5F5F7, 0x8C141416) else tone(0xEBF5F5F7, 0xE6141416)
    Box(GlanceModifier.fillMaxSize().appWidgetBackground().cornerRadius(android.R.dimen.system_app_widget_background_radius).background(tint)) {
        Box(GlanceModifier.fillMaxSize().background(ImageProvider(R.drawable.widget_sheen))) {}
        val inner = GlanceModifier.fillMaxSize().let { if (pad) it.padding(horizontal = d.pad, vertical = d.padV) else it }
        Box(if (onClick != null) inner.clickable(onClick) else inner) { content() }
    }
}

/** A solid root in one colour (a whole-widget tile, the media card's cover colour). */
@Composable
private fun Solid(color: ColorProvider, onClick: Action?, content: @Composable () -> Unit) {
    Box(GlanceModifier.fillMaxSize().appWidgetBackground().cornerRadius(android.R.dimen.system_app_widget_background_radius).background(color)) {
        val inner = GlanceModifier.fillMaxSize()
        Box(if (onClick != null) inner.clickable(onClick) else inner, contentAlignment = Alignment.Center) { content() }
    }
}

@Composable
private fun Glyph(icon: Int, size: Dp, tint: ColorProvider, desc: String? = null, modifier: GlanceModifier = GlanceModifier) =
    Image(ImageProvider(icon), desc, colorFilter = ColorFilter.tint(tint), modifier = modifier.size(size))

/** A coloured rounded square with a white glyph: the app's icon badges. */
@Composable
private fun Badge(icon: Int, hue: Hue, size: Dp) {
    Box(GlanceModifier.size(size).cornerRadius(size * 0.3f).background(hue.fill), contentAlignment = Alignment.Center) {
        Glyph(icon, size * 0.58f, Ink.white)
    }
}

/** Rings drawn into bitmaps (RemoteViews can't draw arcs), the app's 6 dp round-capped strokes. */
private object Rings {
    private val cache = object : LinkedHashMap<String, Bitmap>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>) = size > 16
    }

    fun ring(context: Context, sizeDp: Float, fraction: Float, color: Int, track: Int, strokeDp: Float): Bitmap = synchronized(cache) {
        val density = context.resources.displayMetrics.density
        val px = (sizeDp * density).toInt().coerceAtLeast(8)
        val key = "$px:${(fraction * 200).toInt()}:$color:$track:$strokeDp"
        cache.getOrPut(key) {
            createBitmap(px, px).also { bmp ->
                val stroke = strokeDp * density
                val r = RectF(stroke / 2, stroke / 2, px - stroke / 2, px - stroke / 2)
                val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE; strokeWidth = stroke; strokeCap = Paint.Cap.ROUND }
                Canvas(bmp).apply {
                    drawArc(r, 0f, 360f, false, p.apply { this.color = track })
                    if (fraction > 0f) drawArc(r, -90f, 360f * fraction.coerceIn(0.005f, 1f), false, p.apply { this.color = color })
                }
            }
        }
    }
}

/** Night or day, as the launcher draws it now (a redraw follows a theme change). */
private fun Context.night() =
    resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK == android.content.res.Configuration.UI_MODE_NIGHT_YES

/**
 * A battery ring as on Home: green, red at 15% and below unless it's charging; grey when the
 * number is from before the PC went away. Inside: the percent, or a device glyph when [glyph] is set.
 */
@Composable
private fun BatteryRing(percent: Int?, charging: Boolean, size: Dp, stale: Boolean = false, glyph: Int? = null) {
    val context = LocalContext.current
    val night = context.night()
    val color = when {
        stale || percent == null -> if (night) 0xFF636366.toInt() else 0xFFAEAEB2.toInt()
        percent <= 15 && !charging -> (if (night) Hue.RED.dark else Hue.RED.light).toInt()
        else -> (if (night) Hue.GREEN.dark else Hue.GREEN.light).toInt()
    }
    val track = if (night) 0x33FFFFFF else 0x1F000000
    val stroke = if (size >= 72.dp) 7f else if (size >= 40.dp) 6f else 4.5f
    val bmp = Rings.ring(context, size.value, (percent ?: 0) / 100f, color, track, stroke)
    Box(GlanceModifier.size(size), contentAlignment = Alignment.Center) {
        Image(ImageProvider(bmp), null, modifier = GlanceModifier.fillMaxSize())
        when {
            glyph != null -> Glyph(glyph, size * 0.4f, if (stale) Ink.secondary else Ink.primary)
            else -> Row(verticalAlignment = Alignment.Bottom) {
                Text(percent?.toString() ?: "–", maxLines = 1, style = style((size.value / 3f).coerceIn(11f, 28f).sp, if (stale) Ink.secondary else Ink.primary, FontWeight.Bold))
                if (percent != null && size >= 64.dp) Text("%", maxLines = 1, style = style((size.value / 6.5f).sp, Ink.secondary, FontWeight.Medium))
            }
        }
    }
}

/** "82%" with the unit smaller and quieter, as every reading in the app; a bolt while charging. */
@Composable
private fun Reading(percent: Int?, charging: Boolean, size: TextUnit, stale: Boolean = false) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(percent?.toString() ?: "–", maxLines = 1, style = style(size, if (stale) Ink.secondary else Ink.primary, FontWeight.Bold))
        if (percent != null) Text("%", maxLines = 1, style = style((size.value * 0.6f).sp, Ink.secondary, FontWeight.Medium))
        if (charging && !stale) {
            HGap(2.dp)
            Glyph(R.drawable.ic_w_bolt, (size.value * 0.85f).dp, Ink.green)
        }
    }
}

/** What the widgets say about the PC, in words. */
private fun linkWords(context: Context, s: WidgetSnapshot, short: Boolean = false): String {
    val at = s.since?.let { " since " + DateFormat.getTimeFormat(context).format(Date(it)) }.orEmpty().takeUnless { short }.orEmpty()
    return when (s.link) {
        PcLink.NONE -> "Pair a PC"
        PcLink.CONNECTED -> "Connected"
        PcLink.AWAY -> "Away$at"
        PcLink.OFF -> when (s.offWhy) {
            "sleep" -> "Asleep$at"
            "shutdown" -> "Switched off$at"
            else -> "Vyoma closed$at"
        }
    }
}

/** The PC's name and where it is, one line each; a green dot when connected. */
@Composable
private fun Header(s: WidgetSnapshot, title: TextUnit, modifier: GlanceModifier = GlanceModifier, short: Boolean = false) {
    val context = LocalContext.current
    Column(modifier) {
        Text(s.pcName ?: "Vyoma", maxLines = 1, style = style(title, Ink.primary, FontWeight.Bold))
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (s.connected) {
                Box(GlanceModifier.size(6.dp).cornerRadius(3.dp).background(Ink.green)) {}
                HGap(5.dp)
            }
            Text(
                linkWords(context, s, short), maxLines = 1,
                style = style((title.value * 0.75f).coerceAtLeast(11f).sp, if (s.connected) Ink.green else Ink.secondary, FontWeight.Medium),
            )
        }
    }
}

/**
 * Common shape of our widgets: exact sizes on the home screen, and on API 35+ a generated
 * widget-picker preview drawn by the same code with sample data at each grid size (the XML
 * previewLayout is the fallback).
 */
abstract class VyomaWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Exact
    override val previewSizeMode = SizeMode.Responsive(PREVIEW_SIZES)

    @Composable
    internal abstract fun Content(context: Context, s: WidgetSnapshot)

    override suspend fun provideGlance(context: Context, id: GlanceId) = provideContent {
        val scale = 1f / maxOf(1f, LocalContext.current.resources.configuration.fontScale / MAX_TEXT_SCALE)
        val s = Widgets.snapshot(context)
        CompositionLocalProvider(LocalTextScale provides scale) {
            GlanceTheme { Content(context, s) }
        }
    }

    override suspend fun providePreview(context: Context, widgetCategory: Int) = provideContent {
        CompositionLocalProvider(LocalPreview provides PREVIEW_SAMPLE) {
            GlanceTheme { Content(context, PREVIEW_SAMPLE) }
        }
    }

    private companion object {
        // One UI 5x6 home grid cells a picker offers. RemoteViews caps a size map at 16 entries.
        val PREVIEW_SIZES = listOf(1 to 1, 2 to 1, 4 to 1, 2 to 2, 3 to 2, 4 to 2, 5 to 2, 2 to 3, 4 to 3, 4 to 4)
            .mapTo(mutableSetOf()) { (c, r) -> DpSize((71.1f * c - 13f).dp, (99.2f * r - 41f).dp) }
    }
}

internal val LocalPreview = compositionLocalOf<WidgetSnapshot?> { null }

private val PREVIEW_SAMPLE = WidgetSnapshot(
    pcName = "My PC", link = PcLink.CONNECTED, pcBattery = 82, phoneBattery = 64, phoneCharging = true,
    title = "Song title", artist = "Artist", playing = true,
    clips = listOf("https://kde.org/connect", "Meeting at 4:30", "Wi-Fi password", "Grocery list"),
)

// ---- 1. PC ------------------------------------------------------------------------------------

class PcWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context, s: WidgetSnapshot) {
        val d = dims()
        val stale = !s.connected
        val pcCharging = s.pcCharging && s.connected
        Glass(onClick = Widgets.openApp(context)) {
            when {
                // 1x1: the PC's ring, the percent inside (grey once it's gone).
                d.oneCol -> Box(GlanceModifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    BatteryRing(s.pcBattery, pcCharging, minOf(d.innerW, d.innerH), stale, glyph = if (s.pcBattery == null) R.drawable.ic_w_laptop else null)
                }
                // Two columns, one row: no room for the name; the PC in the ring, its charge and where it is.
                d.oneRow && d.narrow -> Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    BatteryRing(s.pcBattery, pcCharging, minOf(d.innerH, 42.dp), stale, glyph = R.drawable.ic_w_laptop)
                    HGap(8.dp)
                    Column(GlanceModifier.defaultWeight()) {
                        Reading(s.pcBattery, pcCharging, 16.sp, stale)
                        Text(linkWords(context, s, short = true), maxLines = 1, style = style(11.sp, if (s.connected) Ink.green else Ink.secondary, FontWeight.Medium))
                    }
                }
                // One row: ring, name and where it is; the phone's battery too when there's room.
                d.oneRow -> Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    BatteryRing(s.pcBattery, pcCharging, minOf(d.innerH, 52.dp), stale)
                    HGap(10.dp)
                    Header(s, 14.sp, GlanceModifier.defaultWeight(), short = d.narrow)
                    if (d.innerW >= 200.dp) {
                        HGap(8.dp)
                        Glyph(R.drawable.ic_w_phone, 15.dp, Ink.secondary)
                        HGap(3.dp)
                        Reading(s.phoneBattery, s.phoneCharging, 14.sp)
                    }
                }
                // Two columns: name on top, the PC's ring as the hero, the phone underneath.
                d.narrow -> Column(GlanceModifier.fillMaxSize()) {
                    Header(s, 15.sp, short = true)
                    Flex()
                    val ring = minOf(d.innerW, d.innerH - 40.dp - 24.dp).coerceIn(36.dp, 96.dp)
                    Row(GlanceModifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                        BatteryRing(s.pcBattery, pcCharging, ring, stale)
                    }
                    Flex()
                    Row(GlanceModifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally, verticalAlignment = Alignment.CenterVertically) {
                        Glyph(R.drawable.ic_w_phone, 14.dp, Ink.secondary)
                        HGap(3.dp)
                        Reading(s.phoneBattery, s.phoneCharging, 13.sp)
                    }
                }
                // Wider: name and where it is (since when), then both batteries side by side, each a
                // ring with its device inside and the number beside it.
                else -> Column(GlanceModifier.fillMaxSize()) {
                    Header(s, 17.sp)
                    Flex()
                    val ring = minOf(d.innerH - 44.dp - 8.dp, if (d.innerW >= 300.dp) 64.dp else 52.dp).coerceAtLeast(36.dp)
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Battery("PC", R.drawable.ic_w_laptop, s.pcBattery, pcCharging, ring, stale, GlanceModifier.defaultWeight())
                        HGap(12.dp)
                        Battery("Phone", R.drawable.ic_w_phone, s.phoneBattery, s.phoneCharging, ring, false, GlanceModifier.defaultWeight())
                    }
                }
            }
        }
    }

    @Composable
    private fun Battery(label: String, glyph: Int, percent: Int?, charging: Boolean, ring: Dp, stale: Boolean, modifier: GlanceModifier) {
        Row(modifier, verticalAlignment = Alignment.CenterVertically) {
            BatteryRing(percent, charging, ring, stale, glyph = glyph)
            HGap(10.dp)
            Column {
                Text(label, maxLines = 1, style = style(12.sp, Ink.secondary, FontWeight.Medium))
                Reading(percent, charging, if (ring >= 60.dp) 24.sp else 20.sp, stale)
            }
        }
    }
}

// ---- 2. shortcuts -----------------------------------------------------------------------------

class ShortcutsWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context, s: WidgetSnapshot) {
        val d = dims()
        val picked = if (LocalPreview.current != null) QuickAction.DEFAULT else {
            val glanceId = LocalGlanceId.current
            QuickActionStore.get(context, remember(glanceId) { GlanceAppWidgetManager(context).getAppWidgetId(glanceId) })
        }.ifEmpty { QuickAction.DEFAULT }
        val faded = !s.connected

        // 1x1: the first pick is the whole widget (Send files by default: the old File drop).
        if (d.oneCol) {
            val a = picked.first()
            Solid(if (faded) a.hue.faded else a.hue.fill, a.action(context)) {
                TileFace(context, a, s, minOf(d.w, d.h), labelled = false)
            }
            return
        }

        Glass(pad = false) {
            val gap = 8.dp
            val pad = if (d.h < 90.dp) 8.dp else 10.dp
            val w = d.w - pad * 2
            val h = d.h - pad * 2
            fun fit(span: Dp, min: Dp) = ((span.value + gap.value) / (min.value + gap.value)).toInt().coerceAtLeast(1)
            // Labelled tiles (Home's: glyph top left, name bottom left) where they fit without
            // costing tiles; else glyph-only tiles, which also take a one-row widget.
            val count = picked.size
            val labelCols = fit(w, 76.dp)
            val labelRows = if (h >= 60.dp) fit(h, 60.dp) else 0
            val labelled = labelRows > 0 && labelCols * labelRows >= minOf(count, 4)
            val maxCols = if (labelled) labelCols else fit(w, 44.dp)
            val maxRows = if (labelled) labelRows else fit(h, 44.dp)
            val rows = minOf(maxRows, (count + maxCols - 1) / maxCols)
            val cols = minOf(maxCols, (count + rows - 1) / rows)
            val shown = picked.take(rows * cols)
            Column(GlanceModifier.fillMaxSize().padding(pad)) {
                shown.chunked(cols).forEachIndexed { r, row ->
                    if (r > 0) VGap(gap)
                    Row(GlanceModifier.fillMaxWidth().defaultWeight()) {
                        for (i in 0 until cols) {
                            if (i > 0) HGap(gap)
                            val a = row.getOrNull(i)
                            if (a == null) Spacer(GlanceModifier.defaultWeight().fillMaxHeight())
                            else Tile(context, a, s, faded, labelled, minOf((w - gap * (cols - 1)) / cols, (h - gap * (rows - 1)) / rows))
                        }
                    }
                }
            }
        }
    }

    @Composable
    private fun androidx.glance.layout.RowScope.Tile(context: Context, a: QuickAction, s: WidgetSnapshot, faded: Boolean, labelled: Boolean, side: Dp) {
        Box(
            GlanceModifier.defaultWeight().fillMaxHeight().cornerRadius(16.dp)
                .background(if (faded) a.hue.faded else a.hue.fill).clickable(a.action(context)),
            contentAlignment = if (labelled) Alignment.TopStart else Alignment.Center,
        ) { TileFace(context, a, s, side, labelled) }
    }

    @Composable
    private fun TileFace(context: Context, a: QuickAction, s: WidgetSnapshot, side: Dp, labelled: Boolean) {
        val progress = s.transferFraction.takeIf { a == QuickAction.FILES }
        val icon = if (a == QuickAction.PLAY_PAUSE && s.playing) R.drawable.ic_pause else a.icon
        val label = when {
            progress != null -> "${(progress * 100).toInt()}%"
            a == QuickAction.PLAY_PAUSE -> if (s.playing) "Pause" else "Play"
            else -> a.label
        }
        @Composable
        fun Mark(size: Dp) {
            if (progress != null) {
                Box(GlanceModifier.size(size * 1.5f), contentAlignment = Alignment.Center) {
                    Image(ImageProvider(Rings.ring(context, size.value * 1.5f, progress, android.graphics.Color.WHITE, 0x4DFFFFFF, 3f)), null, modifier = GlanceModifier.fillMaxSize())
                    Glyph(icon, size * 0.8f, Ink.white, label)
                }
            } else Glyph(icon, size, Ink.white, label)
        }
        if (labelled) {
            Column(GlanceModifier.fillMaxSize().padding(horizontal = 10.dp, vertical = 9.dp)) {
                Mark(22.dp)
                Flex()
                Text(label, maxLines = 1, style = style(11.sp, Ink.white, FontWeight.Medium))
            }
        } else {
            Box(GlanceModifier.fillMaxSize(), contentAlignment = Alignment.Center) { Mark((side * 0.42f).coerceIn(18.dp, 30.dp)) }
        }
    }
}

// ---- 3. now playing ---------------------------------------------------------------------------

class NowPlayingWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context, s: WidgetSnapshot) {
        val d = dims()
        if (s.title == null) return Empty(context, s, d)
        val art = s.art
        // With a cover: its deep colour fills the card and the cover fades into it, white type.
        // Without: the glass, in the theme's ink.
        if (art != null) {
            val tint = Covers.tint(art)
            Solid(ColorProvider(day = Color(tint), night = Color(tint)), Widgets.openApp(context)) {
                Playing(context, s, d, art, onCover = true)
            }
        } else {
            Glass(onClick = Widgets.openApp(context), pad = false) { Playing(context, s, d, null, onCover = false) }
        }
    }

    @Composable
    private fun Playing(context: Context, s: WidgetSnapshot, d: Dims, art: Bitmap?, onCover: Boolean) {
        val ink = if (onCover) Ink.white else Ink.primary
        val soft = if (onCover) Ink.whiteSoft else Ink.secondary
        val live = s.connected
        val subtitle = when {
            !live -> linkWords(context, s, short = true).let { if (s.link == PcLink.AWAY) "PC away" else it }
            else -> s.artist?.takeIf { it.isNotBlank() } ?: s.pcName?.let { "On $it" }.orEmpty()
        }
        when {
            // 1x1: the cover, play/pause over it.
            d.oneCol -> Box(GlanceModifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                if (art != null) {
                    Image(ImageProvider(art), s.title, contentScale = ContentScale.Crop, modifier = GlanceModifier.fillMaxSize().cornerRadius(android.R.dimen.system_app_widget_background_radius))
                    Box(GlanceModifier.fillMaxSize().background(tone(0x59000000, 0x59000000))) {}
                }
                if (live) PlayButton(context, s, 40.dp, onCover)
                else Glyph(R.drawable.ic_w_music, 26.dp, ink)
            }
            // Two columns, one row: no room for words; the cover, play/pause and next.
            d.oneRow && d.w < 200.dp -> Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                val coverW = minOf(d.h * 1.2f, d.w - 60.dp)
                if (art != null) Image(ImageProvider(Covers.faded(context, art, coverW, d.h)), s.title, modifier = GlanceModifier.width(coverW).fillMaxHeight())
                else {
                    HGap(d.pad)
                    Badge(R.drawable.ic_w_music, Hue.PINK, minOf(d.h - 16.dp, 36.dp))
                }
                Row(GlanceModifier.defaultWeight(), horizontalAlignment = Alignment.CenterHorizontally, verticalAlignment = Alignment.CenterVertically) {
                    if (live) Controls(context, s, onCover, prev = false, next = d.w - coverW >= 96.dp, size = minOf(d.h - 16.dp, 40.dp))
                    else Glyph(R.drawable.ic_w_music, 22.dp, soft)
                }
            }
            // One row: the cover at the start edge to edge, title, then the controls that fit.
            d.oneRow -> Box(GlanceModifier.fillMaxSize()) {
                // Words start where the cover has all but faded (it's 16% there).
                val coverW = minOf(d.h * 1.25f, d.w * 0.4f)
                if (art != null) Image(ImageProvider(Covers.faded(context, art, coverW, d.h)), null, modifier = GlanceModifier.width(coverW).fillMaxHeight())
                Row(GlanceModifier.fillMaxSize().padding(start = if (art != null) coverW * 0.88f else d.pad, end = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(GlanceModifier.defaultWeight()) {
                        Text(s.title.orEmpty(), maxLines = 1, style = style(13.sp, ink, FontWeight.Bold))
                        if (subtitle.isNotEmpty()) Text(subtitle, maxLines = 1, style = style(11.sp, soft))
                    }
                    if (live) Controls(context, s, onCover, prev = d.w >= 300.dp, next = true, size = minOf(d.h - 14.dp, 44.dp))
                }
            }
            // Two columns: the cover, title and artist, the controls along the bottom.
            d.narrow -> Column(GlanceModifier.fillMaxSize().padding(horizontal = d.pad, vertical = d.padV)) {
                Row(GlanceModifier.fillMaxWidth()) {
                    // Cover, a line each for title and artist, controls: 44 + 34 + 46 = 124 dp before a bigger cover fits.
                    Cover(art, if (d.innerH >= 150.dp) 56.dp else 40.dp)
                }
                Flex()
                Text(s.title.orEmpty(), maxLines = if (d.innerH >= 170.dp) 2 else 1, style = style(14.sp, ink, FontWeight.Bold))
                if (subtitle.isNotEmpty()) Text(subtitle, maxLines = 1, style = style(12.sp, soft))
                if (live && d.innerH >= 120.dp) {
                    VGap(6.dp)
                    Row(GlanceModifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                        Controls(context, s, onCover, prev = d.innerW >= 120.dp, next = d.innerW >= 120.dp, size = 40.dp)
                    }
                }
            }
            // Wide: the media card. The cover fills the start edge to edge, fading into its colour.
            else -> Box(GlanceModifier.fillMaxSize()) {
                val coverW = minOf(d.h * 1.45f, d.w * 0.5f)
                if (art != null) Image(ImageProvider(Covers.faded(context, art, coverW, d.h)), null, modifier = GlanceModifier.width(coverW).fillMaxHeight())
                Row(GlanceModifier.fillMaxSize()) {
                    if (art != null) HGap(coverW * 0.86f) else HGap(d.pad)
                    Column(GlanceModifier.defaultWeight().fillMaxHeight().padding(top = d.padV, bottom = d.padV, end = d.pad)) {
                        Text(s.pcName?.let { "On $it" } ?: "PC", maxLines = 1, style = style(11.sp, soft, FontWeight.Medium))
                        Flex()
                        Text(s.title.orEmpty(), maxLines = if (d.innerH >= 140.dp) 2 else 1, style = style(if (d.tall) 20.sp else 16.sp, ink, FontWeight.Bold))
                        if (subtitle.isNotEmpty()) Text(subtitle, maxLines = 1, style = style(13.sp, soft))
                        if (live) {
                            VGap(8.dp)
                            Controls(context, s, onCover, prev = true, next = true, size = 40.dp)
                        }
                    }
                }
            }
        }
    }

    /** Nothing playing: the app's pink music badge and where music will show. */
    @Composable
    private fun Empty(context: Context, s: WidgetSnapshot, d: Dims) {
        val line = if (s.connected) s.pcName?.let { "Music on $it shows here" } ?: "Music on your PC shows here" else linkWords(context, s, short = true)
        Glass(onClick = Widgets.openApp(context)) {
            when {
                d.oneCol -> Box(GlanceModifier.fillMaxSize(), contentAlignment = Alignment.Center) { Badge(R.drawable.ic_w_music, Hue.PINK, 36.dp) }
                d.oneRow -> Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    Badge(R.drawable.ic_w_music, Hue.PINK, minOf(d.innerH, 36.dp))
                    HGap(10.dp)
                    Column(GlanceModifier.defaultWeight()) {
                        Text("Nothing playing", maxLines = if (d.narrow) 2 else 1, style = style(if (d.narrow) 12.sp else 13.sp, Ink.primary, FontWeight.Bold))
                        if (!d.narrow) Text(line, maxLines = 1, style = style(11.sp, Ink.secondary))
                    }
                }
                else -> Column(GlanceModifier.fillMaxSize()) {
                    Badge(R.drawable.ic_w_music, Hue.PINK, 36.dp)
                    Flex()
                    Text("Nothing playing", maxLines = 2, style = style(if (d.narrow) 14.sp else 15.sp, Ink.primary, FontWeight.Bold))
                    // Two columns: where, in a few words.
                    Text(if (d.narrow && s.connected) s.pcName?.let { "On $it" } ?: "On your PC" else line, maxLines = 2, style = style(12.sp, Ink.secondary))
                }
            }
        }
    }

    @Composable
    private fun Cover(art: Bitmap?, size: Dp) {
        if (art != null) Image(ImageProvider(art), null, contentScale = ContentScale.Crop, modifier = GlanceModifier.size(size).cornerRadius(size * 0.22f))
        else Badge(R.drawable.ic_w_music, Hue.PINK, size)
    }

    /** Plain glyphs either side, play/pause filled: white on a cover, the theme's ink on glass. */
    @Composable
    private fun Controls(context: Context, s: WidgetSnapshot, onCover: Boolean, prev: Boolean, next: Boolean, size: Dp) {
        val ink = if (onCover) Ink.white else Ink.primary
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (prev) {
                Box(GlanceModifier.size(size).cornerRadius(size / 2).clickable(Widgets.command(context, Commands.PREVIOUS)), contentAlignment = Alignment.Center) {
                    Glyph(R.drawable.ic_skip_previous, size * 0.6f, ink, "Previous")
                }
                HGap(4.dp)
            }
            PlayButton(context, s, size, onCover)
            if (next) {
                HGap(4.dp)
                Box(GlanceModifier.size(size).cornerRadius(size / 2).clickable(Widgets.command(context, Commands.NEXT)), contentAlignment = Alignment.Center) {
                    Glyph(R.drawable.ic_skip_next, size * 0.6f, ink, "Next")
                }
            }
        }
    }

    @Composable
    private fun PlayButton(context: Context, s: WidgetSnapshot, size: Dp, onCover: Boolean) {
        Box(
            GlanceModifier.size(size).cornerRadius(size / 2).background(if (onCover) Ink.white else Ink.primary)
                .clickable(Widgets.command(context, Commands.PLAY_PAUSE)),
            contentAlignment = Alignment.Center,
        ) {
            Glyph(
                if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, size * 0.5f,
                if (onCover) Ink.dark else tone(0xFFFFFFFF, 0xFF0B0B0F), if (s.playing) "Pause" else "Play",
            )
        }
    }
}

/** A cover's deep colour, and the cover fading into it, made once per picture and size. */
private object Covers {
    private var tintOf: Bitmap? = null
    private var tint = 0
    private val faded = object : LinkedHashMap<String, Bitmap>(4, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>) = size > 4
    }

    /** Palette's dark muted swatch, kept deep (luminance ≤ 0.06) so white type holds 4.5:1, as the app's media card. */
    @Synchronized
    fun tint(art: Bitmap): Int {
        if (art === tintOf) return tint
        val palette = Palette.from(art).maximumColorCount(16).generate()
        val c = palette.getDarkMutedColor(palette.getDarkVibrantColor(0xFF1C1C1E.toInt()))
        tint = (if (ColorUtils.calculateLuminance(c) > 0.06) {
            val hsl = FloatArray(3).also { ColorUtils.colorToHSL(c, it) }
            hsl[2] = 0.2f
            ColorUtils.HSLToColor(hsl)
        } else c) or 0xFF000000.toInt()
        tintOf = art
        return tint
    }

    /** [w] x [h]: the cover, centre-cropped to that shape, fading out from 40% across. */
    @Synchronized
    fun faded(context: Context, art: Bitmap, w: Dp, h: Dp): Bitmap {
        val density = context.resources.displayMetrics.density
        val wPx = (w.value * density).toInt().coerceAtLeast(1)
        val hPx = (h.value * density).toInt().coerceAtLeast(1)
        val key = "${System.identityHashCode(art)}:$wPx:$hPx"
        return faded.getOrPut(key) {
            createBitmap(wPx, hPx).also { out ->
                val canvas = Canvas(out)
                // Centre-crop the cover to the destination's shape.
                val scale = maxOf(wPx.toFloat() / art.width, hPx.toFloat() / art.height)
                val sw = (wPx / scale).toInt().coerceAtMost(art.width)
                val sh = (hPx / scale).toInt().coerceAtMost(art.height)
                val sx = (art.width - sw) / 2
                val sy = (art.height - sh) / 2
                canvas.drawBitmap(art, Rect(sx, sy, sx + sw, sy + sh), Rect(0, 0, wPx, hPx), Paint(Paint.FILTER_BITMAP_FLAG))
                val fade = Paint().apply {
                    shader = LinearGradient(wPx * 0.4f, 0f, wPx * 0.97f, 0f, 0xFF000000.toInt(), 0x00000000, Shader.TileMode.CLAMP)
                    xfermode = PorterDuffXfermode(PorterDuff.Mode.DST_IN)
                }
                canvas.drawRect(0f, 0f, wPx.toFloat(), hPx.toFloat(), fade)
            }
        }
    }
}

// ---- 4. clipboard -----------------------------------------------------------------------------

class ClipsWidget : VyomaWidget() {
    @Composable
    override fun Content(context: Context, s: WidgetSnapshot) {
        val d = dims()
        val send = Widgets.sendClipboard(context)
        val open = Widgets.openApp(context, MainActivity.SCREEN_CLIPBOARD)
        if (d.oneCol) {
            Solid(if (s.connected) Hue.TEAL.fill else Hue.TEAL.faded, send) { Glyph(R.drawable.ic_w_send, 26.dp, Ink.white, "Send clipboard") }
            return
        }
        Glass {
            when {
                // One row: the newest copy and Send.
                d.oneRow -> Row(GlanceModifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    Row(GlanceModifier.defaultWeight().clickable(open), verticalAlignment = Alignment.CenterVertically) {
                        if (!d.narrow) {
                            Badge(R.drawable.ic_w_clipboard, Hue.TEAL, minOf(d.innerH, 36.dp))
                            HGap(10.dp)
                        }
                        Column(GlanceModifier.defaultWeight()) {
                            Text("Clipboard", maxLines = 1, style = style(11.sp, Ink.secondary, FontWeight.Medium))
                            Text(newest(s), maxLines = 1, style = style(13.sp, Ink.primary, FontWeight.Medium))
                        }
                    }
                    HGap(8.dp)
                    SendButton(send, compact = d.narrow, faded = !s.connected)
                }
                else -> Column(GlanceModifier.fillMaxSize()) {
                    Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Row(GlanceModifier.defaultWeight().clickable(open), verticalAlignment = Alignment.CenterVertically) {
                            // Two columns: the word alone, with the round Send beside it.
                            if (!d.narrow) {
                                Badge(R.drawable.ic_w_clipboard, Hue.TEAL, 28.dp)
                                HGap(8.dp)
                            }
                            Text("Clipboard", maxLines = 1, style = style(if (d.narrow) 14.sp else 15.sp, Ink.primary, FontWeight.Bold))
                        }
                        SendButton(send, compact = d.narrow, faded = !s.connected)
                    }
                    VGap(8.dp)
                    when {
                        s.clipsHidden -> Note(open, "Locked", "Open Vyoma to see what you copied")
                        s.clips.isEmpty() -> Note(open, "Nothing copied yet", "Copies from the phone and the PC show up here")
                        else -> {
                            // A row is ~32 dp: 7 dp above and below one 13 sp line, and a hairline.
                            val rows = ((d.innerH.value - 36) / 32).toInt().coerceIn(1, 6)
                            s.clips.take(rows).forEachIndexed { i, clip ->
                                if (i > 0) Box(GlanceModifier.fillMaxWidth().height(1.dp).background(Ink.hairline)) {}
                                Text(
                                    clip, maxLines = 1,
                                    style = style(13.sp, if (i == 0) Ink.primary else Ink.secondary, if (i == 0) FontWeight.Medium else FontWeight.Normal),
                                    modifier = GlanceModifier.fillMaxWidth().padding(vertical = 7.dp).clickable(open),
                                )
                            }
                        }
                    }
                }
            }
        }
    }

    private fun newest(s: WidgetSnapshot) = when {
        s.clipsHidden -> "Locked"
        else -> s.clips.firstOrNull() ?: "Nothing copied yet"
    }

    @Composable
    private fun Note(open: Action, title: String, line: String) {
        Column(GlanceModifier.fillMaxWidth().clickable(open)) {
            Text(title, maxLines = 1, style = style(13.sp, Ink.primary, FontWeight.Medium))
            Text(line, maxLines = 3, style = style(12.sp, Ink.secondary))
        }
    }

    /** A filled teal capsule with the word (an icon alone when a row is short of room). */
    @Composable
    private fun SendButton(send: Action, compact: Boolean, faded: Boolean, size: Dp = 36.dp) {
        val fill = if (faded) Hue.TEAL.faded else Hue.TEAL.fill
        if (compact) {
            Box(GlanceModifier.size(size).cornerRadius(size / 2).background(fill).clickable(send), contentAlignment = Alignment.Center) {
                Glyph(R.drawable.ic_w_send, 18.dp, Ink.white, "Send clipboard")
            }
            return
        }
        Row(
            GlanceModifier.height(34.dp).cornerRadius(17.dp).background(fill).padding(horizontal = 14.dp).clickable(send),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Glyph(R.drawable.ic_w_send, 15.dp, Ink.white)
            HGap(6.dp)
            Text("Send", maxLines = 1, style = style(13.sp, Ink.white, FontWeight.Medium))
        }
    }
}

class PcWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = PcWidget() }
class ShortcutsWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = ShortcutsWidget()

    override fun onDeleted(context: Context, appWidgetIds: IntArray) {
        super.onDeleted(context, appWidgetIds)
        QuickActionStore.remove(context, appWidgetIds)
    }
}
class NowPlayingWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = NowPlayingWidget() }
class ClipsWidgetReceiver : GlanceAppWidgetReceiver() { override val glanceAppWidget: GlanceAppWidget = ClipsWidget() }
