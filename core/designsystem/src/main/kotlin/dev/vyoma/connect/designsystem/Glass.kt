package dev.vyoma.connect.designsystem

import androidx.compose.ui.graphics.asAndroidBitmap

import androidx.compose.ui.modifier.modifierLocalProvider

import android.annotation.SuppressLint
import android.graphics.RuntimeShader
import android.os.Build
import androidx.annotation.ChecksSdkIntAtLeast
import androidx.annotation.RequiresApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.graphics.ClipOp
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.addOutline
import androidx.compose.ui.graphics.asAndroidPath
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlurEffect
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RenderEffect
import androidx.compose.ui.graphics.asComposeRenderEffect
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.node.DrawModifierNode
import androidx.compose.ui.node.GlobalPositionAwareModifierNode
import androidx.compose.ui.node.CompositionLocalConsumerModifierNode
import androidx.compose.ui.node.currentValueOf
import androidx.compose.ui.node.requireGraphicsContext
import androidx.compose.ui.node.invalidateDraw
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.graphics.drawscope.ContentDrawScope
import androidx.compose.ui.graphics.GraphicsContext
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.geometry.isSimple
import androidx.compose.material3.LocalContentColor
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.roundToInt

/*
 * Glass, done cheaply.
 *
 * Cards are "static glass": a translucent fill and a light edge over the aurora backdrop, which is
 * drawn once per size and cached. No per-frame blur, so long lists scroll at full speed.
 *
 * Only chrome that content scrolls under (the floating tab bar, the collapsed header) gets real
 * backdrop blur: the content is recorded into one GraphicsLayer (a RenderNode display list), and
 * the bar redraws that same display list through a GPU blur. Nothing is rendered twice on the CPU.
 */

/** True when the current theme is dark; read from the canvas so it follows the app's own setting. */
val isDarkUi: Boolean
    @Composable @ReadOnlyComposable
    get() = MaterialTheme.colorScheme.background.let { 0.2126f * it.red + 0.7152f * it.green + 0.0722f * it.blue } < 0.5f

/** Height the floating tab bar takes from the bottom of the screen, so lists can scroll clear of it. */
val LocalBottomBarInset = compositionLocalOf { 0.dp }

/** iOS-style system colors for feature icons and active controls; each has a light and dark tone. */
enum class SystemColor(private val light: Long, private val dark: Long) {
    RED(0xFFFF3B30, 0xFFFF453A),
    ORANGE(0xFFFF9500, 0xFFFF9F0A),
    YELLOW(0xFFF5B800, 0xFFFFD60A),
    GREEN(0xFF34C759, 0xFF30D158),
    MINT(0xFF00C7BE, 0xFF63E6E2),
    TEAL(0xFF30B0C7, 0xFF40C8E0),
    CYAN(0xFF32ADE6, 0xFF64D2FF),
    BLUE(0xFF007AFF, 0xFF0A84FF),
    PINK(0xFFFF2D55, 0xFFFF375F),
    BROWN(0xFFA2845E, 0xFFAC8E68),
    GRAY(0xFF8E8E93, 0xFF8E8E93),
    ;

    val color: Color
        @Composable @ReadOnlyComposable
        get() = Color(if (isDarkUi) dark else light)
}

/** Translucent card fill and its edge highlight (brighter at the top, like light catching glass). */
object Glass {
    val fill: Color
        @Composable @ReadOnlyComposable
        get() = if (isDarkUi) Color.White.copy(alpha = 0.055f) else Color.White.copy(alpha = 0.32f)

    /** A step up from [fill], for buttons sitting on a glass card. */
    val raised: Color
        @Composable @ReadOnlyComposable
        get() = if (isDarkUi) Color.White.copy(alpha = 0.07f) else Color.Black.copy(alpha = 0.04f)

    /**
     * A filled accent control (prominent pill, send button, the chosen chip): a deep shade of the
     * accent under white ink in both themes. The accent itself is pastel in the dark and mid-tone in
     * the light, and dark ink on either read muddy.
     */
    val accent: Color
        @Composable @ReadOnlyComposable
        get() = MaterialTheme.colorScheme.primary.toned(1.2f, if (isDarkUi) 0.56f else 0.5f)

    /** Tint laid over real backdrop blur. Clear: Apple's glass never greys what's behind it. */
    val chrome: Color
        @Composable @ReadOnlyComposable
        get() = if (isDarkUi) Color(0xFF1C1D21).copy(alpha = 0.42f) else Color.White.copy(alpha = 0.38f)

