package dev.vyoma.connect.designsystem

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
        get() = if (isDarkUi) Color.White.copy(alpha = 0.075f) else Color.White.copy(alpha = 0.72f)

    /** A step up from [fill], for buttons sitting on a glass card. */
    val raised: Color
        @Composable @ReadOnlyComposable
        get() = if (isDarkUi) Color.White.copy(alpha = 0.11f) else Color.Black.copy(alpha = 0.05f)

    /** Tint laid over real backdrop blur. */
    val chrome: Color
        @Composable @ReadOnlyComposable
        get() = if (isDarkUi) Color(0xFF1C1D21).copy(alpha = 0.56f) else Color.White.copy(alpha = 0.72f)

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
)

// Dark is true black (AMOLED): glass glows instead of casting shadows.
private val DarkDepth = GlassDepth(
    fill = Color.White.copy(alpha = 0.095f),
    // Bright, like Apple's: on true black the lit edge is what makes it glass.
    rimLit = Color.White.copy(alpha = 0.42f), rimBase = Color.White.copy(alpha = 0.06f),
    shadow = Color.Transparent, shadowBlur = 0f, shadowDrop = 0f,
    glow = Color.White.copy(alpha = 0.05f),
    band = Color.White.copy(alpha = 0.2f),
)
private val LightDepth = GlassDepth(
    fill = Color.White.copy(alpha = 0.72f),
    // White on white can't show; a navy hairline gives the edge and the light lifts out of it.
    rimLit = Color.White, rimBase = Color(0xFF1A2B4C).copy(alpha = 0.06f),
    shadow = Color(0xFF1A2B4C).copy(alpha = 0.05f), shadowBlur = 14f, shadowDrop = 3f,
    band = Color.White.copy(alpha = 0.7f),
)

/** The current theme's [GlassDepth]. */
val glassDepth: GlassDepth
    @Composable @ReadOnlyComposable
    get() = when {
        isDarkUi -> DarkDepth
        else -> LightDepth
    }

/**
 * The specular edge: a sweep around the shape's centre, brightest facing the light (up and a little left at
 * rest, following [SpecularLight] as the phone tilts), a softer catch opposite, the base tone
 * between. Angles run clockwise from 3 o'clock.
 */
/**
 * Apple's rim: a soft band of light just inside an outline's edge, [width] deep, brightest where
 * the edge faces the light and nearly as bright opposite (as [specularRim]), fading smoothly inward
 * with no steps. One small AGSL pass per element on Android 13+ (distance to a rounded rect, so
 * cards, pills and circles); before that, or for an odd shape, many faint nested strokes whose
 * steps are too fine to see. Needs nothing behind it: works on true black and in lighter glass.
 * One per element: it keeps that element's shader.
 */
internal class InnerGlow {
    private val shader = if (liquidGlassSupported) RuntimeShader(BAND_AGSL) else null
    private var path: Path? = null
    private var pathFor: Outline? = null

    fun DrawScope.draw(outline: Outline, color: Color, width: Float, light: Float = SpecularLight.angle, opposite: Float = 0.8f) {
        if (color.alpha <= 0f || width <= 0f || size.minDimension < 1f) return
        val radius = when (outline) {
            is Outline.Rectangle -> 0f
            is Outline.Rounded -> outline.roundRect.topLeftCornerRadius.x.takeIf { outline.roundRect.isSimple }
            else -> null
        }
        if (shader != null && radius != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val a = Math.toRadians(light.toDouble())
            shader.setFloatUniform("size", size.width, size.height)
            shader.setFloatUniform("radius", radius.coerceAtMost(size.minDimension / 2))
            shader.setFloatUniform("width", width)
            shader.setFloatUniform("light", cos(a).toFloat(), sin(a).toFloat())
            shader.setFloatUniform("opposite", opposite)
            shader.setFloatUniform("color", color.red, color.green, color.blue, color.alpha)
            drawOutline(outline, ShaderBrush(shader))
            return
        }
        val p = path.takeIf { pathFor === outline } ?: Path().apply { addOutline(outline) }.also { path = it; pathFor = outline }
        val brush = specularRim(color, Color.Transparent, center, light, opposite)
        clipPath(p) {
            // Narrowing strokes, each adding a little: the light piles up towards the edge.
            for (i in STEPS downTo 1) drawPath(p, brush, alpha = 1.6f / STEPS, style = Stroke(width * 2f * i / STEPS))
        }
    }

