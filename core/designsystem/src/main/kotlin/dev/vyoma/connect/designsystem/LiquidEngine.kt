package dev.vyoma.connect.designsystem

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
 * The liquid glass engine: everything that makes VyomaConnect's glass look like Apple's, in one place.
 *
 * Model. Every glass element is a slab whose outline is a rounded rect (pills and circles are its
 * extremes). Its surface is flat in the middle and rolls over a rim at the edge; light from the
 * screen-space direction [SpecularLight] (the phone's tilt) catches the rim where it faces the light
 * and, nearly as bright, on the opposite side.
 *
 *  - Optics ([LiquidGlass]): one AGSL pass per element over a recording of what's behind it. Chrome
 *    (bars, buttons, menus, dialogs) refracts through its rim by Snell's law over a deep slab, so
 *    the rim shows what's across the element; lifted drops ([LensStyle.Lift]) use Apple's drop
 *    optics: the body shrinks what's behind, the rim's middle magnifies what's just outside, the
 *    steepest edge flips to show what's across.
 *  - Light ([InnerGlow], [specularRim], [LensLight]): drawn at full resolution over the optics,
 *    from the same outline and light, so edges stay crisp while the optics run at reduced
 *    resolution.
 *  - Lenses ([liquidLens], [LensPainter]): a drop never re-renders glass. It sees the page behind
 *    it, the glass it lifts out of as a plain tinted shape, and the element's own content; so
 *    nothing's rim is ever drawn twice.
 *  - Shadows ([SoftShadow], [chromeShadow]): soft, outside the shape only (glass is see-through).
 */

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
 * steps are too fine to see. With a [shade] (AGSL only) the edge is also modelled as a thick drop
 * rolling over, [depth] deep: lit where it slopes towards the light, [shade] where it slopes away,
 * and a shallow counter-curve further in, so even the flat sides read as 3D (Apple's WWDC25 glass). Needs nothing behind it: works on true black and in lighter glass.
 * One per element: it keeps that element's shader.
 */
internal class InnerGlow {
    private val shader = if (liquidGlassSupported) bandShader else null
    private var path: Path? = null
    private var pathFor: Outline? = null

    fun DrawScope.draw(
        outline: Outline, color: Color, width: Float, light: Float = SpecularLight.angle, opposite: Float = 0.8f,
        shade: Color = Color.Transparent, depth: Float = width,
        /** The outline's size, when it isn't the element's (glass swollen under a finger). */
        area: Size = size,
        /** The corner radius of a path outline (continuous corners), shaded as a rounded rect. */
        corner: Float? = null,
        /** The corner's superellipse exponent: 2 for circular arcs, [SmoothCornerShape.EXPONENT] for continuous ones. */
        exponent: Float = 2f,
    ) {
        if (color.alpha <= 0f || width <= 0f || area.minDimension < 1f) return
        val radius = when (outline) {
            is Outline.Rectangle -> 0f
            is Outline.Rounded -> outline.roundRect.topLeftCornerRadius.x.takeIf { outline.roundRect.isSimple }
            else -> corner
        }
        if (shader != null && radius != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val a = Math.toRadians(light.toDouble())
            shader.setFloatUniform("size", area.width, area.height)
            shader.setFloatUniform("radius", radius.coerceAtMost(area.minDimension / 2))
            shader.setFloatUniform("exponent", exponent)
            shader.setFloatUniform("width", width)
            shader.setFloatUniform("light", cos(a).toFloat(), sin(a).toFloat())
            shader.setFloatUniform("opposite", opposite)
            shader.setFloatUniform("color", color.red, color.green, color.blue, color.alpha)
            shader.setFloatUniform("depth", depth.coerceAtLeast(1f))
            shader.setFloatUniform("shade", shade.red, shade.green, shade.blue, shade.alpha)
            drawOutline(outline, ShaderBrush(shader))
            return
        }
        val p = path.takeIf { pathFor === outline } ?: Path().apply { addOutline(outline) }.also { path = it; pathFor = outline }
        val brush = specularRim(color, Color.Transparent, Offset(area.width / 2, area.height / 2), light, opposite, area)
        clipPath(p) {
            // Narrowing strokes, each adding a little: the light piles up towards the edge.
            for (i in STEPS downTo 1) drawPath(p, brush, alpha = 1.6f / STEPS, style = Stroke(width * 2f * i / STEPS))
        }
    }

    private companion object {
        const val STEPS = 10

        /**
         * One compiled shader for every element (each card and button used to compile its own on
         * the UI thread whenever its draw cache was rebuilt). Sharing is safe: a draw takes a copy
         * of the uniforms as they are then, and all of this runs on the UI thread.
         */
        // Read only where liquidGlassSupported.
        val bandShader by lazy(LazyThreadSafetyMode.NONE) { @SuppressLint("NewApi") RuntimeShader(BAND_AGSL) }

        const val BAND_AGSL = """
uniform float2 size;
uniform float radius;
uniform float exponent;
uniform float width;
uniform float2 light;
uniform float opposite;
uniform half4 color;
uniform float depth;
uniform half4 shade;

half4 main(float2 xy) {
    float2 c = size * 0.5;
    float2 p = xy - c;
    // Signed distance to the rounded rect: negative inside.
    float2 q = abs(p) - c + radius;
    float d;
    if (q.x > 0.0 && q.y > 0.0) {
        // In a corner: distance to its superellipse (a circle when exponent is 2), divided by the
        // gradient so the band keeps one width all round a continuous corner instead of pooling.
        float2 a = q / radius;
        float2 an = pow(a, float2(exponent));
        float s = pow(an.x + an.y, 1.0 / exponent);
        float2 g = pow(a, float2(exponent - 1.0)) / pow(s, exponent - 1.0);
        d = (s - 1.0) * radius / max(length(g), 0.5);
    } else {
        d = max(q.x, q.y) - radius;
    }
    float t = clamp(1.0 + d / width, 0.0, 1.0);
    // Eases in from nothing, so the band has no inner edge to see.
    float band = t * t * t;
    // Which way this bit of edge faces, as on a squircle: lit facing the light, and the far side.
    float2 n = normalize(p / max(c, float2(1.0)) + 1e-4);
    float f = dot(n, light);
    // Spreads along the edge but gathers towards where it faces the light, so the lit corners stand out.
    float k = pow(max(f, 0.0), 1.6) + opposite * pow(max(-f, 0.0), 1.6);
    float a = float(color.a) * band * k;
    // The rolled edge: how steeply the surface falls away here, steepest at the rim and flat
    // inside, with a shallow dip the other way past the roll (the liquid S of a thick drop).
    float v = clamp(-d / depth, 0.0, 1.0);
    float u = (v - 0.62) / 0.17;
    float slope = (1.0 - v) * (1.0 - v) - 0.2 * exp(-u * u);
    float lit = slope * f;
    a = min(a + float(color.a) * 0.22 * max(lit, 0.0), 1.0);
    float s = min(float(shade.a) * max(-lit, 0.0), 1.0);
    // White over the shade, premultiplied.
    return half4(color.rgb * half(a) + shade.rgb * half(s * (1.0 - a)), half(a + s * (1.0 - a)));
}
"""
    }
}

/**
 * Apple's floating-glass shadow: soft, wide, all round with a little more underneath (iOS 26's tab bar
 * on white measures ~6% dark at its edge, gone ~12 dp out). Drawn only outside the shape, since the
 * glass is see-through and a shadow under it would grey it. A blurred round rect is analytic on the
 * GPU, so this is cheap to draw every frame. UI thread only (shared paint).
 */
internal object SoftShadow {
    private val paint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG)
    private val filters = HashMap<Int, android.graphics.BlurMaskFilter>()
    private val hole = Path()

    fun DrawScope.draw(rect: Rect, corner: Float, color: Color, blur: Float, drop: Float) {
        if (color.alpha <= 0f || blur < 1f || rect.width < 1f || rect.height < 1f) return
        val key = blur.roundToInt()
        paint.maskFilter = filters.getOrPut(key) { android.graphics.BlurMaskFilter(key.toFloat(), android.graphics.BlurMaskFilter.Blur.NORMAL) }
        paint.color = color.toArgb()
        val c = corner.coerceAtMost(rect.minDimension / 2)
        hole.reset()
        hole.addRoundRect(RoundRect(rect, CornerRadius(c)))
        clipPath(hole, ClipOp.Difference) {
            drawIntoCanvas { it.nativeCanvas.drawRoundRect(rect.left, rect.top + drop, rect.right, rect.bottom + drop, c, c, paint) }
        }
    }
}

/**
 * The shadow under floating glass, per theme and size: bigger panes (menus, dialogs) cast wider ones,
 * as Apple's do. Dark mode's is deep, for when chrome floats over lighter content (on black it's moot).
 */
internal fun DrawScope.chromeShadow(rect: Rect, corner: Float, dark: Boolean) {
    val blur = (rect.minDimension * 0.22f).coerceIn(8.dp.toPx(), 22.dp.toPx())
    // Apple's light-mode buttons (Voice Memos) cast theirs clearly downwards.
    val color = if (dark) Color.Black.copy(alpha = 0.5f) else Color(0xFF0B1220).copy(alpha = 0.16f)
    with(SoftShadow) { draw(rect, corner, color, blur, drop = blur * if (dark) 0.25f else 0.45f) }
}

/** Where [specularRim]'s sweep puts the light before turning it (0.625 of a turn from 3 o'clock). */
private const val RIM_LAYOUT_DEG = 225f

/**
 * [size] is the shape's: the sweep runs in its normalised space (as InnerGlow's facing does), so a
 * long pill catches the light along its edges the way a card does instead of only near its middle.
 */
internal fun specularRim(lit: Color, base: Color, center: Offset, light: Float = SpecularLight.angle, opposite: Float = 0.8f, size: Size? = null): Brush {
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
    val stretch = size?.takeIf { it.height > 0f }?.let { it.width / it.height } ?: 1f
    shader.setLocalMatrix(android.graphics.Matrix().apply {
        setRotate(light - RIM_LAYOUT_DEG, center.x, center.y)
        postScale(stretch, 1f, center.x, center.y)
    })
    return ShaderBrush(shader)
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
         *  segmented knobs): Apple's drop optics (the shader's `drop` branch, chosen by the negative
         *  depth): bright flipped ring, dark band of what's outside, caustic line, a body a little
         *  lighter than what it sits on (iOS Music's drop: 155 over a 120 bar). */
        val Lift = LensStyle(rim = 0.14f, depth = -1.1f, disperse = 0.015f, milk = 0.05f to 0.04f)
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
    /**
     * The glass this drop lifts out of (a tab bar drawn by an outer [glassChrome]), as its tint. The
     * drop sees it as a plain capsule of this element's bounds under the content: never the glass
     * itself, whose rim and light, shrunk inside the drop, read as a second border.
     */
    over: Color = Color.Transparent,
    /**
     * The blur of the glass this drop lifts out of, as it shows the [backdrop] (what the bar shows, in
     * px of the page). The drop sees the page through it, so it reads as the bar's material lifted, not
     * a crisp hole cut in it. The element's own drawing (the tabs) stays sharp.
     */
    blur: Dp = 0.dp,
    rect: DrawScope.() -> Rect,
): Modifier = if (!liquidGlassSupported) this else this then LiquidLensElement(backdrop, strength, zoom, underlay, style, frost, over, blur, rect)