    val edge: Brush
        @Composable @ReadOnlyComposable
        get() = if (isDarkUi) {
            Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.16f), Color.White.copy(alpha = 0.03f)))
        } else {
            Brush.verticalGradient(listOf(Color.White, Color.White.copy(alpha = 0.5f)))
        }
}

/**
 * How a card sits on the canvas, per theme: a sheet of glass, translucent so the page's accent glow
 * shows through (an opaque grey slab read as dull), even in tone, no gradient. Its edge is a faint specular
 * hairline and, in the dark, a soft glow on the lit side, both following the phone's tilt
 * ([SpecularLight]), so the card reads as a lit object rather than painted plastic (a static
 * gradient rim was rejected as cheap). Only light mode gets a shadow, faint, since there's a pale
 * canvas for it to fall on.
 */
@Immutable
class GlassDepth internal constructor(
    val fill: Color,
    /** The edge where the light hits it; transparent for none. */
    val rimLit: Color,
    /** The edge everywhere else. */
    val rimBase: Color,
    val shadow: Color,
    val shadowBlur: Float,
    val shadowDrop: Float,
    /** Light caught inside the card near its lit edge; transparent for none. */
    val glow: Color = Color.Transparent,
    /** The soft band of light just inside the edge (see [InnerGlow]); transparent for none. */
    val band: Color = Color.Transparent,
    /** The rolled edge's shaded side (see [InnerGlow]); transparent for none. */
    val shade: Color = Color.Transparent,
)

// Dark is true black (AMOLED): glass glows instead of casting shadows.
private val DarkDepth = GlassDepth(
    // Dark glass, so the lit edge has something to stand out from (lighter read as milky).
    fill = Color.White.copy(alpha = 0.045f),
    // Bright, like Apple's: on true black the lit edge is what makes it glass.
    rimLit = Color.White.copy(alpha = 0.42f), rimBase = Color.White.copy(alpha = 0.06f),
    shadow = Color.Transparent, shadowBlur = 0f, shadowDrop = 0f,
    // No glow pooled in the lit corner: Apple's cards (macOS widgets, Settings) carry their light in
    // the edge alone; a blob of it inside read as a smudge on every card.
    // Content is a thinner pane than the floating chrome: half the chrome-like band, no rolled edge,
    // so the bars read as a layer above the cards (with the same edge, on black they didn't).
    band = Color.White.copy(alpha = 0.15f),
)
private val LightDepth = GlassDepth(
    // See-through, so the canvas's tint shows in the card (0.72 read as solid white blocks).
    fill = Color.White.copy(alpha = 0.32f),
    // White on white can't show; a navy hairline gives the edge and the light lifts out of it.
    rimLit = Color.White, rimBase = Color(0xFF1A2B4C).copy(alpha = 0.06f),
    shadow = Color(0xFF1A2B4C).copy(alpha = 0.05f), shadowBlur = 14f, shadowDrop = 3f,
    band = Color.White.copy(alpha = 0.35f),
)

/** The current theme's [GlassDepth]. */
val glassDepth: GlassDepth
    @Composable @ReadOnlyComposable
    get() = when {
        isDarkUi -> DarkDepth
        else -> LightDepth
    }

/**
 * Paints one glass pane of [outline]: soft shadow outside it (never under it, so the glass stays
 * clear), an even fill, and the specular rim. Cached per size by the callers (drawWithCache).
 */
