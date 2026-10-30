package dev.vyoma.connect.designsystem

import android.graphics.RuntimeShader
import android.os.Build
import androidx.annotation.ChecksSdkIntAtLeast
import androidx.annotation.RequiresApi
import androidx.compose.foundation.border
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
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
        get() = if (isDarkUi) Color(0xFF1C1D21).copy(alpha = 0.66f) else Color.White.copy(alpha = 0.7f)

    val edge: Brush
        @Composable @ReadOnlyComposable
        get() = if (isDarkUi) {
            Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.16f), Color.White.copy(alpha = 0.03f)))
        } else {
            Brush.verticalGradient(listOf(Color.White, Color.White.copy(alpha = 0.5f)))
        }
}

/** Static glass: translucent fill plus the edge highlight, content clipped to [shape]. */
fun Modifier.glassCard(shape: Shape, fill: Color? = null): Modifier = composed {
    val color = fill ?: Glass.fill
    val edge = Glass.edge
    this
        .drawBehind { drawOutline(shape.createOutline(size, layoutDirection, this), color) }
        .border(0.75.dp, edge, shape)
        .clip(shape)
}

/**
 * A [glassCard] behind every visible item of [contentType] in a lazy list, for groups too long to
 * live in one item (a folder listing). Drawn by the list itself in the draw phase, so it's one
 * card with one highlight however many rows it holds, and it follows scrolling for free. Where
 * the group runs past the viewport its rounded end is pushed off-screen.
 */
fun Modifier.glassListGroup(state: LazyListState, contentType: Any, gutter: Dp = 16.dp, radius: Dp = 26.dp): Modifier = composed {
    val color = Glass.fill
    val edge = Glass.edge
    drawBehind {
        val visible = state.layoutInfo.visibleItemsInfo
        val first = visible.firstOrNull { it.contentType == contentType } ?: return@drawBehind
        val last = visible.last { it.contentType == contentType }
        val r = radius.toPx()
        // A neighbour of another type on screen means the group ends there; otherwise it continues off-screen.
        val endsAbove = visible.any { it.index == first.index - 1 }
        val endsBelow = visible.any { it.index == last.index + 1 } || last.index == state.layoutInfo.totalItemsCount - 1
        val top = if (endsAbove) first.offset.toFloat() else minOf(first.offset.toFloat(), -2 * r)
        val bottom = if (endsBelow) (last.offset + last.size).toFloat() else maxOf((last.offset + last.size).toFloat(), size.height + 2 * r)
        val x = gutter.toPx()
        val w = size.width - 2 * x
        val stroke = 0.75.dp.toPx()
        // Brushes are laid out from the origin, so draw at the card's top for the highlight to start there.
        translate(left = x, top = top) {
            val h = bottom - top
            drawRoundRect(color, Offset.Zero, Size(w, h), CornerRadius(r))
            drawRoundRect(edge, Offset(stroke / 2, stroke / 2), Size(w - stroke, h - stroke), CornerRadius(r - stroke / 2), style = Stroke(stroke))
        }
    }
}

/**
 * The app canvas: the theme background with two soft glows of the accent near the top. Cached per
 * size, so it costs one bitmap-free draw of three shapes. Pure-black mode keeps the glow faint.
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
        val r = size.maxDimension * 0.75f
        val left = Brush.radialGradient(
            listOf(accent.copy(alpha = strength), Color.Transparent),
            center = Offset(size.width * 0.05f, size.height * 0.02f), radius = r,
        )
        val right = Brush.radialGradient(
            listOf(accent.copy(alpha = strength * 0.6f), Color.Transparent),
            center = Offset(size.width * 1.0f, size.height * 0.28f), radius = r * 0.8f,
        )
        onDrawBehind {
            drawRect(base)
            drawRect(left)
            drawRect(right)
        }
    }
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
 * Real glass: whatever of [backdrop] sits behind this element, blurred on the GPU, then [tint]
 * and the edge highlight. The element must not be inside the backdrop's source.
 *
 * With a [bevel] (pills and circles only) and Android 13+, it's liquid glass instead: the rim bends
 * the content toward the centre like the edge of a lens, splits colour a little, and catches light
 * from the top left. One AGSL pass on the already-blurred layer, rebuilt only when the size changes.
 */
fun Modifier.glassChrome(
    backdrop: Backdrop,
    shape: Shape,
    blur: Dp = 24.dp,
    tint: Color? = null,
    edge: Boolean = true,
    bevel: Dp = 0.dp,
): Modifier = composed {
    val blurred = rememberGraphicsLayer()
    var position by remember { mutableStateOf(Offset.Zero) }
    val fill = tint ?: Glass.chrome
    val edgeBrush = Glass.edge
    val dark = isDarkUi
    val lens = remember(bevel) { if (bevel > 0.dp && liquidGlassSupported) LiquidGlass() else null }
    val blurs = remember { BlurCache() }
    this
        .onGloballyPositioned { position = it.positionInRoot() }
        .clip(shape)
        .drawBehind {
            val r = blur.toPx()
            blurred.renderEffect = lens?.effect(
                size, radius = size.minDimension / 2, bevel = bevel.toPx(), refract = bevel.toPx() * 0.55f, zoom = 1f,
                disperse = 0.15f, tint = fill.copy(alpha = fill.alpha * 0.85f), spec = if (dark) 0.42f else 0.8f, blur = r,
            ) ?: blurs[r]
            val shift = backdrop.origin - position
            blurred.record { translate(shift.x, shift.y) { drawLayer(backdrop.layer) } }
            drawLayer(blurred)
            if (lens == null) drawRect(fill)
        }
        .then(if (edge && lens == null) Modifier.border(0.75.dp, edgeBrush, shape) else Modifier)
}

