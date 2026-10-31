package dev.vyoma.connect.designsystem

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.util.lerp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** One line of a [LiquidMenu]. */
class MenuItem(val label: String, val icon: ImageVector? = null, val destructive: Boolean = false, val onClick: () -> Unit)

/**
 * A long press's menu, grown out of the thing pressed like a drop of liquid glass: it starts as
 * exactly that element (its rect and corner radius, [from] in window coordinates), and stretches
 * into the menu, its leading edge on a quick spring and its trailing edge and height on softer,
 * wobbling ones, so it pours out instead of scaling. Its rim catches the tilt light like the
 * chrome; the items fade in once it has mostly formed. Dismissed (outside tap, back, or a pick), it
 * flows back into the element and is gone.
 */
@Composable
fun LiquidMenu(
    from: Rect,
    fromRadius: Dp,
    items: List<MenuItem>,
    onDismiss: () -> Unit,
    title: String? = null,
) {
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val window = LocalWindowInfo.current.containerSize
    // Width and height pour out on different springs: the drop stretches before it settles.
    val wide = remember { Animatable(0f) }
    val tall = remember { Animatable(0f) }
    var closing by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        launch { wide.animateTo(1f, VyomaMotion.leading()) }
        tall.animateTo(1f, VyomaMotion.jelly())
    }
    fun close(then: () -> Unit = {}) {
        if (closing) return
        closing = true
        scope.launch {
            launch { tall.animateTo(0f, VyomaMotion.page()) }
            wide.animateTo(0f, VyomaMotion.page())
            onDismiss()
            then()
        }
    }

    val menuW = with(density) { MENU_WIDTH.toPx() }
    val rowH = with(density) { ROW_HEIGHT.toPx() }
    val titleH = if (title != null) with(density) { 40.dp.toPx() } else 0f
    val pad = with(density) { 8.dp.toPx() }
    val margin = with(density) { 16.dp.toPx() }
    val gap = with(density) { 10.dp.toPx() }
    val menuH = titleH + rowH * items.size + pad * 2
    // Above the element when there's room (thumb-side menus read from the finger up), else below;
    // aligned to its nearer side and kept on screen.
    val left = (if (from.center.x < window.width / 2f) from.left else from.right - menuW)
        .coerceIn(margin, (window.width - margin - menuW).coerceAtLeast(margin))
    val top = if (from.top - gap - menuH > margin * 4) from.top - gap - menuH else from.bottom + gap
    val target = Rect(left, top, left + menuW, top + menuH)
    val endRadius = with(density) { 24.dp.toPx() }
    val startRadius = with(density) { fromRadius.toPx() }
    val dark = isDarkUi
    val fill = MaterialTheme.colorScheme.surfaceContainerHigh

    Popup(popupPositionProvider = WindowOrigin, onDismissRequest = { close() }, properties = PopupProperties(focusable = true)) {
        Box(
            Modifier
                .fillMaxSize()
                .pointerInput(Unit) { detectTapGestures { close() } }
                // The page behind dims as the drop forms.
                .drawBehind { drawRect(Color.Black, alpha = wide.value.coerceIn(0f, 1f) * if (dark) 0.45f else 0.18f) },
        ) {
            // The drop itself, drawn every frame from the two springs.
            Box(
                Modifier
                    .fillMaxSize()
                    .drawBehindDrop(fill, dark) {
                        val w = wide.value
                        val h = tall.value
                        val l = lerp(from.left, target.left, w)
                        val r = lerp(from.right, target.right, w)
                        val t = lerp(from.top, target.top, h)
                        val b = lerp(from.bottom, target.bottom, h)
                        DropFrame(Rect(l, t, r, b), lerp(startRadius, endRadius, w.coerceIn(0f, 1f)))
                    },
            )
            // The items, faded in as it forms, clipped to where the drop has reached.
            Layout(
                content = {
                    Column(
                        Modifier
                            .width(MENU_WIDTH)
                            .graphicsLayer {
                                alpha = ((minOf(wide.value, tall.value) - 0.55f) / 0.35f).coerceIn(0f, 1f)
                                compositingStrategy = CompositingStrategy.ModulateAlpha
                            }
                            .padding(vertical = 8.dp),
                    ) {
                        if (title != null) {
                            Text(
                                title, style = MaterialTheme.typography.labelLarge,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1, overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.height(40.dp).padding(start = 20.dp, end = 20.dp, top = 12.dp),
                            )
                        }
                        items.forEachIndexed { i, item ->
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
                            if (i < items.lastIndex) HorizontalDivider(Modifier.padding(horizontal = 20.dp), color = MaterialTheme.colorScheme.outlineVariant)
                        }
                    }
                },
            ) { measurables, constraints ->
                val p = measurables.first().measure(Constraints())
                layout(constraints.maxWidth, constraints.maxHeight) { p.place(target.left.roundToInt(), target.top.roundToInt()) }
            }
        }
    }
    LaunchedEffect(Unit) { haptics.perform(Haptic.Hold) }
}

