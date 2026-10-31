package dev.vyoma.connect.designsystem

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.requiredWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Rect
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.ClipOp
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import androidx.compose.ui.util.lerp
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/**
 * One line of a long-press menu. [groupEnd] puts a separator after it: like iOS, separators split
 * groups, they don't sit between every item.
 */
class MenuItem(
    val label: String,
    val icon: ImageVector? = null,
    val destructive: Boolean = false,
    val groupEnd: Boolean = false,
    val onClick: () -> Unit,
)

internal class MenuRequest(
    val id: Long,
    val from: Rect,
    val fromRadius: Dp,
    val items: List<MenuItem>,
    val title: String?,
    val morph: MenuMorph?,
)

/**
 * The pressed element turns INTO the menu: the drop starts as an exact copy of it ([color] and
 * [face], its own content at its own size) that clears into glass as it grows, and closing turns
 * back into it before handing over, so nothing swaps. The element hides itself while
 * [MenuController.covers] says so.
 */
class MenuMorph(val owner: Any, val color: Color, val face: @Composable () -> Unit)

/** Opens long-press menus; get it from [LocalMenu]. */
class MenuController {
    internal var request by mutableStateOf<MenuRequest?>(null)
    private var seq = 0L

    /**
     * [from] is the pressed element's bounds in root coordinates (`boundsInRoot()`), [fromRadius]
     * its corner: it stays lit through the dim, like iOS, so you see where the menu came from.
     */
    fun show(from: Rect, items: List<MenuItem>, title: String? = null, fromRadius: Dp = 16.dp, morph: MenuMorph? = null) {
        request = MenuRequest(++seq, from, fromRadius, items, title, morph)
    }

    /** True while [owner]'s menu stands in for it (read it in a graphicsLayer to hide the element). */
    fun covers(owner: Any): Boolean = request?.morph?.owner === owner
}

val LocalMenu = staticCompositionLocalOf { MenuController() }

/**
 * Hosts long-press menus over the whole app, in the app's own window so the menu can be real
 * liquid glass: while one is open, the app is recorded as a backdrop and the menu is the same
 * chrome glass as the tab bar ([glassChrome], blur + Snell refraction at its rim) over the page.
 * (A popup window can't see the page behind it, so the first version was an opaque slab.)
 */
@Composable
fun MenuHost(content: @Composable () -> Unit) {
    val controller = remember { MenuController() }
    val backdrop = rememberBackdrop()
    CompositionLocalProvider(LocalMenu provides controller) {
        Box(Modifier.fillMaxSize()) {
            // Recording costs a little every frame, so only while a menu is open.
            Box(Modifier.fillMaxSize().then(if (controller.request != null) Modifier.backdropSource(backdrop) else Modifier)) { content() }
            controller.request?.let { r ->
                key(r.id) { MenuOverlay(r, backdrop) { if (controller.request?.id == r.id) controller.request = null } }
            }
        }
    }
}

/**
 * The menu pours out of the pressed element like a drop of liquid glass: it starts as exactly that
 * element's rect and stretches into the menu, its width on a quick spring and its height on a
 * softer, wobbling one, so it flows out rather than scales. The page dims behind it (the glass
 * still refracts the undimmed page, so it glows against the dim). The items fade in once it has
 * mostly formed. Closing, the items go first and the empty drop drains back into the element.
 */