/**
 * A clear liquid droplet over this element: what's under [rect] (this element's own drawing and
 * the [backdrop] around it, so it can spill past the edges) magnified by [zoom], with a refracting
 * rim. Drawn while [strength] > 0, faded by it; costs nothing otherwise. Android 13+ only
 * ([liquidGlassSupported]); callers keep their plain look below that.
 */
fun Modifier.liquidLens(backdrop: Backdrop, strength: () -> Float, zoom: () -> Float, rect: DrawScope.() -> Rect): Modifier = composed {
    if (!liquidGlassSupported) return@composed this
    val self = rememberGraphicsLayer()
    val drop = rememberGraphicsLayer()
    val lens = remember { LiquidGlass() }
    var position by remember { mutableStateOf(Offset.Zero) }
    val dark = isDarkUi
    this
        .onGloballyPositioned { position = it.positionInRoot() }
        .drawWithContent {
            val a = strength().coerceIn(0f, 1f)
            if (a < 0.01f) {
                drawContent()
                return@drawWithContent
            }
            self.record { this@drawWithContent.drawContent() }
            drawLayer(self)
            val r = rect()
            if (r.width < 1f || r.height < 1f) return@drawWithContent
            val radius = r.minDimension / 2
            drop.renderEffect = lens.effect(
                r.size, radius, bevel = radius * 0.7f, refract = radius * 0.35f, zoom = zoom(), disperse = 0.12f,
                tint = Color.White.copy(alpha = if (dark) 0.05f else 0.12f), spec = if (dark) 0.55f else 0.9f, blur = 0f,
            )
            drop.alpha = a
            val shift = backdrop.origin - position
            drop.record(IntSize(r.width.roundToInt(), r.height.roundToInt())) {
                translate(-r.left, -r.top) {
                    translate(shift.x, shift.y) { drawLayer(backdrop.layer) }
                    drawLayer(self)
                }
            }
            translate(r.left, r.top) { drawLayer(drop) }
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
 * The liquid glass pass (AGSL). The shape is a rounded rect given by its size and corner radius;
 * outside it the result is transparent, so it also clips. A RenderEffect copies its shader's
 * uniforms when built, so it's rebuilt only when an input actually changes.
 */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
internal class LiquidGlass {
    private val shader = RuntimeShader(AGSL)
    private var last = FloatArray(13) { Float.NaN }
    private var effect: RenderEffect? = null

    internal fun effect(
        size: Size, radius: Float, bevel: Float, refract: Float, zoom: Float, disperse: Float, tint: Color, spec: Float, blur: Float,
    ): RenderEffect {
        val args = floatArrayOf(size.width, size.height, radius, bevel, refract, zoom, disperse, tint.red, tint.green, tint.blue, tint.alpha, spec, blur)
        effect?.let { if (args.contentEquals(last)) return it }
        last = args
        shader.setFloatUniform("size", size.width, size.height)
        shader.setFloatUniform("radius", radius)
        shader.setFloatUniform("bevel", bevel.coerceAtLeast(1f))
        shader.setFloatUniform("bendBy", refract)
        shader.setFloatUniform("zoom", zoom)
        shader.setFloatUniform("disperse", disperse)
        shader.setFloatUniform("tint", tint.red, tint.green, tint.blue, tint.alpha)
        shader.setFloatUniform("spec", spec)
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
uniform float radius;
uniform float bevel;
uniform float bendBy;
uniform float zoom;
uniform float disperse;
uniform float4 tint;
uniform float spec;

half4 main(float2 p) {
    float2 c = size * 0.5;
    float2 rel = p - c;
    float2 q = abs(rel) - (c - radius);
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
    if (d > 1.0) return half4(0.0);
    // Outward normal of the rounded rect.
    float2 n = (q.x > 0.0 && q.y > 0.0) ? normalize(q) : (q.x > q.y ? float2(1.0, 0.0) : float2(0.0, 1.0));
    n *= sign(rel);
    // 1 at the rim, 0 past the bevel; squared so the bend gathers at the very edge like a meniscus.
    float t = clamp(1.0 + d / bevel, 0.0, 1.0);
    float bend = t * t * bendBy;
    float2 at = c + rel / zoom - n * bend;
    float2 k = n * bend * disperse * 0.3;
    half4 col = content.eval(at);
    col.r = content.eval(at + k).r;
    col.b = content.eval(at - k).b;
    col.rgb = mix(col.rgb, half3(tint.rgb) * col.a, half(tint.a));
    // A thin specular rim, brightest where it faces the light (top left) and its opposite.
    float rim = 1.0 - smoothstep(0.0, 1.8, -d);
    float facing = abs(dot(n, float2(-0.6, -0.8)));
    float light = rim * (0.25 + 0.75 * facing * facing) + t * t * t * 0.12;
    col.rgb += half3(light * spec) * col.a;
    return col * half(clamp(0.5 - d, 0.0, 1.0));
}
"""
    }
}
