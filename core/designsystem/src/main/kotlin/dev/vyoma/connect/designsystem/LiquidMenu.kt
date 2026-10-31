package dev.vyoma.connect.designsystem

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.Animatable
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
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Rect
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
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** One line of a long-press menu. */
class MenuItem(val label: String, val icon: ImageVector? = null, val destructive: Boolean = false, val onClick: () -> Unit)

internal class MenuRequest(val id: Long, val from: Rect, val items: List<MenuItem>, val title: String?)

/** Opens long-press menus; get it from [LocalMenu]. */
class MenuController {
    internal var request by mutableStateOf<MenuRequest?>(null)
    private var seq = 0L

    /** [from] is the pressed element's bounds in root coordinates (`boundsInRoot()`). */
    fun show(from: Rect, items: List<MenuItem>, title: String? = null) {
        request = MenuRequest(++seq, from, items, title)
    }
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
 * mostly formed. Closing, it flows back into the element.
 */
@Composable
private fun MenuOverlay(r: MenuRequest, backdrop: Backdrop, onGone: () -> Unit) {
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val wide = remember { Animatable(0f) }
    val tall = remember { Animatable(0f) }
    var closing by remember { mutableStateOf(false) }
    val bars = WindowInsets.systemBars
    LaunchedEffect(Unit) {
        haptics.perform(Haptic.Hold)
        launch { wide.animateTo(1f, VyomaMotion.leading()) }
        tall.animateTo(1f, VyomaMotion.jelly())
    }
    fun close(then: () -> Unit = {}) {
        if (closing) return
        closing = true
        scope.launch {
            launch { wide.animateTo(0f, VyomaMotion.page()) }
            tall.animateTo(0f, VyomaMotion.page())
            onGone()
        }
        // Act at once; the drop flows home while it happens.
        then()
    }
    BackHandler { close() }

    Box(
        Modifier
            .fillMaxSize()
            .drawBehind { drawRect(Color.Black, alpha = wide.value.coerceIn(0f, 1f) * SCRIM) }
            .pointerInput(Unit) { detectTapGestures { close() } },
    ) {
        Box(
            Modifier
                .layout { m, c ->
                    val from = r.from
                    val menuW = MENU_WIDTH.toPx()
                    val menuH = (if (r.title != null) TITLE_HEIGHT.toPx() else 0f) + ROW_HEIGHT.toPx() * r.items.size + PAD.toPx() * 2
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
                    val width = (rr - l).roundToInt().coerceAtLeast(1)
                    val height = (b - t).roundToInt().coerceAtLeast(1)
                    val p = m.measure(Constraints.fixed(width, height))
                    layout(c.maxWidth, c.maxHeight) { p.place(l.roundToInt(), t.roundToInt()) }
                }
                // Taps on the glass stay on the glass.
                .pointerInput(Unit) { detectTapGestures { } }
                .glassChrome(backdrop, MenuShape, blur = 16.dp, bevel = 16.dp)
                .clip(MenuShape),
        ) {
            // Laid out at full size from the start and revealed by the drop, so nothing reflows.
            Column(
                Modifier
                    .wrapContentSize(Alignment.TopStart, unbounded = true)
                    .requiredWidth(MENU_WIDTH)
                    .graphicsLayer {
                        alpha = ((minOf(wide.value, tall.value) - 0.55f) / 0.35f).coerceIn(0f, 1f)
                        compositingStrategy = CompositingStrategy.ModulateAlpha
                    }
                    .padding(vertical = PAD),
            ) {
                if (r.title != null) {
                    Text(
                        r.title, style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.height(TITLE_HEIGHT).padding(start = 20.dp, end = 20.dp, top = 12.dp),
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
                    if (i < r.items.lastIndex) HorizontalDivider(Modifier.padding(horizontal = 20.dp), color = MaterialTheme.colorScheme.outlineVariant)
                }
            }
        }
    }
}

private val MenuShape = RoundedCornerShape(24.dp)
private val MENU_WIDTH = 240.dp
private val ROW_HEIGHT = 52.dp
private val TITLE_HEIGHT = 40.dp
private val PAD = 8.dp
private const val SCRIM = 0.35f