@Composable
private fun MenuOverlay(r: MenuRequest, backdrop: Backdrop, onGone: () -> Unit) {
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val wide = remember { Animatable(0f) }
    val tall = remember { Animatable(0f) }
    val scrim = remember { Animatable(0f) }
    val ink = remember { Animatable(0f) }
    var closing by remember { mutableStateOf(false) }
    val bars = WindowInsets.systemBars
    // Where the drop is this frame (set while laying it out), so the element's face can hold still.
    val origin = remember { FloatArray(2) }
    fun formed() = minOf(wide.value, tall.value).coerceIn(0f, 1f)
    // Its corners round from the element's into the menu's as it forms (read while drawing).
    val shape = remember(r.id) {
        object : Shape {
            override fun createOutline(size: Size, layoutDirection: LayoutDirection, density: Density): Outline {
                val corner = with(density) { lerp(r.fromRadius.toPx(), MENU_RADIUS.toPx(), formed()) }
                return Outline.Rounded(RoundRect(0f, 0f, size.width, size.height, CornerRadius(corner)))
            }
        }
    }
    LaunchedEffect(Unit) {
        haptics.perform(Haptic.Hold)
        launch { scrim.animateTo(1f, VyomaMotion.effects()) }
        launch { wide.animateTo(1f, POUR_WIDE) }
        launch { tall.animateTo(1f, POUR_TALL) }
        // The items appear once the drop has mostly formed, so they never ride its wobble.
        snapshotFlow { minOf(wide.value, tall.value) > 0.6f }.first { it }
        ink.animateTo(1f, VyomaMotion.effects())
    }
    fun close(then: () -> Unit = {}) {
        if (closing) return
        closing = true
        scope.launch {
            // Drains back towards the element while the items and the glass fade together: height
            // leads, width trails, so it narrows like water running home rather than shrinking as a box.
            launch { scrim.animateTo(0f, VyomaMotion.effects()) }
            launch { ink.animateTo(0f, INK_OUT) }
            val narrow = launch { wide.animateTo(0f, DRAIN_WIDE) }
            val lower = launch { tall.animateTo(0f, DRAIN_TALL) }
            if (r.morph != null) {
                // Turns all the way back into the element, then hands over to it: identical by then.
                narrow.join(); lower.join()
            } else {
                // The glass has faded out by here (see the drop's alpha): end without waiting for it
                // to settle on the element.
                snapshotFlow { minOf(wide.value, tall.value) <= GONE_AT }.first { it }
            }
            onGone()
        }
        // Act at once; the drop flows home while it happens.
        then()
    }
    BackHandler { close() }

    Box(
        Modifier
            .fillMaxSize()
            .drawBehind {
                val a = scrim.value.coerceIn(0f, 1f) * SCRIM
                if (a < 0.005f) return@drawBehind
                // A morphing element became the menu, so there's nothing left there to keep lit.
                if (r.morph != null) { drawRect(Color.Black, alpha = a); return@drawBehind }
                val hole = Path().apply { addRoundRect(RoundRect(r.from, CornerRadius(r.fromRadius.toPx()))) }
                clipPath(hole, ClipOp.Difference) { drawRect(Color.Black, alpha = a) }
            }
            // Closing, touches already reach the page again.
            .then(if (closing) Modifier else Modifier.pointerInput(Unit) { detectTapGestures { close() } }),
    ) {
        Box(
            Modifier
                .layout { m, c ->
                    val from = r.from
                    val menuW = MENU_WIDTH.toPx()
                    val menuH = (if (r.title != null) TITLE_HEIGHT.toPx() else 0f) + ROW_HEIGHT.toPx() * r.items.size +
                        GROUP_GAP.toPx() * r.items.dropLast(1).count { it.groupEnd } + PAD.toPx() * 2
                    val margin = 16.dp.toPx()
                    val gap = 10.dp.toPx()
                    // Above the element when it fits there (read from the finger up), else below, else
                    // on whichever side has more room; aligned to its nearer side; always on screen.
                    val left = (if (from.center.x < c.maxWidth / 2f) from.left else from.right - menuW)
                        .coerceIn(margin, (c.maxWidth - margin - menuW).coerceAtLeast(margin))
                    val minTop = bars.getTop(this) + margin
                    val maxBottom = c.maxHeight - bars.getBottom(this) - margin
                    val above = from.top - gap - menuH
                    val below = from.bottom + gap
                    val top = when {
                        above >= minTop -> above
                        below + menuH <= maxBottom -> below
                        from.top - minTop > maxBottom - from.bottom -> minTop
                        else -> maxBottom - menuH
                    }.coerceAtMost(maxBottom - menuH).coerceAtLeast(minTop)
                    val w = wide.value
                    val h = tall.value
                    val l = lerp(from.left, left, w)
                    val rr = lerp(from.right, left + menuW, w)
                    val t = lerp(from.top, top, h)
                    val b = lerp(from.bottom, top + menuH, h)
                    origin[0] = l; origin[1] = t
                    val width = (rr - l).roundToInt().coerceAtLeast(1)
                    val height = (b - t).roundToInt().coerceAtLeast(1)
                    val p = m.measure(Constraints.fixed(width, height))
                    layout(c.maxWidth, c.maxHeight) { p.place(l.roundToInt(), t.roundToInt()) }
                }
                // Taps on the glass stay on the glass.
                .then(if (closing) Modifier else Modifier.pointerInput(Unit) { detectTapGestures { } })
                // Closing, the glass dissolves the whole way while it drains towards the element
                // and is gone before it gets there: arriving whole, it sat over the button as an
                // opaque blurred blob, then vanished in one frame.
                .graphicsLayer {
                    alpha = if (!closing || r.morph != null) 1f else {
                        val t = ((minOf(wide.value, tall.value) - GONE_AT) / (1f - GONE_AT)).coerceIn(0f, 1f)
                        t * t * (3f - 2f * t)
                    }
                    compositingStrategy = CompositingStrategy.ModulateAlpha
                }
                .glassChrome(backdrop, shape, blur = 13.dp, bevel = 16.dp)
                // Morphing, the element's own colour, clearing into glass as the drop grows.
                .then(
                    if (r.morph == null) Modifier else Modifier.drawBehind {
                        val a = r.morph.color.alpha * (1f - smooth(formed() / 0.5f))
                        if (a > 0.003f) drawOutline(shape.createOutline(size, layoutDirection, this), r.morph.color.copy(alpha = a))
                    },
                )
                .clip(shape),
        ) {
            // The element's face, held where the element is, fading as the menu forms.
            if (r.morph != null) {
                Box(
                    Modifier
                        .wrapContentSize(Alignment.TopStart, unbounded = true)
                        .offset { IntOffset((r.from.left - origin[0]).roundToInt(), (r.from.top - origin[1]).roundToInt()) }
                        .requiredSize(with(LocalDensity.current) { DpSize(r.from.width.toDp(), r.from.height.toDp()) })
                        .graphicsLayer {
                            alpha = 1f - smooth(formed() / 0.25f)
                            compositingStrategy = CompositingStrategy.ModulateAlpha
                        },
                ) { r.morph.face() }
            }
            // Laid out at full size from the start and revealed by the drop, so nothing reflows.
            Column(
                Modifier
                    .wrapContentSize(Alignment.TopStart, unbounded = true)
                    .requiredWidth(MENU_WIDTH)
                    .graphicsLayer {
                        alpha = ink.value.coerceIn(0f, 1f)
                        compositingStrategy = CompositingStrategy.ModulateAlpha
                    }
                    .padding(vertical = PAD),
            ) {
                if (r.title != null) {
                    Text(
                        // Footnote-sized, like an iOS menu title: at item size it read as a disabled item.
                        r.title, style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.height(TITLE_HEIGHT).padding(start = 20.dp, end = 20.dp, top = 10.dp),
                    )
                }
                r.items.forEachIndexed { i, item ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .height(ROW_HEIGHT)
                            .pressable(haptic = Haptic.Tap) { close(item.onClick) }
                            .padding(horizontal = 20.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        val tint = if (item.destructive) SystemColor.RED.color else MaterialTheme.colorScheme.onSurface
                        Text(item.label, style = MaterialTheme.typography.bodyLarge, color = tint, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                        if (item.icon != null) {
                            Spacer(Modifier.width(12.dp))
                            Icon(item.icon, null, tint = tint, modifier = Modifier.size(22.dp))
                        }
                    }
                    if (item.groupEnd && i < r.items.lastIndex) {
                        HorizontalDivider(Modifier.padding(vertical = (GROUP_GAP - 1.dp) / 2), thickness = 1.dp, color = MaterialTheme.colorScheme.outlineVariant)
                    }
                }
            }
        }
    }
}

private val MENU_RADIUS = 24.dp

private fun smooth(x: Float): Float { val t = x.coerceIn(0f, 1f); return t * t * (3f - 2f * t) }
private val MENU_WIDTH = 240.dp
private val ROW_HEIGHT = 48.dp
private val TITLE_HEIGHT = 32.dp
private val GROUP_GAP = 9.dp
private val PAD = 8.dp
private const val SCRIM = 0.35f
/** Opening: the width runs out quickly, the height pours after it with a soft wobble. */
private val POUR_WIDE = spring<Float>(dampingRatio = 0.8f, stiffness = 700f)
private val POUR_TALL = spring<Float>(dampingRatio = 0.6f, stiffness = 340f)
/** Closing: no overshoot (a drop flowing home doesn't bounce off the tile), height a beat ahead. */
private val DRAIN_TALL = spring<Float>(dampingRatio = 1f, stiffness = 340f)
private val DRAIN_WIDE = spring<Float>(dampingRatio = 1f, stiffness = 220f)
private val INK_OUT = tween<Float>(110, easing = LinearEasing)
/** How far drained the drop is when it has faded out and the menu ends. */
private const val GONE_AT = 0.35f