    private companion object {
        const val STEPS = 10
        const val BAND_AGSL = """
uniform float2 size;
uniform float radius;
uniform float width;
uniform float2 light;
uniform float opposite;
uniform half4 color;

half4 main(float2 xy) {
    float2 c = size * 0.5;
    float2 p = xy - c;
    // Signed distance to the rounded rect: negative inside.
    float2 q = abs(p) - c + radius;
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
    float t = clamp(1.0 + d / width, 0.0, 1.0);
    // Eases in from nothing, so the band has no inner edge to see.
    float band = t * t * t;
    // Which way this bit of edge faces, as on a squircle: lit facing the light, and the far side.
    float2 n = normalize(p / max(c, float2(1.0)) + 1e-4);
    float f = dot(n, light);
    // Broad, unlike the rim line: Apple's inner light spreads well along the edge.
    float k = max(f, 0.0) + opposite * max(-f, 0.0);
    float a = float(color.a) * band * k;
    return half4(color.rgb * half(a), half(a));
}
"""
    }
}

/** Where [specularRim]'s sweep puts the light before turning it (0.625 of a turn from 3 o'clock). */
private const val RIM_LAYOUT_DEG = 225f

internal fun specularRim(lit: Color, base: Color, center: Offset, light: Float = SpecularLight.angle, opposite: Float = 0.8f): Brush {
    val b = base.toArgb()
    fun at(k: Float) = lerp(base, lit, k).toArgb()
    val o = opposite
    val shader = android.graphics.SweepGradient(
        center.x, center.y,
        // Like Apple's glass: a broad, soft highlight facing the light and a nearly as bright one
        // opposite (the light passing through and catching the far edge), each fading over about
        // a little under a quarter of the way round, dark between.
        intArrayOf(at(0.4f * o), at(o), at(0.4f * o), b, b, at(0.4f), lit.toArgb(), at(0.4f), b, b, at(0.4f * o)),
        // Laid out with the light at 0.625 of the sweep (225°), then turned to where it is now.
        floatArrayOf(0f, 0.125f, 0.236f, 0.347f, 0.403f, 0.514f, 0.625f, 0.736f, 0.847f, 0.903f, 1f),
    )
    shader.setLocalMatrix(android.graphics.Matrix().apply { setRotate(light - RIM_LAYOUT_DEG, center.x, center.y) })
    return ShaderBrush(shader)
}

/**
 * Paints one glass pane of [outline]: soft shadow outside it (never under it, so the glass stays
 * clear), an even fill, and the specular rim. Cached per size by the callers (drawWithCache).
 */