private val MENU_WIDTH = 240.dp
private val ROW_HEIGHT = 52.dp

private class DropFrame(val rect: Rect, val radius: Float)

/**
 * The drop: an opaque glass slab (a popup can't see the page to refract it) with a soft depth
 * shadow, a band of light just inside the edge and a tilt-lit rim, like the chrome's edge.
 */
private fun Modifier.drawBehindDrop(fill: Color, dark: Boolean, frame: () -> DropFrame) = this.then(
    Modifier.drawBehind {
        val f = frame()
        val r = f.rect
        if (r.width <= 0f || r.height <= 0f) return@drawBehind
        val cr = CornerRadius(f.radius.coerceAtMost(minOf(r.width, r.height) / 2f))
        // Depth: a few soft rings of shadow under the slab.
        for (i in 1..4) {
            val grow = i * 3.dp.toPx()
            drawRoundRect(
                Color.Black.copy(alpha = if (dark) 0.10f else 0.04f),
                topLeft = Offset(r.left - grow / 2, r.top - grow / 2 + i * 2.dp.toPx()),
                size = Size(r.width + grow, r.height + grow),
                cornerRadius = CornerRadius(cr.x + grow / 2),
            )
        }
        drawRoundRect(fill, topLeft = r.topLeft, size = r.size, cornerRadius = cr)
        // A band of light inside the edge, brightest towards the light (as `InnerGlow` on chrome).
        val a = Math.toRadians(SpecularLight.angle.toDouble())
        val dx = kotlin.math.cos(a).toFloat()
        val dy = kotlin.math.sin(a).toFloat()
        val c = r.center
        val reach = maxOf(r.width, r.height) / 2f
        val lit = Brush.linearGradient(
            0f to Color.White.copy(alpha = if (dark) 0.42f else 0.9f),
            0.5f to Color.White.copy(alpha = if (dark) 0.06f else 0.2f),
            1f to Color.White.copy(alpha = if (dark) 0.3f else 0.7f),
            start = Offset(c.x + dx * reach, c.y + dy * reach),
            end = Offset(c.x - dx * reach, c.y - dy * reach),
        )
        val inset = 0.5.dp.toPx()
        drawRoundRect(
            lit,
            topLeft = Offset(r.left + inset, r.top + inset),
            size = Size(r.width - inset * 2, r.height - inset * 2),
            cornerRadius = CornerRadius(cr.x - inset),
            style = Stroke(1.dp.toPx()),
        )
        drawRoundRect(
            lit, alpha = 0.18f,
            topLeft = Offset(r.left + 3.dp.toPx(), r.top + 3.dp.toPx()),
            size = Size(r.width - 6.dp.toPx(), r.height - 6.dp.toPx()),
            cornerRadius = CornerRadius((cr.x - 3.dp.toPx()).coerceAtLeast(0f)),
            style = Stroke(5.dp.toPx()),
        )
    },
)

/** Puts the popup's window over the whole app window, so window coordinates work inside it. */
private object WindowOrigin : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize) = IntOffset.Zero
}