private class GlassPainter(private val d: GlassDepth, private val flat: Color?, private val lit: Boolean, density: Float, private val corner: Float? = null) {
    private val drop = d.shadowDrop * density
    private val stroke = 1f * density
    private val band = 8f * density
    private val roll = 16f * density
    private val glow = InnerGlow()
    private val shadowPaint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
        color = d.shadow.toArgb()
        if (d.shadow.alpha > 0f) maskFilter = android.graphics.BlurMaskFilter(d.shadowBlur * density, android.graphics.BlurMaskFilter.Blur.NORMAL)
    }

    /** [outline] fills, [edge] (the same shape inset by half the hairline, at [inset]) is stroked. */
    fun DrawScope.paint(outline: Outline, edge: Outline, inset: Float, height: Float, width: Float) {
        if (d.shadow.alpha > 0f) {
            val path = Path().apply { addOutline(outline) }
            // Away from the page's light (it sways with the tilt, as the edge light does).
            val fall = SpecularLight.cast(drop * 1.6f, SpecularLight.content)
            val shadow = path.asAndroidPath().let { android.graphics.Path(it).apply { offset(fall.x, fall.y) } }
            clipPath(path, ClipOp.Difference) { drawIntoCanvas { it.nativeCanvas.drawPath(shadow, shadowPaint) } }
        }
        val fill = flat ?: d.fill
        val light = SpecularLight.content
        if (lit && d.glow.alpha > 0f) {
            // The glow is part of the fill (one gradient pass, not a second layer over the card):
            // brightest at the edge point facing the light, spreading in from there.
            val a = Math.toRadians(light.toDouble())
            val at = Offset(width / 2 + cos(a).toFloat() * width / 2, height / 2 + sin(a).toFloat() * height / 2)
            // Soft, but gone before the middle: spread across the whole card it greyed it.
            drawOutline(outline, Brush.radialGradient(listOf(d.glow.compositeOver(fill), fill), at, minOf(width, height) * 1.1f))
        } else {
            drawOutline(outline, fill)
        }
        if (!lit || d.rimLit.alpha <= 0f) return
        with(glow) { draw(outline, d.band, band, light, shade = d.shade, depth = roll, corner = corner, exponent = if (corner != null) SmoothCornerShape.EXPONENT else 2f) }
        translate(inset, inset) {
            val area = Size(width - 2 * inset, height - 2 * inset)
            val r = corner ?: (edge as? Outline.Rounded)?.roundRect?.takeIf { it.isSimple }?.topLeftCornerRadius?.x ?: (edge as? Outline.Rectangle)?.let { 0f }
            with(RimLight) { stroke(edge, d.rimLit, d.rimBase, stroke, area, r, if (corner != null) SmoothCornerShape.EXPONENT else 2f, light) }
        }
    }
}

/**
 * Card: an opaque slab with a tilt-lit hairline ([lit]; off for slices of a card) and, in light
 * mode, a soft shadow, content clipped to [shape]. Static (no blur), cached per size, so long
 * lists scroll at full speed. [fill] replaces the theme's card tone (e.g. a dialog).
 */
fun Modifier.glassCard(shape: Shape, fill: Color? = null, lit: Boolean = true): Modifier = composed {
    val d = glassDepth
    this
        .drawWithCache {
            val outline = shape.createOutline(size, layoutDirection, this)
            // Inset by half the hairline so all of it shows inside the clip.
            val half = 0.5f * density
            val edge = shape.createOutline(Size(size.width - 2 * half, size.height - 2 * half), layoutDirection, this)
            val painter = GlassPainter(d, fill, lit, density, (shape as? SmoothCornerShape)?.radiusPx(size, this))
            onDrawBehind { with(painter) { paint(outline, edge, half, size.height, size.width) } }
        }
        .clip(shape)
        // What's pressed inside lights the card, not just itself (see pressScale).
        .then(InCardProvider)
}

@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
private val InCardProvider = Modifier.modifierLocalProvider(InGlassCard) { true }

/**
 * A card's inner light (band and rolled edge, as [glassCard] draws them) laid over its content
 * instead of under it: for cards whose content reaches the edge (the media card's artwork), where
 * the light drawn underneath would stop dead where the content starts.
 */
fun Modifier.glassLightOver(shape: Shape): Modifier = composed {
    val d = glassDepth
    drawWithCache {
        val outline = shape.createOutline(size, layoutDirection, this)
        val glow = InnerGlow()
        val band = 8f * density
        val roll = 16f * density
        val corner = (shape as? SmoothCornerShape)?.radiusPx(size, this)
        onDrawWithContent {
            drawContent()
            with(glow) { draw(outline, d.band, band, SpecularLight.content, shade = d.shade, depth = roll, corner = corner, exponent = if (corner != null) SmoothCornerShape.EXPONENT else 2f) }
        }
    }
}

/**
 * Raised controls sitting on a card (round buttons, chips, keys): an even fill one step up from
 * the card and a hairline specular edge that follows the phone's tilt ([SpecularLight]), so it reads
 * as lit glass rather than a painted ring. With [color] it's filled with that color instead (an
 * active toggle, an accent pill).
 */