private class GlassPainter(private val d: GlassDepth, private val flat: Color?, private val lit: Boolean, density: Float) {
    private val drop = d.shadowDrop * density
    private val stroke = 1f * density
    private val band = 7f * density
    private val glow = InnerGlow()
    private val shadowPaint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
        color = d.shadow.toArgb()
        if (d.shadow.alpha > 0f) maskFilter = android.graphics.BlurMaskFilter(d.shadowBlur * density, android.graphics.BlurMaskFilter.Blur.NORMAL)
    }

    /** [outline] fills, [edge] (the same shape inset by half the hairline, at [inset]) is stroked. */
    fun DrawScope.paint(outline: Outline, edge: Outline, inset: Float, height: Float, width: Float) {
        if (d.shadow.alpha > 0f) {
            val path = Path().apply { addOutline(outline) }
            val shadow = path.asAndroidPath().let { android.graphics.Path(it).apply { offset(0f, drop) } }
            clipPath(path, ClipOp.Difference) { drawIntoCanvas { it.nativeCanvas.drawPath(shadow, shadowPaint) } }
        }
        val fill = flat ?: d.fill
        val light = SpecularLight.angle
        if (lit && d.glow.alpha > 0f) {
            // The glow is part of the fill (one gradient pass, not a second layer over the card):
            // brightest at the edge point facing the light, spreading in from there.
            val a = Math.toRadians(light.toDouble())
            val at = Offset(width / 2 + cos(a).toFloat() * width / 2, height / 2 + sin(a).toFloat() * height / 2)
            // Spread wide, so it washes softly into the card rather than sitting as a spot.
            drawOutline(outline, Brush.radialGradient(listOf(d.glow.compositeOver(fill), fill), at, minOf(width, height) * 1.6f))
        } else {
            drawOutline(outline, fill)
        }
        if (!lit || d.rimLit.alpha <= 0f) return
        with(glow) { draw(outline, d.band, band, light) }
        translate(inset, inset) {
            val c = Offset(width / 2 - inset, height / 2 - inset)
            drawOutline(edge, specularRim(d.rimLit, d.rimBase, c, light), style = Stroke(stroke))
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
            val painter = GlassPainter(d, fill, lit, density)
            onDrawBehind { with(painter) { paint(outline, edge, half, size.height, size.width) } }
        }
        .clip(shape)
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
            val fill = base ?: if (dark) Color.White.copy(alpha = 0.12f) else Color(0xFF1A2B4C).copy(alpha = 0.06f)
            val lit = Color.White.copy(alpha = if (base != null) 0.5f else if (dark) 0.4f else 0.95f)
            val rimBase = when {
                base != null -> Color.White.copy(alpha = 0.1f)
                dark -> Color.White.copy(alpha = 0.05f)
                else -> Color(0xFF1A2B4C).copy(alpha = 0.06f)
            }
            val stroke = Stroke(0.9.dp.toPx())
            val center = Offset(size.width / 2, size.height / 2)
            val glow = InnerGlow()
            val band = Color.White.copy(alpha = if (base != null) 0.26f else if (dark) 0.18f else 0.6f)
            val bandWidth = (size.minDimension * 0.12f).coerceAtMost(4.dp.toPx())
            onDrawBehind {
                drawOutline(outline, fill)
                // The light's angle is read here only, so tilting redraws the edge and nothing else.
                val light = SpecularLight.angle
                with(glow) { draw(outline, band, bandWidth, light) }
                drawOutline(outline, specularRim(lit, rimBase, center, light), style = stroke)
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
            translate(w / 2, w / 2) { drawOutline(edge, specularRim(lit, base, center - Offset(w / 2, w / 2)), style = stroke) }
        }
    }

/**
 * One row's share of a card, for groups too long to live in one lazy item (a folder listing):
 * each row paints its own slice, rounded only where the group starts or ends. Being part of the
 * row, it moves with it: scrolling, the overscroll stretch, item animations.
 */
fun Modifier.cardSegment(first: Boolean, last: Boolean, radius: Dp = 26.dp): Modifier = composed {
    val d = glassDepth
    val shape = androidx.compose.foundation.shape.RoundedCornerShape(
        topStart = if (first) radius else 0.dp, topEnd = if (first) radius else 0.dp,
        bottomStart = if (last) radius else 0.dp, bottomEnd = if (last) radius else 0.dp,
    )
    // Slices can't each catch the light: their edges would rule lines between the rows.
    glassCard(shape, d.fill, lit = false)
}

/**
 * The app canvas: the theme background with, in light mode, two soft glows of the accent near the
 * top. Dark mode is true black throughout, glow and all, for AMOLED screens. Painted once per size and colour into a quarter-resolution bitmap and drawn stretched:
 * soft gradients survive the upscale untouched, and one textured quad is far cheaper for the GPU
 * than three full-screen gradient fills every frame (again for every glass element replaying it).
 */