@RequiresApi(Build.VERSION_CODES.TIRAMISU)
private data class LiquidLensElement(
    val backdrop: Backdrop?,
    val strength: () -> Float,
    val zoom: () -> Float,
    val underlay: Color,
    val style: LensStyle,
    val frost: () -> Float,
    val over: Color,
    val blur: Dp,
    val rect: DrawScope.() -> Rect,
) : ModifierNodeElement<LiquidLensNode>() {
    override fun create() = LiquidLensNode(backdrop, strength, zoom, underlay, style, frost, over, blur, rect)
    override fun update(node: LiquidLensNode) {
        node.backdrop = backdrop
        node.strength = strength
        node.zoom = zoom
        node.underlay = underlay
        node.style = style
        node.frost = frost
        node.over = over
        node.blur = blur
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
    var over: Color,
    var blur: Dp,
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
        with(p) { paint(a, r, zoom(), style, underlay, dark, backdrop, shift ?: Offset.Zero, frost().coerceIn(0f, 1f), over, blur.toPx()) }
    }

    override fun onDetach() {
        painter?.release()
        painter = null
    }
}

/**
 * A lens's light, drawn over its optics at full resolution: a crisp 1–2 px edge brightest where it
 * faces [SpecularLight] and opposite (light mode adds a dark hairline on the shaded side, as Apple's),
 * a Fresnel sheen on the steep edge, and for drop optics the caustic line where the flipped ring meets
 * the dark band (iOS Music's drop: 190 between a 25 band and a 155 body). One shared shader; a draw
 * copies its uniforms, and all of this runs on the UI thread.
 */