fun Modifier.glassRaised(shape: Shape, color: Color? = null): Modifier = composed {
    val dark = isDarkUi
    val base = color
    this
        .drawWithCache {
            val outline = shape.createOutline(size, layoutDirection, this)
            val fill = base ?: if (dark) Color.White.copy(alpha = 0.07f) else Color(0xFF1A2B4C).copy(alpha = 0.045f)
            // Filled (coloured) controls get a quieter edge: a bright band on a saturated fill read as a candy bevel.
            val lit = Color.White.copy(alpha = if (base != null) 0.35f else if (dark) 0.4f else 0.95f)
            val rimBase = when {
                base != null -> Color.White.copy(alpha = 0.1f)
                dark -> Color.White.copy(alpha = 0.05f)
                else -> Color(0xFF1A2B4C).copy(alpha = 0.06f)
            }
            val stroke = Stroke(0.9.dp.toPx())
            val center = Offset(size.width / 2, size.height / 2)
            val glow = InnerGlow()
            val band = Color.White.copy(alpha = if (base != null) 0.18f else if (dark) 0.34f else 0.6f)
            val bandWidth = (size.minDimension * 0.12f).coerceAtMost((if (size.minDimension < 64.dp.toPx()) 3 else 4).dp.toPx())
            val smooth = (shape as? SmoothCornerShape)?.radiusPx(size, this)
            val radius = (outline as? Outline.Rounded)?.roundRect?.topLeftCornerRadius?.x ?: smooth ?: (size.minDimension / 2)
            onDrawBehind {
                // Light mode: a small soft shadow, as Apple's glass buttons on pale surfaces have (on a
                // dark card one would only muddy the edge, so the rim does the work there).
                if (!dark) {
                    val fall = SpecularLight.cast(2.5.dp.toPx(), SpecularLight.angle)
                    with(SoftShadow) { draw(Rect(Offset.Zero, size), radius, Color(0xFF0B1220).copy(alpha = 0.1f), 6.dp.toPx(), fall.y, fall.x) }
                }
                drawOutline(outline, fill)
                // The light's angle is read here only, so tilting redraws the edge and nothing else.
                val light = SpecularLight.angle
                // No rolled shading here: on small round controls it turned them into plastic balls.
                with(glow) { draw(outline, band, bandWidth, light, corner = smooth, exponent = if (smooth != null) SmoothCornerShape.EXPONENT else 2f) }
                with(RimLight) { stroke(outline, lit, rimBase, stroke.width, size, radius, if (smooth != null && (outline !is Outline.Rounded)) SmoothCornerShape.EXPONENT else 2f, light) }
            }
        }
        .clip(shape)
}

/**
 * A specular hairline on a solid, colourful object (icon squircles, the device glyph): it catches
 * the light where [SpecularLight] says it is, like iOS 26's icon edges, and moves as the phone tilts.
 * Drawn over the content, inside the edge.
 */
fun Modifier.specularEdge(shape: Shape, width: Dp = 1.dp, lit: Color = Color.White.copy(alpha = 0.75f), base: Color = Color.White.copy(alpha = 0.14f)): Modifier =
    drawWithCache {
        val w = width.toPx()
        // Inset by half the stroke so all of it shows when the object is clipped to the same shape.
        val edge = shape.createOutline(Size(size.width - w, size.height - w), layoutDirection, this)
        val stroke = Stroke(w)
        val center = Offset(size.width / 2, size.height / 2)
        onDrawWithContent {
            drawContent()
            translate(w / 2, w / 2) { drawOutline(edge, specularRim(lit, base, center - Offset(w / 2, w / 2), size = size), style = stroke) }
        }
    }

/**
 * One row's share of a card, for groups too long to live in one lazy item (a folder listing):
 * each row paints its own slice, rounded only where the group starts or ends. Being part of the
 * row, it moves with it: scrolling, the overscroll stretch, item animations.
 */
fun Modifier.cardSegment(first: Boolean, last: Boolean, radius: Dp = 26.dp): Modifier = composed {
    val d = glassDepth
    val shape = SmoothCornerShape(
        topStart = androidx.compose.foundation.shape.CornerSize(if (first) radius else 0.dp),
        topEnd = androidx.compose.foundation.shape.CornerSize(if (first) radius else 0.dp),
        bottomEnd = androidx.compose.foundation.shape.CornerSize(if (last) radius else 0.dp),
        bottomStart = androidx.compose.foundation.shape.CornerSize(if (last) radius else 0.dp),
    )
    // Slices can't each catch the light: their edges would rule lines between the rows.
    glassCard(shape, d.fill, lit = false)
}

/**
 * The app canvas: the theme background with the accent glowing from the top centre (a broad glow and
 * a fainter one below it, both centred: the user wanted it centred, not off in a corner). In dark mode
 * they're dimmer and the rest stays true black (AMOLED); bare black left the glass nothing to show
 * through or bend. Painted once per size and colour into a quarter-resolution bitmap and drawn stretched:
 * soft gradients survive the upscale untouched, and one textured quad is far cheaper for the GPU
 * than three full-screen gradient fills every frame (again for every glass element replaying it).
 */