fun Modifier.aurora(): Modifier = composed {
    val base = MaterialTheme.colorScheme.background
    val accent = MaterialTheme.colorScheme.primary
    if (isDarkUi) return@composed background(base)
    val strength = 0.20f
    drawWithCache {
        val w = (size.width / AURORA_SCALE).roundToInt().coerceAtLeast(1)
        val h = (size.height / AURORA_SCALE).roundToInt().coerceAtLeast(1)
        val small = Size(w.toFloat(), h.toFloat())
        val r = small.maxDimension * 0.75f
        val left = Brush.radialGradient(
            listOf(accent.copy(alpha = strength), accent.copy(alpha = strength * 0.35f), Color.Transparent),
            center = Offset(small.width * 0.05f, small.height * 0.02f), radius = r,
        )
        val right = Brush.radialGradient(
            listOf(accent.copy(alpha = strength * 0.6f), Color.Transparent),
            center = Offset(small.width * 1.0f, small.height * 0.28f), radius = r * 0.8f,
        )
        val bitmap = ImageBitmap(w, h)
        CanvasDrawScope().draw(this, layoutDirection, Canvas(bitmap), small) {
            drawRect(base)
            drawRect(left)
            drawRect(right)
        }
        val dst = IntSize(size.width.roundToInt(), size.height.roundToInt())
        onDrawBehind { drawImage(bitmap, dstSize = dst, filterQuality = FilterQuality.Low) }
    }
}

private const val AURORA_SCALE = 4f

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
private const val CHROME_RES = 0.25f

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
     * A button made of this glass: under a finger it swells a little and magnifies what's behind
     * it, in place. The glass never scales as a layer (that slid the blurred backdrop around
     * inside it); the glyph inside squishes on its own.
     */
    interactive: Boolean = false,
): Modifier = composed {
    val blurred = rememberGraphicsLayer()
    val press = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    var position by remember { mutableStateOf(Offset.Zero) }
    val fill = tint ?: Glass.chrome
    // Lighter glass: opaque enough that the missing blur isn't missed (content passing under stays a shadow).
    val liteFill = fill.copy(alpha = maxOf(fill.alpha, if (tint == null) 0.92f else 0.6f))
    // The rim's inner band of light (see InnerGlow).
    val glow = remember { InnerGlow() }
    val band = Color.White.copy(alpha = if (isDarkUi) 0.3f else 0.6f)
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
                    scope.launch { press.animateTo(0f, VyomaMotion.release()) }
                }
            },
        )
        // The shader clips to the shape itself, and its rim needs to reach past the edge.
        .then(if (lens == null) Modifier.clip(shape) else Modifier)
        .drawBehind {
            // An empty slot (an actions row with nothing in it) draws nothing.
            if (size.width < 1f || size.height < 1f) return@drawBehind
            // Read while drawing, so switching it redraws every element in place.
            val bandWidth = (size.minDimension * 0.1f).coerceAtMost(4.dp.toPx())
            if (GlassQuality.lite) {
                val outline = shape.createOutline(size, layoutDirection, this)
                drawOutline(outline, liteFill)
                with(glow) { draw(outline, band, bandWidth) }
                // Without the lens the border modifier below draws the edge already.
                if (edge && lens != null) drawOutline(outline, edgeBrush, style = Stroke(0.75.dp.toPx()))
                return@drawBehind
            }
            val r = blur.toPx()
            val shift = backdrop.origin - position
            if (lens == null) {
                blurred.renderEffect = blurs[r]
                blurred.record { translate(shift.x, shift.y) { drawLayer(backdrop.layer) } }
                drawLayer(blurred)
                drawRect(fill)
                with(glow) { draw(shape.createOutline(size, layoutDirection, this@drawBehind), band, bandWidth) }
                return@drawBehind
            }
            // The rim stays a fifth of the element at most, so small buttons are still mostly clear.
            val rim = bevel.toPx().coerceAtMost(size.minDimension * LensStyle.Chrome.rim)
            // A deep slab throws the edge's view across the element; the margin is what lies
            // beyond its far side.
            val depth = rim * LensStyle.Chrome.depth
            val p = press.value
            // Pressed, the glass grows past its bounds (drawn bigger, sampled bigger: nothing slides).
            val grow = (size.minDimension * 0.08f * p).coerceAtLeast(0f)
            val g = Size(size.width + 2 * grow, size.height + 2 * grow)
            // The far side plus what lies beyond it, and room for the inflated shape and the blur.
            val m = (rim * 2f).roundToInt() + 12
            // Blurred and refracted in one pass at quarter resolution: inside, it only bends blurred
            // content, which looks the same at that size, for a sixteenth of the GPU fill. What
            // would show the lower resolution is the outline and the highlight, so the shader's
            // shape is inflated a little past the real one and the visible edge comes from an
            // anti-aliased full-resolution clip, with the specular rim stroked at full resolution.
            val q = CHROME_RES
            val inflate = 2f / q
            val gs = Size(g.width + 2 * inflate, g.height + 2 * inflate)
            blurred.renderEffect = lens.effect(
                gs * q, (m - inflate) * q, radius = gs.minDimension / 2 * q, bevel = rim * q, refract = depth * q, zoom = 1f + 0.14f * p,
                // Clear in dark mode; light mode keeps more milk so dark labels stay legible over photos.
                disperse = LensStyle.Chrome.disperse, tint = fill.copy(alpha = fill.alpha * if (dark) 0.5f else 0.92f),
                spec = 0.15f + 0.2f * p.coerceIn(0f, 1f), shade = if (dark) 0f else 0.1f, blur = r * q,
            )
            val o = m + grow
            blurred.record(IntSize(((g.width + 2 * m) * q).roundToInt(), ((g.height + 2 * m) * q).roundToInt())) {
                scale(q, q, pivot = Offset.Zero) {
                    translate(shift.x + o, shift.y + o) { drawLayer(backdrop.layer) }
                }
            }
            val edgePath = Path().apply { addRoundRect(RoundRect(-grow, -grow, size.width + grow, size.height + grow, CornerRadius(g.minDimension / 2))) }
            clipPath(edgePath) {
                translate(-o, -o) { scale(1f / q, 1f / q, pivot = Offset.Zero) { drawLayer(blurred) } }
            }
            with(glow) { draw(shape.createOutline(size, layoutDirection, this@drawBehind), band, bandWidth) }
            // The highlight: brightest where it faces the light and its opposite; it follows the tilt.
            val pr = p.coerceIn(0f, 1f)
            drawPath(
                edgePath,
                // On a light page a white highlight alone vanishes: the rest of the edge is a soft dark hairline.
                specularRim(
                    Color.White.copy(alpha = (if (dark) 0.55f else 1f) * (0.75f + 0.25f * pr) + 0.25f * pr),
                    if (dark) Color.White.copy(alpha = 0.08f) else Color(0xFF1A2B4C).copy(alpha = 0.12f),
                    center,
                ),
                style = Stroke(1.dp.toPx()),
            )
        }
        .then(if (edge && lens == null) Modifier.border(0.75.dp, edgeBrush, shape) else Modifier)
}

