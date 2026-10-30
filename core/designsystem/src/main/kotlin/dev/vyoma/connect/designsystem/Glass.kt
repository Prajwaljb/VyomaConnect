package dev.vyoma.connect.designsystem

import android.graphics.RuntimeShader
import android.os.Build
import androidx.annotation.ChecksSdkIntAtLeast
import androidx.annotation.RequiresApi
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
import androidx.compose.material3.LocalContentColor
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import kotlinx.coroutines.launch
import kotlin.math.abs
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

/** Pure-black (AMOLED) mode: the canvas is true black, so glass glows instead of casting shadows. */
val isAmoledUi: Boolean
    @Composable @ReadOnlyComposable
    get() = MaterialTheme.colorScheme.background == Color.Black

/**
 * How a card sits on the canvas, per theme. iOS-style grouped surfaces: one opaque, even tone that
 * stands clearly off the canvas (high contrast on true black), no gradient and no rim, which read
 * as cheap plastic. Only light mode gets a shadow, faint, since there's a pale canvas for it to
 * fall on. A rim is for glass that floats (chrome, raised controls), not for the page's paper.
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
)

private val DarkDepth = GlassDepth(
    fill = Color(0xFF1C1C1E),
    rimLit = Color.Transparent, rimBase = Color.Transparent,
    shadow = Color.Transparent, shadowBlur = 0f, shadowDrop = 0f,
)
private val AmoledDepth = DarkDepth
private val LightDepth = GlassDepth(
    fill = Color.White,
    rimLit = Color.Transparent, rimBase = Color.Transparent,
    shadow = Color(0xFF1A2B4C).copy(alpha = 0.05f), shadowBlur = 14f, shadowDrop = 3f,
)

/** The current theme's [GlassDepth]. */
val glassDepth: GlassDepth
    @Composable @ReadOnlyComposable
    get() = when {
        isAmoledUi -> AmoledDepth
        isDarkUi -> DarkDepth
        else -> LightDepth
    }

/**
 * The specular edge: a sweep around the shape's centre, brightest at the top-left (where the
 * light is), a softer catch at the bottom-right, the base tone between. Angles run clockwise from
 * 3 o'clock.
 */
internal fun specularRim(lit: Color, base: Color, center: Offset): Brush {
    val back = lerp(base, lit, 0.45f)
    return Brush.sweepGradient(
        0f to base,
        0.125f to back, // bottom-right
        0.3f to base,
        0.5f to base,
        0.625f to lit, // top-left
        0.8f to base,
        1f to base,
        center = center,
    )
}

/**
 * Paints one glass pane of [outline]: soft shadow outside it (never under it, so the glass stays
 * clear), an even fill, and the specular rim. Cached per size by the callers (drawWithCache).
 */
private class GlassPainter(private val d: GlassDepth, private val flat: Color?, density: Float) {
    private val drop = d.shadowDrop * density
    private val stroke = 1f * density
    private val shadowPaint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
        color = d.shadow.toArgb()
        if (d.shadow.alpha > 0f) maskFilter = android.graphics.BlurMaskFilter(d.shadowBlur * density, android.graphics.BlurMaskFilter.Blur.NORMAL)
    }

    fun DrawScope.paint(path: Path, height: Float, width: Float) {
        if (d.shadow.alpha > 0f) {
            val shadow = path.asAndroidPath().let { android.graphics.Path(it).apply { offset(0f, drop) } }
            clipPath(path, ClipOp.Difference) { drawIntoCanvas { it.nativeCanvas.drawPath(shadow, shadowPaint) } }
        }
        drawPath(path, flat ?: d.fill)
        if (d.rimLit.alpha > 0f) drawPath(path, specularRim(d.rimLit, d.rimBase, Offset(width / 2, height / 2)), style = Stroke(stroke))
    }
}

/**
 * Glass card: a lit, rimmed slab with a soft shadow, content clipped to [shape]. Static (no
 * blur), cached per size, so long lists scroll at full speed. [fill] replaces the gradient with
 * a solid color (e.g. a dialog over no backdrop).
 */
fun Modifier.glassCard(shape: Shape, fill: Color? = null): Modifier = composed {
    val d = glassDepth
    this
        .drawWithCache {
            val path = Path().apply { addOutline(shape.createOutline(size, layoutDirection, this@drawWithCache)) }
            val painter = GlassPainter(d, fill, density)
            onDrawBehind { with(painter) { paint(path, size.height, size.width) } }
        }
        .clip(shape)
}

/**
 * Raised glass for controls sitting on a card (round buttons, chips): a smaller slab with a
 * glossy top edge. With [color] it's that color as lit glass instead (an active toggle).
 */