fun Modifier.aurora(): Modifier = composed {
    val base = MaterialTheme.colorScheme.background
    val accent = MaterialTheme.colorScheme.primary
    if (!AppCanvas.glow) return@composed background(base)
    val strength = if (isDarkUi) 0.16f else 0.20f
    val wallpaper = AppCanvas.wallpaper.takeIf { AppCanvas.useWallpaper }
    val dark = isDarkUi
    drawWithCache {
        val w = (size.width / AURORA_SCALE).roundToInt().coerceAtLeast(1)
        val h = (size.height / AURORA_SCALE).roundToInt().coerceAtLeast(1)
        val bitmap = if (wallpaper != null) AuroraCache.wallpaper(w, h, base, wallpaper, dark) else AuroraCache.get(w, h, base, accent, strength)
        val dst = IntSize(size.width.roundToInt(), size.height.roundToInt())
        onDrawBehind { drawImage(bitmap, dstSize = dst, filterQuality = FilterQuality.Low) }
    }
}

private const val AURORA_SCALE = 4f

/**
 * Canvas bitmaps by size and colours, shared by every page: they were painted on the CPU (UI
 * thread) each time a page opened, a few ms of the first frame of every push. A handful of sizes
 * (full screen, with and without the keyboard) covers the app.
 */
private object AuroraCache {
    private class Key(val w: Int, val h: Int, val base: Color, val accent: Color, val strength: Float) {
        override fun equals(other: Any?) = other is Key && other.w == w && other.h == h && other.base == base && other.accent == accent && other.strength == strength
        override fun hashCode() = ((w * 31 + h) * 31 + base.hashCode()) * 31 + accent.hashCode() + strength.hashCode()
    }
    private val entries = LinkedHashMap<Key, ImageBitmap>(8, 0.75f, true)
    private val walls = LinkedHashMap<List<Any>, ImageBitmap>(4, 0.75f, true)

    /**
     * The PC's wallpaper as the canvas: its colour grid stretched smoothly over the page (bilinear,
     * so it's the wallpaper's light, heavily blurred), toned down so text stays legible (dark: a deep
     * shade, richer colour; light: a pale wash over the canvas grey) and fading into the plain canvas
     * towards the bottom, where the controls sit, as the accent glow does.
     */
    fun wallpaper(w: Int, h: Int, base: Color, wall: CanvasWallpaper, dark: Boolean): ImageBitmap {
        val key = listOf(w, h, base, wall, dark)
        walls[key]?.let { return it }
        val grid = ImageBitmap(wall.cols, wall.rows)
        val toned = IntArray(wall.colors.size) { i ->
            val c = Color(0xFF000000.toInt() or wall.colors[i])
            val rich = c.toned(saturation = 1.25f)
            (if (dark) lerp(base, rich, 0.42f) else lerp(base, rich, 0.32f)).toArgb()
        }
        grid.asAndroidBitmap().setPixels(toned, 0, wall.cols, 0, 0, wall.cols, wall.rows)
        val small = Size(w.toFloat(), h.toFloat())
        val bitmap = ImageBitmap(w, h)
        CanvasDrawScope().draw(Density(1f), LayoutDirection.Ltr, Canvas(bitmap), small) {
            drawRect(base)
            drawImage(grid, srcSize = IntSize(wall.cols, wall.rows), dstSize = IntSize(w, h), filterQuality = FilterQuality.Low)
            drawRect(Brush.verticalGradient(0.35f to Color.Transparent, 1f to base.copy(alpha = if (dark) 0.85f else 0.7f)))
        }
        walls[key] = bitmap
        if (walls.size > 4) walls.remove(walls.keys.first())
        return bitmap
    }

    fun get(w: Int, h: Int, base: Color, accent: Color, strength: Float): ImageBitmap {
        val key = Key(w, h, base, accent, strength)
        entries[key]?.let { return it }
        val small = Size(w.toFloat(), h.toFloat())
        val r = small.maxDimension * 0.75f
        val left = Brush.radialGradient(
            listOf(accent.copy(alpha = strength), accent.copy(alpha = strength * 0.35f), Color.Transparent),
            center = Offset(small.width * 0.5f, 0f), radius = r,
        )
        val right = Brush.radialGradient(
            listOf(accent.copy(alpha = strength * 0.4f), Color.Transparent),
            center = Offset(small.width * 0.5f, small.height * 0.3f), radius = r * 0.7f,
        )
        val bitmap = ImageBitmap(w, h)
        CanvasDrawScope().draw(Density(1f), LayoutDirection.Ltr, Canvas(bitmap), small) {
            drawRect(base)
            drawRect(left)
            drawRect(right)
        }
        entries[key] = bitmap
        if (entries.size > 4) entries.remove(entries.keys.first())
        return bitmap
    }
}