/**
 * How a lens bends: the rim as a share of the lens's short side, and the slab's depth in rims.
 * [milk] is the white mixed into the glass in dark and light themes.
 */
@Immutable
class LensStyle(val rim: Float, val depth: Float, val disperse: Float, val milk: Pair<Float, Float> = 0.05f to 0.12f) {
    companion object {
        /** Bars and buttons (chrome): water-like, the rim shows what's across the element. */
        val Chrome = LensStyle(rim = 0.2f, depth = 6f, disperse = 0.04f)
        /** A tube holding liquid: a wide rim so the surface visibly climbs the walls. */
        val Tube = LensStyle(rim = 0.3f, depth = 3f, disperse = 0f)
        /** Drops that move or grow over content: a thinner, calmer rim so what's under the middle
         *  stays readable. */
        val Drop = LensStyle(rim = 0.18f, depth = 2.6f, disperse = 0.03f)
        /** Lifted drops that outgrow what they sit on (the tab bar's drop, slider, switch and
         *  segmented knobs). The rim looks outward (negative depth): the edge gathers in what's
         *  around, the way a drop of water on glass rims itself with its surroundings. Looking
         *  inward, the rim replayed the track or bar as mirrored stripes. Crystal clear: any milk
         *  read as a dirty white haze as it grew. */
        val Lift = LensStyle(rim = 0.22f, depth = -1.1f, disperse = 0.03f, milk = 0f to 0.02f)
    }
}