internal object LensLight {
    private val shader by lazy(LazyThreadSafetyMode.NONE) { @SuppressLint("NewApi") RuntimeShader(AGSL) }

    @RequiresApi(Build.VERSION_CODES.TIRAMISU)
    fun DrawScope.draw(r: Rect, rim: Float, dark: Boolean, alpha: Float, caustic: Boolean) {
        if (alpha < 0.01f || r.width < 1f) return
        val a = Math.toRadians(SpecularLight.angle.toDouble())
        shader.setFloatUniform("origin", r.left, r.top)
        shader.setFloatUniform("size", r.width, r.height)
        shader.setFloatUniform("radius", r.minDimension / 2)
        shader.setFloatUniform("rim", rim.coerceAtLeast(1f))
        shader.setFloatUniform("light", cos(a).toFloat(), sin(a).toFloat())
        shader.setFloatUniform("dark", if (dark) 1f else 0f)
        shader.setFloatUniform("alpha", alpha)
        shader.setFloatUniform("caustic", if (caustic) 1f else 0f)
        drawRect(ShaderBrush(shader), r.topLeft, r.size)
    }

    private const val AGSL = """
uniform float2 origin;
uniform float2 size;
uniform float radius;
uniform float rim;
uniform float2 light;
uniform float dark;
uniform float alpha;
uniform float caustic;

half4 main(float2 xy) {
    float2 c = size * 0.5;
    float2 p = xy - origin - c;
    float2 q = abs(p) - c + radius;
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
    float cov = clamp(0.5 - d, 0.0, 1.0);
    if (cov <= 0.0) return half4(0.0);
    float t = clamp(1.0 + d / rim, 0.0, 1.0);
    float2 nn = normalize(p / max(c, float2(1.0)) + 1e-4);
    float f = dot(nn, light);
    float face = pow(max(f, 0.0), 1.6) + 0.8 * pow(max(-f, 0.0), 1.6);
    float line = 1.0 - smoothstep(0.0, 1.6, -d);
    float nz = sqrt(max(1.0 - t * t, 0.0));
    float fres = pow(1.0 - nz, 4.0);
    float u = (t - 0.6) / 0.035;
    float cau = caustic * exp(-u * u) * step(0.001, t);
    float k = dark > 0.5 ? 0.75 : 1.0;
    float a = clamp(k * (line * (0.3 + 0.7 * face) + fres * 0.16 * (0.4 + 0.6 * face) + cau * 0.3 * (0.5 + 0.5 * face)), 0.0, 1.0);
    float b = (1.0 - dark) * line * (1.0 - face) * 0.16;
    half3 rgb = half3(a) + half3(0.10, 0.17, 0.30) * half(b * (1.0 - a));
    float o = a + b * (1.0 - a);
    return half4(rgb * half(cov * alpha), half(o * cov * alpha));
}
"""
}