/** The canvas's accent glows ([aurora]); the user can turn them off. Changes apply on recomposition. */
object AppCanvas {
    var glow by mutableStateOf(true)
    /** The PC's wallpaper colours (from its companion), the last ones seen; null before any. */
    var wallpaper by mutableStateOf<CanvasWallpaper?>(null)
    /** The user's choice: the glow takes [wallpaper]'s colours instead of the accent's. */
    var useWallpaper by mutableStateOf(true)
}

/** A desktop wallpaper as a [cols] x [rows] grid of 0xRRGGBB colours, row-major from the top left. */
class CanvasWallpaper(val cols: Int, val rows: Int, val colors: IntArray) {
    override fun equals(other: Any?) = other is CanvasWallpaper && other.cols == cols && other.rows == rows && other.colors.contentEquals(colors)
    override fun hashCode() = (cols * 31 + rows) * 31 + colors.contentHashCode()
}

/** What chrome blurs: one recorded display list of the content underneath. */
@Stable
class Backdrop internal constructor(internal val layer: GraphicsLayer) {
    internal var origin by mutableStateOf(Offset.Zero)
}

@Composable
fun rememberBackdrop(): Backdrop {
    val layer = rememberGraphicsLayer()
    return remember(layer) { Backdrop(layer) }
}

/** Records this content into [backdrop] while drawing it as usual. */
fun Modifier.backdropSource(backdrop: Backdrop): Modifier = this
    .onGloballyPositioned { backdrop.origin = it.positionInRoot() }
    .drawWithContent {
        backdrop.layer.record { this@drawWithContent.drawContent() }
        drawLayer(backdrop.layer)
    }

/**
 * Chrome's blur and refraction run at this share of full resolution (see [glassChrome]). Tried and
 * measured: one shared full-page blur was slower (it replays the whole page; each element's own
 * copy only replays what's behind it), and full resolution cost ~3 ms of GPU per frame scrolling.
 */
// Half resolution: at a quarter, content under bars could only be shown as mush (Apple's bars keep
// it legible, lightly blurred).
private const val CHROME_RES = 0.5f

/**
 * Real glass: whatever of [backdrop] sits behind this element, blurred on the GPU, then [tint]
 * and the edge highlight. The element must not be inside the backdrop's source.
 *
 * With a [bevel] (pills and circles only) and Android 13+, it's liquid glass instead: clearer, and
 * the rim bends in what lies just outside the element like the edge of a lens, splits colour a
 * little, and catches light from above, a little left. One AGSL pass over a lightly blurred layer that
 * covers a margin around the element, rebuilt only when the size changes.
 */