/**
 * A clear liquid drop over this element: what's under [rect] (this element's own drawing, over
 * [underlay], plus the [backdrop] around it if given, so it can spill past the edges) magnified by
 * [zoom], refracted by its rim. Drawn while [strength] > 0, faded by it. A modifier node that
 * allocates nothing until the first frame it's visible and draws plain content otherwise, so it's
 * fine on every control. Android 13+ only ([liquidGlassSupported]); a no-op below.
 */
fun Modifier.liquidLens(
    backdrop: Backdrop?,
    strength: () -> Float,
    zoom: () -> Float = { 1f },
    underlay: Color = Color.Transparent,
    style: LensStyle = LensStyle.Drop,
    /** 0 = clear water, 1 = frosted white (a slider knob at rest that clears as it's grabbed). */
    frost: () -> Float = { 0f },
    rect: DrawScope.() -> Rect,
): Modifier = if (!liquidGlassSupported) this else this then LiquidLensElement(backdrop, strength, zoom, underlay, style, frost, rect)

@RequiresApi(Build.VERSION_CODES.TIRAMISU)
private data class LiquidLensElement(
    val backdrop: Backdrop?,
    val strength: () -> Float,
    val zoom: () -> Float,
    val underlay: Color,
    val style: LensStyle,
    val frost: () -> Float,
    val rect: DrawScope.() -> Rect,
) : ModifierNodeElement<LiquidLensNode>() {
    override fun create() = LiquidLensNode(backdrop, strength, zoom, underlay, style, frost, rect)
    override fun update(node: LiquidLensNode) {
        node.backdrop = backdrop
        node.strength = strength
        node.zoom = zoom
        node.underlay = underlay
        node.style = style
        node.frost = frost
        node.rect = rect
        node.invalidateDraw()
    }
}

@RequiresApi(Build.VERSION_CODES.TIRAMISU)
private class LiquidLensNode(
    var backdrop: Backdrop?,
    var strength: () -> Float,
    var zoom: () -> Float,
    var underlay: Color,
    var style: LensStyle,
    var frost: () -> Float,
    var rect: DrawScope.() -> Rect,
) : Modifier.Node(), DrawModifierNode, GlobalPositionAwareModifierNode, CompositionLocalConsumerModifierNode {
    private var position = Offset.Zero
    private var painter: LensPainter? = null

    override fun onGloballyPositioned(coordinates: LayoutCoordinates) {
        position = coordinates.positionInRoot()
    }

    override fun ContentDrawScope.draw() {
        val a = strength().coerceIn(0f, 1f)
        val r = if (a >= 0.01f) rect() else Rect.Zero
        if (a < 0.01f || r.width < 1f || r.height < 1f) {
            drawContent()
            return
        }
        // Light glyphs on it means a dark surface under it.
        val dark = currentValueOf(LocalContentColor).luminance() > 0.5f
        val p = painter ?: LensPainter(requireGraphicsContext()).also { painter = it }
        val shift = backdrop?.let { it.origin - position }
        with(p) { paint(a, r, zoom(), style, underlay, dark, backdrop, shift ?: Offset.Zero, frost().coerceIn(0f, 1f)) }
    }

    override fun onDetach() {
        painter?.release()
        painter = null
    }
}

/**
 * Draws content plus a refracting drop over part of it. Owns two layers from the graphics context,
 * released by [release]; one shader, rebuilt only when an input changes.
 */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
internal class LensPainter(private val graphics: GraphicsContext) {
    private val self = graphics.createGraphicsLayer()
    private val drop = graphics.createGraphicsLayer()
    private val glass = LiquidGlass()