fun Modifier.glassRaised(shape: Shape, color: Color? = null): Modifier = composed {
    val dark = isDarkUi
    val base = color
    this
        .drawWithCache {
            val outline = shape.createOutline(size, layoutDirection, this)
            val fill = base ?: if (dark) Color.White.copy(alpha = 0.12f) else Color(0xFF1A2B4C).copy(alpha = 0.06f)
            val rim = specularRim(
                lit = Color.White.copy(alpha = if (dark || base != null) 0.45f else 0.95f),
                base = when {
                    base != null -> Color.White.copy(alpha = 0.12f)
                    dark -> Color.White.copy(alpha = 0.06f)
                    else -> Color(0xFF1A2B4C).copy(alpha = 0.06f)
                },
                center = Offset(size.width / 2, size.height / 2),
            )
            val stroke = Stroke(0.9.dp.toPx())
            onDrawBehind {
                drawOutline(outline, fill)
                drawOutline(outline, rim, style = stroke)
            }
        }
        .clip(shape)
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
    glassCard(shape, d.fill)
}

/**
 * The app canvas: the theme background with two soft glows of the accent near the top. Pure black
 * keeps true black below the fold; its glow is a little cooler and fades out sooner so the bottom
 * stays ink. Painted once per size and colour into a quarter-resolution bitmap and drawn stretched:
 * soft gradients survive the upscale untouched, and one textured quad is far cheaper for the GPU
 * than three full-screen gradient fills every frame (again for every glass element replaying it).
 */
fun Modifier.aurora(): Modifier = composed {
    val base = MaterialTheme.colorScheme.background
    val accent = MaterialTheme.colorScheme.primary
    val dark = isDarkUi
    val amoled = base == Color.Black
    val strength = when {
        amoled -> 0.16f
        dark -> 0.24f
        else -> 0.20f
    }
    drawWithCache {
        val w = (size.width / AURORA_SCALE).roundToInt().coerceAtLeast(1)
        val h = (size.height / AURORA_SCALE).roundToInt().coerceAtLeast(1)
        val small = Size(w.toFloat(), h.toFloat())
        val r = small.maxDimension * if (amoled) 0.6f else 0.75f
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
 * little, and catches light from the top left. One AGSL pass over a lightly blurred layer that
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
            val r = blur.toPx()
            val shift = backdrop.origin - position
            if (lens == null) {
                blurred.renderEffect = blurs[r]
                blurred.record { translate(shift.x, shift.y) { drawLayer(backdrop.layer) } }
                drawLayer(blurred)
                drawRect(fill)
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
            // The highlight: brightest where it faces the light (top left) and its opposite.
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

/** How a lens bends: the rim as a share of the lens's short side, and the slab's depth in rims. */
@Immutable
class LensStyle(val rim: Float, val depth: Float, val disperse: Float) {
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
         *  inward, the rim replayed the track or bar as mirrored stripes. */
        val Lift = LensStyle(rim = 0.22f, depth = -1.1f, disperse = 0.03f)
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
            tint = Color.White.copy(alpha = androidx.compose.ui.util.lerp(if (dark) 0.05f else 0.12f, 0.97f, (frost * 32).roundToInt() / 32f)),
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
    }

    fun release() {
        graphics.releaseGraphicsLayer(self)
        graphics.releaseGraphicsLayer(drop)
    }
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
    private var last = FloatArray(15) { Float.NaN }
    private var effect: RenderEffect? = null

    internal fun effect(
        size: Size, margin: Float, radius: Float, bevel: Float, refract: Float, zoom: Float, disperse: Float, tint: Color, spec: Float, shade: Float, blur: Float,
    ): RenderEffect {
        val args = floatArrayOf(size.width, size.height, margin, radius, bevel, refract, zoom, disperse, tint.red, tint.green, tint.blue, tint.alpha, spec, shade, blur)
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
    // A thin specular rim, brightest where it faces the light (top left) and its opposite.
    float rim = 1.0 - smoothstep(0.0, 1.8, -d);
    float facing = abs(dot(n, float2(-0.6, -0.8)));
    float light = rim * (0.25 + 0.75 * facing * facing) + t * t * t * 0.12;
    col.rgb += half3(light * spec) * col.a;
    // On light backgrounds a highlight alone vanishes: a faint shaded underside gives the edge.
    col.rgb -= half3(rim * shade * (0.4 + 0.6 * max(n.y, 0.0))) * col.a;
    return col * half(clamp(0.5 - d, 0.0, 1.0));
}
"""
    }
}