fun Modifier.glassChrome(
    backdrop: Backdrop,
    shape: Shape,
    blur: Dp = 24.dp,
    tint: Color? = null,
    edge: Boolean = true,
    bevel: Dp = 0.dp,
    /**
     * A button made of this glass: under a finger it swells like a bubble and wobbles back on
     * release, glyph and all held still inside it (a glyph squishing the other way, or the glass
     * magnifying, read as glitches). The glass never scales as a layer (that slid the blurred
     * backdrop around inside it).
     */
    interactive: Boolean = false,
): Modifier = composed {
    val blurred = rememberGraphicsLayer()
    val press = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    var position by remember { mutableStateOf(Offset.Zero) }
    val fill = tint ?: Glass.chrome
    // Lighter glass: opaque enough that the missing blur isn't missed (content passing under stays a shadow).
    // (Dark's default tint is a grey that would turn light if made opaque: there it's the grey it shows over black.)
    val liteFill = fill.copy(alpha = maxOf(fill.alpha, if (tint == null) 0.92f else 0.6f))
    // The rim's inner band of light (see InnerGlow).
    val glow = remember { InnerGlow() }
    val band = Color.White.copy(alpha = if (isDarkUi) 0.52f else 0.6f)
    val shade = if (isDarkUi) Color.Black.copy(alpha = 0.28f) else Color(0xFF1A2B4C).copy(alpha = 0.07f)
    val edgeBrush = Glass.edge
    val dark = isDarkUi
    val lens = remember(bevel) { if (bevel > 0.dp && liquidGlassSupported) LiquidGlass() else null }
    val blurs = remember { BlurCache() }
    this
        .onGloballyPositioned { position = it.positionInRoot() }
        .then(
            if (!interactive || lens == null) Modifier else Modifier.pointerInput(Unit) {
                // Watches the finger without taking it: the button inside still gets the click.
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    scope.launch { press.animateTo(1f, VyomaMotion.spatial()) }
                    do {
                        val e = awaitPointerEvent()
                    } while (e.changes.any { it.pressed })
                    // Thrown back: overshoots smaller and wobbles in, like a bubble settling.
                    scope.launch { press.animateTo(0f, VyomaMotion.jelly(), initialVelocity = -4f) }
                }
            },
        )
        // Its shadow, ahead of any clip; it swells with the pressed glass.
        .drawBehind {
            if (size.width < 1f || size.height < 1f) return@drawBehind
            val grow = if (lens == null) 0f else (size.minDimension * 0.08f * press.value).coerceAtLeast(-size.minDimension * 0.04f)
            val corner = ((shape.createOutline(size, layoutDirection, this) as? Outline.Rounded)?.roundRect?.topLeftCornerRadius?.x
                ?: size.minDimension / 2).coerceAtMost(size.minDimension / 2) + grow
            chromeShadow(Rect(-grow, -grow, size.width + grow, size.height + grow), corner, dark)
        }
        // The shader clips to the shape itself, and its rim needs to reach past the edge.
        .then(if (lens == null) Modifier.clip(shape) else Modifier)
        .drawBehind {
            // An empty slot (an actions row with nothing in it) draws nothing.
            if (size.width < 1f || size.height < 1f) return@drawBehind
            // Read while drawing, so switching it redraws every element in place.
            val bandWidth = (size.minDimension * 0.1f).coerceAtMost(4.dp.toPx())
            val roll = (size.minDimension * 0.18f).coerceAtMost(10.dp.toPx())
            if (GlassQuality.lite) {
                val outline = shape.createOutline(size, layoutDirection, this)
                drawOutline(outline, liteFill)
                with(glow) { draw(outline, band, bandWidth, shade = shade, depth = roll) }
                // Without the lens the border modifier below draws the edge already.
                if (edge && lens != null) drawOutline(outline, edgeBrush, style = Stroke(0.75.dp.toPx()))
                return@drawBehind
            }
            val r = blur.toPx()
            val shift = backdrop.origin - position
            if (lens == null || !liquidGlassSupported) {
                blurred.renderEffect = blurs[r]
                blurred.record { translate(shift.x, shift.y) { drawLayer(backdrop.layer) } }
                drawLayer(blurred)
                drawRect(fill)
                with(glow) { draw(shape.createOutline(size, layoutDirection, this@drawBehind), band, bandWidth, shade = shade, depth = roll) }
                return@drawBehind
            }
            // The rim stays a fifth of the element at most, so small buttons are still mostly clear.
            val rim = bevel.toPx().coerceAtMost(size.minDimension * LensStyle.Chrome.rim)
            // A deep slab throws the edge's view across the element; the margin is what lies
            // beyond its far side.
            val depth = rim * LensStyle.Chrome.depth
            val p = press.value
            // Pressed, the glass grows past its bounds (drawn bigger, sampled bigger: nothing slides);
            // settling, it may dip a little under its size.
            val grow = (size.minDimension * 0.08f * p).coerceAtLeast(-size.minDimension * 0.04f)
            val g = Size(size.width + 2 * grow, size.height + 2 * grow)
            // The far side plus what lies beyond it, and room for the inflated shape and the blur.
            val m = (rim * 2f).roundToInt() + 12
            // Blurred and refracted in one pass at half resolution, for a quarter of the GPU fill. What
            // would show the lower resolution is the outline and the highlight, so the shader's
            // shape is inflated a little past the real one and the visible edge comes from an
            // anti-aliased full-resolution clip, with the specular rim stroked at full resolution.
            val q = CHROME_RES
            val inflate = 2f / q
            val gs = Size(g.width + 2 * inflate, g.height + 2 * inflate)
            // The shape's own corner (a menu is a rounded rect); pills and circles come out as half the height.
            val corner = ((shape.createOutline(size, layoutDirection, this) as? Outline.Rounded)?.roundRect?.topLeftCornerRadius?.x
                ?: size.minDimension / 2).coerceAtMost(size.minDimension / 2) + grow
            blurred.renderEffect = lens.effect(
                gs * q, (m - inflate) * q, radius = (corner + inflate) * q, bevel = rim * q, refract = depth * q, zoom = 1f,
                // No tint (user, 2026-11-05: "dont want tint at all"; Apple's tinted glass is an
                // option, not the default): what's behind keeps its own colour. A caller's tint
                // (the PC screen's frost over video) still applies.
                disperse = LensStyle.Chrome.disperse, tint = tint?.let { it.copy(alpha = it.alpha * if (dark) 0.5f else 0.92f) } ?: Color.Transparent,
                spec = 0.15f + 0.2f * p.coerceIn(0f, 1f), shade = if (dark) 0f else 0.1f, blur = r * q,
                // No colour boost either: it turned the pale canvas under the title's bubble blue.
                vibrance = 1f, gloss = if (dark) 0.22f else 0.3f,
            )
            val o = m + grow
            blurred.record(IntSize(((g.width + 2 * m) * q).roundToInt(), ((g.height + 2 * m) * q).roundToInt())) {
                scale(q, q, pivot = Offset.Zero) {
                    translate(shift.x + o, shift.y + o) { drawLayer(backdrop.layer) }
                }
            }
            val edgePath = Path().apply { addRoundRect(RoundRect(-grow, -grow, size.width + grow, size.height + grow, CornerRadius(corner))) }
            clipPath(edgePath) {
                translate(-o, -o) { scale(1f / q, 1f / q, pivot = Offset.Zero) { drawLayer(blurred) } }
            }
            // The band and its shading swell with the glass: left at rest size they read as a
            // second, smaller pane on top of the pressed one.
            translate(-grow, -grow) {
                with(glow) { draw(shape.createOutline(g, layoutDirection, this@drawBehind), band, bandWidth, shade = shade, depth = roll, area = g) }
            }
            // The highlight: brightest where it faces the light and its opposite; it follows the tilt.
            val pr = p.coerceIn(0f, 1f)
            drawPath(
                edgePath,
                // On a light page a white highlight alone vanishes: the rest of the edge is a soft dark hairline.
                specularRim(
                    Color.White.copy(alpha = (if (dark) 0.55f else 1f) * (0.75f + 0.25f * pr) + 0.25f * pr),
                    if (dark) Color.White.copy(alpha = 0.08f) else Color(0xFF1A2B4C).copy(alpha = 0.12f),
                    center,
                    size = g,
                ),
                style = Stroke(1.dp.toPx()),
            )
        }
        .then(if (edge && lens == null) Modifier.border(0.75.dp, edgeBrush, shape) else Modifier)
}