    fun ContentDrawScope.paint(a: Float, r: Rect, zoom: Float, style: LensStyle, underlay: Color, dark: Boolean, backdrop: Backdrop?, shift: Offset, frost: Float = 0f) {
        self.record { this@paint.drawContent() }
        // Under the drop only the drop shows: drawing the original there too doubled anything
        // seen through a translucent surface (a pill's label showed twice, magnified and not).
        // While the drop forms or fades, the original crossfades with it under there (a small
        // offscreen for those few frames only), so it never punches a hole or flashes.
        val hole = Path().apply { addRoundRect(RoundRect(r.deflate(0.5f), CornerRadius(r.minDimension / 2))) }
        clipPath(hole, ClipOp.Difference) { drawLayer(self) }
        if (a < 0.99f) clipPath(hole) {
            drawContext.canvas.saveLayer(r, Paint().apply { alpha = 1f - a })
            drawLayer(self)
            drawContext.canvas.restore()
        }
        val rim = r.minDimension * style.rim
        // Room for the rim to look past the far edge.
        val m = (r.minDimension * style.rim * (2f + abs(style.depth).coerceAtMost(4f) * 0.5f)).roundToInt() + 1
        drop.renderEffect = glass.effect(
            r.size, m.toFloat(), r.minDimension / 2, bevel = rim, refract = rim * style.depth, zoom = zoom, disperse = style.disperse,
            // Quantized so a frost animation rebuilds the shader a few dozen times, not every frame.
            tint = Color.White.copy(alpha = androidx.compose.ui.util.lerp(if (dark) style.milk.first else style.milk.second, 0.97f, (frost * 32).roundToInt() / 32f)),
            spec = if (dark) 0.55f else 0.9f,
            shade = if (dark) 0f else 0.12f, blur = 0f,
        )
        drop.alpha = a
        drop.record(IntSize(r.width.roundToInt() + 2 * m, r.height.roundToInt() + 2 * m)) {
            if (underlay.alpha > 0f) drawRect(underlay)
            translate(m - r.left, m - r.top) {
                if (backdrop != null) translate(shift.x, shift.y) { drawLayer(backdrop.layer) }
                drawLayer(self)
            }
        }
        translate(r.left - m, r.top - m) { drawLayer(drop) }
        // The edge catches the light like the bars' glass does: a drop over a plain track has
        // nothing around it to refract, and without this its rim vanished.
        val half = 0.5.dp.toPx()
        val edge = r.deflate(half)
        drawRoundRect(
            specularRim(
                Color.White.copy(alpha = if (dark) 0.6f else 1f),
                if (dark) Color.White.copy(alpha = 0.1f) else Color(0xFF1A2B4C).copy(alpha = 0.14f),
                edge.center,
            ),
            edge.topLeft, edge.size, CornerRadius(edge.height / 2), alpha = a, style = Stroke(1.dp.toPx()),
        )
    }

    fun release() {
        graphics.releaseGraphicsLayer(self)
        graphics.releaseGraphicsLayer(drop)
    }
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
private class BlurCache {
    private var radius = -1f
    private var effect: RenderEffect? = null
    operator fun get(r: Float): RenderEffect =
        effect.takeIf { r == radius } ?: BlurEffect(r, r, TileMode.Clamp).also { radius = r; effect = it }
}

/**
 * The liquid glass pass (AGSL). The shape is a rounded rect given by its size and corner radius,
 * inset by `margin` in the layer it runs on; outside it the result is transparent, so it also clips.
 * Physically based: Snell refraction through a rounded rim over a slab `bendBy` deep, so the rim shows what's
 * across the element (and, past its far edge, the margin's surroundings). A RenderEffect copies its shader's
 * uniforms when built, so it's rebuilt only when an input actually changes.
 */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
internal class LiquidGlass {
    private val shader = RuntimeShader(AGSL)
    private var last = FloatArray(17) { Float.NaN }
    private var effect: RenderEffect? = null