/**
 * Draws content plus a refracting drop over part of it. Owns two layers from the graphics context,
 * released by [release]; one shader, rebuilt only when an input changes.
 */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
internal class LensPainter(private val graphics: GraphicsContext) {
    private val self = graphics.createGraphicsLayer()
    private val drop = graphics.createGraphicsLayer()
    private val under = graphics.createGraphicsLayer()
    private val glass = LiquidGlass()
    private var underBlur = -1f

    fun ContentDrawScope.paint(a: Float, r: Rect, zoom: Float, style: LensStyle, underlay: Color, dark: Boolean, backdrop: Backdrop?, shift: Offset, frost: Float = 0f, over: Color = Color.Transparent, blur: Float = 0f) {
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
            drop = if (style.depth < 0f) 1f else 0f,
            // Seeing the page as a bar does, it deepens colour as the bar does (see glassChrome).
            vibrance = if (backdrop != null && over.alpha > 0f) 1.7f else 1f,
        )
        drop.alpha = a
        val box = IntSize(r.width.roundToInt() + 2 * m, r.height.roundToInt() + 2 * m)
        if (backdrop != null) {
            // Only the drop's own box of the page, blurred as the bar shows it (rebuilt on change only).
            if (blur != underBlur) {
                under.renderEffect = if (blur > 0.5f) BlurEffect(blur, blur, TileMode.Clamp) else null
                underBlur = blur
            }
            under.record(box) { translate(m - r.left + shift.x, m - r.top + shift.y) { drawLayer(backdrop.layer) } }
        }
        drop.record(box) {
            if (underlay.alpha > 0f) drawRect(underlay)
            if (backdrop != null) drawLayer(under)
            translate(m - r.left, m - r.top) {
                if (over.alpha > 0f) drawRoundRect(over, cornerRadius = CornerRadius(this@paint.size.height / 2), size = this@paint.size)
                drawLayer(self)
            }
        }
        translate(r.left - m, r.top - m) { drawLayer(drop) }
        // The drop's light, at full resolution from the same outline: crisp edge, Fresnel sheen
        // on the steep edge, the caustic line inside it (drop optics only).
        with(LensLight) { draw(r, rim, dark, a, caustic = false) }
    }

    fun release() {
        graphics.releaseGraphicsLayer(self)
        graphics.releaseGraphicsLayer(drop)
        graphics.releaseGraphicsLayer(under)
    }
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
    // Shared, compiled once (see InnerGlow's): a RenderEffect copies the uniforms when it's built.
    private val shader = sharedShader
    private var last = FloatArray(20) { Float.NaN }
    private var effect: RenderEffect? = null

    internal fun effect(
        size: Size, margin: Float, radius: Float, bevel: Float, refract: Float, zoom: Float, disperse: Float, tint: Color, spec: Float, shade: Float, blur: Float,
        /** Saturation of what's seen through (1 = as is). */
        vibrance: Float = 1f,
        /** The rim's glow: how far it lifts what's under the bevel towards white. */
        gloss: Float = 0f,
        /** 1 = a lifted drop's optics (see the shader) instead of the plain Snell rim. */
        drop: Float = 0f,
    ): RenderEffect {
        // Where the light is (see SpecularLight), as a unit vector in screen axes.
        val a = Math.toRadians(SpecularLight.angle.toDouble())
        val lx = cos(a).toFloat()
        val ly = sin(a).toFloat()
        val args = floatArrayOf(size.width, size.height, margin, radius, bevel, refract, zoom, disperse, tint.red, tint.green, tint.blue, tint.alpha, spec, shade, blur, lx, ly, vibrance, gloss, drop)
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
        shader.setFloatUniform("vibrance", vibrance)
        shader.setFloatUniform("gloss", gloss)
        shader.setFloatUniform("drop", drop)
        val glass = android.graphics.RenderEffect.createRuntimeShaderEffect(shader, "content")
        val full = if (blur > 0f) {
            android.graphics.RenderEffect.createChainEffect(
                glass, android.graphics.RenderEffect.createBlurEffect(blur, blur, android.graphics.Shader.TileMode.CLAMP),
            )
        } else glass
        return full.asComposeRenderEffect().also { effect = it }
    }

    private companion object {
        val sharedShader by lazy(LazyThreadSafetyMode.NONE) { RuntimeShader(AGSL) }

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
uniform float vibrance;
uniform float gloss;
uniform float drop;

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
    } else if (drop > 0.5) {
        // A lifted drop, as iOS 26's tab drop (Apple Music): clear all through, its body minifying
        // what's behind (zoom), its rim bending further and further outwards towards the edge, so
        // content compresses into the rim and, where the drop outgrows the bar, the dark beyond the
        // bar shows as a band at the top and bottom. No flipped ring, no caustic line.
        // A round edge: flat almost to the rim, then steepening (a circle's sag), so the body stays a
        // calm, slightly smaller copy of the bar and only the last few px bend. Apple's body is also
        // brighter than the bar it lifts from (Music: 110 -> 155 over a bar, black stays ~24).
        float s = bevel * 1.5 * (1.0 - sqrt(max(1.0 - t * t, 0.0)));
        float2 shift = n * s;
        at = clamp(at + shift, lo, hi);
        col = content.eval(at);
        if (disperse > 0.0) {
            float2 k = n * (s * disperse);
            col.r = content.eval(clamp(at + k, lo, hi)).r;
            col.b = content.eval(clamp(at - k, lo, hi)).b;
        }
    } else {
        // A big pane (a dialog, a menu) has a polished edge: its slope eases in from the flat face.
        // Starting at an angle, the bent image met the straight one in a crease, a thin dark line
        // inside the rim. Bars, buttons and knobs (no wider than 3 rims) keep the plain rim exactly.
        float big = smoothstep(3.0 * bevel, 4.5 * bevel, min(c.x, c.y));
        float eased = (t < 0.35 ? t * t / 0.7 : t - 0.175) / 0.825;
        float tilt = mix(t, eased, big);
        float3 normal = normalize(float3(n * tilt, sqrt(max(1.0 - tilt * tilt, 0.0))));
        float3 ray = refract(float3(0.0, 0.0, -1.0), normal, 1.0 / 1.5);
        float h = bevel * sqrt(max(1.0 - tilt * tilt, 0.0));
        float2 shift = ray.xy * (h + bendBy) / -ray.z;
        at = clamp(at + shift, lo, hi);
        col = content.eval(at);
        if (disperse > 0.0) {
            float2 k = shift * disperse * 0.3;
            col.r = content.eval(clamp(at + k, lo, hi)).r;
            col.b = content.eval(clamp(at - k, lo, hi)).b;
        }
    }
    if (drop > 0.5 && shade <= 0.0) col.rgb = min(col.rgb * 1.2 + half3(0.03) * col.a, half3(col.a));
    // Vibrancy: Apple's glass deepens the colour of what's behind it (under macOS/iOS Control Center
    // tiles a flower's saturation goes 0.25 -> ~0.47, a blue sky 0.82 -> 0.93), never greys it.
    half lum = dot(col.rgb, half3(0.2126, 0.7152, 0.0722));
    col.rgb = clamp(mix(half3(lum), col.rgb, half(vibrance)), half3(0.0), half3(col.a));
    col.rgb = mix(col.rgb, half3(tint.rgb) * col.a, half(tint.a));
    // The gloss: what's under the bevel glows, brightest where the edge faces the light and its
    // opposite (iOS 26 tiles over blue: the rim reads as a brighter, still saturated blue).
    float f0 = dot(normalize(rel / max(c, float2(1.0)) + 1e-4), light);
    float face0 = pow(max(f0, 0.0), 1.6) + 0.8 * pow(max(-f0, 0.0), 1.6);
    float g = gloss * t * t * (0.3 + 0.7 * face0);
    col.rgb += (half3(col.a) - col.rgb) * half(g);
    // A thin specular rim, brightest where it faces the light and its opposite.
    float rim = 1.0 - smoothstep(0.0, 1.8, -d);
    // Lit by the same facing as the inner band and the rim stroke (the shape's normalised position,
    // not its true normal), so bars and cards catch the light alike.
    float f = dot(normalize(rel / max(c, float2(1.0)) + 1e-4), light);
    // As the band: full facing the light, 0.8 on the far side, gathered towards both.
    float facing = pow(max(f, 0.0), 1.6) + 0.8 * pow(max(-f, 0.0), 1.6);
    // Panes and bars glow across the bevel; a drop's light is only its thin rim (drawn by LensLight):
    // a sheen over its bevel drew a second ring inside the edge, a ball rather than a lens.
    float light = drop > 0.5 ? 0.0 : rim * (0.25 + 0.75 * facing) + t * t * t * 0.12;
    col.rgb += half3(light * spec) * col.a;
    // On light backgrounds a highlight alone vanishes: a faint shaded underside gives the edge.
    col.rgb -= half3(rim * shade * (0.4 + 0.6 * max(n.y, 0.0))) * col.a;
    return col * half(clamp(0.5 - d, 0.0, 1.0));
}
"""
    }
}