/**
 * Lighter glass for phones that can't redraw the full kind in time (the app turns it on when frames
 * keep missing their deadline, see FrameBudget). Every live glass element is its own offscreen pass
 * re-rendered whenever anything under it changes; lite chrome is a frosted fill with the same edge,
 * drawn straight onto the page. Changes apply on the next draw.
 */
object GlassQuality {
    var lite by mutableStateOf(false)
}

/** Liquid glass needs AGSL (Android 13); below that chrome stays frosted and lenses aren't drawn. */
@ChecksSdkIntAtLeast(api = Build.VERSION_CODES.TIRAMISU)
val liquidGlassSupported = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU

/** One blur per radius instead of one per frame. */
/**
 * iOS 26's soft scroll edge under the status bar: what passes beneath is blurred and fades out
 * into the canvas, strongest at the top and clear by the bottom of this strip, so the clock and
 * icons never sit on raw content. Stack two (a heavy one over a short band, a light one reaching
 * further) for a progressive blur that thickens towards the top. Not a shade: the canvas colour only thinly, never a dark band
 * (the user rejected the header shade). Only while [active] (content is actually under it).
 */
fun Modifier.scrollEdge(backdrop: Backdrop, blur: Dp = 16.dp, tint: Float = 0.55f, active: () -> Boolean): Modifier = composed {
    val blurred = rememberGraphicsLayer()
    val blurs = remember { BlurCache() }
    var position by remember { mutableStateOf(Offset.Zero) }
    val canvas = MaterialTheme.colorScheme.background
    this
        .onGloballyPositioned { position = it.positionInRoot() }
        // Its own small buffer, so the fade can mask the blur.
        .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
        .drawBehind {
            if (!active() || size.height < 1f) return@drawBehind
            val fade = Brush.verticalGradient(0f to Color.Black, 0.45f to Color.Black.copy(alpha = 0.85f), 1f to Color.Transparent)
            if (!GlassQuality.lite) {
                val shift = backdrop.origin - position
                blurred.renderEffect = blurs[blur.toPx()]
                blurred.record { translate(shift.x, shift.y) { drawLayer(backdrop.layer) } }
                drawLayer(blurred)
            }
            drawRect(canvas.copy(alpha = if (GlassQuality.lite) 0.9f else tint))
            drawRect(fade, blendMode = BlendMode.DstIn)
        }
}

private class BlurCache {
    private var radius = -1f
    private var effect: RenderEffect? = null
    operator fun get(r: Float): RenderEffect =
        effect.takeIf { r == radius } ?: BlurEffect(r, r, TileMode.Clamp).also { radius = r; effect = it }
}