    internal fun effect(
        size: Size, margin: Float, radius: Float, bevel: Float, refract: Float, zoom: Float, disperse: Float, tint: Color, spec: Float, shade: Float, blur: Float,
    ): RenderEffect {
        // Where the light is (see SpecularLight), as a unit vector in screen axes.
        val a = Math.toRadians(SpecularLight.angle.toDouble())
        val lx = cos(a).toFloat()
        val ly = sin(a).toFloat()
        val args = floatArrayOf(size.width, size.height, margin, radius, bevel, refract, zoom, disperse, tint.red, tint.green, tint.blue, tint.alpha, spec, shade, blur, lx, ly)
        effect?.let { if (args.contentEquals(last)) return it }
        last = args
        shader.setFloatUniform("size", size.width, size.height)
        shader.setFloatUniform("margin", margin)
        shader.setFloatUniform("radius", radius)
        shader.setFloatUniform("bevel", bevel.coerceAtLeast(1f))
        shader.setFloatUniform("bendBy", refract)
        shader.setFloatUniform("zoom", zoom)
        shader.setFloatUniform("disperse", disperse)
        shader.setFloatUniform("tint", tint.red, tint.green, tint.blue, tint.alpha)
        shader.setFloatUniform("spec", spec)
        shader.setFloatUniform("shade", shade)
        shader.setFloatUniform("light", lx, ly)
        val glass = android.graphics.RenderEffect.createRuntimeShaderEffect(shader, "content")
        val full = if (blur > 0f) {
            android.graphics.RenderEffect.createChainEffect(
                glass, android.graphics.RenderEffect.createBlurEffect(blur, blur, android.graphics.Shader.TileMode.CLAMP),
            )
        } else glass
        return full.asComposeRenderEffect().also { effect = it }
    }

    private companion object {
        const val AGSL = """
uniform shader content;
uniform float2 size;
uniform float margin;
uniform float radius;
uniform float bevel;
uniform float bendBy;
uniform float zoom;
uniform float disperse;
uniform float4 tint;
uniform float spec;
uniform float shade;
uniform float2 light;

half4 main(float2 p) {
    float2 c = size * 0.5;
    float2 rel = p - margin - c;
    float2 q = abs(rel) - (c - radius);
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
    if (d > 1.0) return half4(0.0);
    // Outward normal of the rounded rect.
    float2 n = (q.x > 0.0 && q.y > 0.0) ? normalize(q) : (q.x > q.y ? float2(1.0, 0.0) : float2(0.0, 1.0));
    n *= sign(rel);
    // The rim is a rounded slope (quarter circle, `bevel` wide) on a flat slab. Its surface normal
    // tilts from straight up (t = 0) to sideways at the edge (t = 1); a ray looking straight down
    // refracts through it (Snell, glass ≈ 1.5) and travels on through the slab's depth, so near the
    // edge it lands far inside: the rim shows what's across the element, bent round like water.
    float t = clamp(1.0 + d / bevel, 0.0, 1.0);
    float2 lo = float2(0.5);
    float2 hi = size + 2.0 * margin - 0.5;
    float2 at = margin + c + rel / zoom;
    half4 col;
    if (t <= 0.0) {
        // The flat middle (most pixels): straight through, one sample.
        col = content.eval(at);
    } else {
        float3 normal = normalize(float3(n * t, sqrt(max(1.0 - t * t, 0.0))));
        float3 ray = refract(float3(0.0, 0.0, -1.0), normal, 1.0 / 1.5);
        float h = bevel * sqrt(max(1.0 - t * t, 0.0));
        float2 shift = ray.xy * (h + bendBy) / -ray.z;
        at = clamp(at + shift, lo, hi);
        col = content.eval(at);
        if (disperse > 0.0) {
            float2 k = shift * disperse * 0.3;
            col.r = content.eval(clamp(at + k, lo, hi)).r;
            col.b = content.eval(clamp(at - k, lo, hi)).b;
        }
    }
    col.rgb = mix(col.rgb, half3(tint.rgb) * col.a, half(tint.a));
    // A thin specular rim, brightest where it faces the light and its opposite.
    float rim = 1.0 - smoothstep(0.0, 1.8, -d);
    float facing = abs(dot(n, light));
    // Linear in how squarely the edge faces it: a broad highlight, not a glint.
    float light = rim * (0.25 + 0.75 * pow(facing, 1.3)) + t * t * t * 0.12;
    col.rgb += half3(light * spec) * col.a;
    // On light backgrounds a highlight alone vanishes: a faint shaded underside gives the edge.
    col.rgb -= half3(rim * shade * (0.4 + 0.6 * max(n.y, 0.0))) * col.a;
    return col * half(clamp(0.5 - d, 0.0, 1.0));
}
"""
    }
}
