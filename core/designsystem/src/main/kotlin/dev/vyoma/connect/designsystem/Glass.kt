package dev.vyoma.connect.designsystem

import androidx.compose.foundation.border
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
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlurEffect
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

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
 * One row of a [glassCard] that a lazy list lays out row by row (a long folder listing): each
 * row draws the whole card's rounded outline, stretched past its own edges where it has
 * neighbours, and clipped to its own slice, so the rows join into one seamless card.
 * [listSpacing] is the list's gap between items: rows above the last report that much less
 * height, so the next row starts right below instead of after a gap.
 */
fun Modifier.glassCardSegment(first: Boolean, last: Boolean, radius: Dp = 26.dp, listSpacing: Dp = 12.dp): Modifier = composed {
    val color = Glass.fill
    val edge = Glass.edge
    this.layout { measurable, constraints ->
        val p = measurable.measure(constraints)
        val gap = if (last) 0 else listSpacing.roundToPx()
        layout(p.width, (p.height - gap).coerceAtLeast(0)) { p.place(0, 0) }
    }.drawBehind {
        val r = radius.toPx()
        val top = if (first) 0f else -2 * r
        val bottom = if (last) size.height else size.height + 2 * r
        val stroke = 0.75.dp.toPx()
        clipRect {
            drawRoundRect(color, Offset(0f, top), Size(size.width, bottom - top), CornerRadius(r))
            val inset = stroke / 2
            drawRoundRect(
                edge, Offset(inset, top + inset), Size(size.width - stroke, bottom - top - stroke), CornerRadius(r - inset),
                style = Stroke(stroke),
            )
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
 */
fun Modifier.glassChrome(backdrop: Backdrop, shape: Shape, blur: Dp = 24.dp, tint: Color? = null, edge: Boolean = true): Modifier = composed {
    val blurred = rememberGraphicsLayer()
    var position by remember { mutableStateOf(Offset.Zero) }
    val fill = tint ?: Glass.chrome
    val edgeBrush = Glass.edge
    this
        .onGloballyPositioned { position = it.positionInRoot() }
        .clip(shape)
        .drawBehind {
            val r = blur.toPx()
            blurred.renderEffect = BlurEffect(r, r, TileMode.Clamp)
            val shift = backdrop.origin - position
            blurred.record { translate(shift.x, shift.y) { drawLayer(backdrop.layer) } }
            drawLayer(blurred)
            drawRect(fill)
        }
        .then(if (edge) Modifier.border(0.75.dp, edgeBrush, shape) else Modifier)
}
