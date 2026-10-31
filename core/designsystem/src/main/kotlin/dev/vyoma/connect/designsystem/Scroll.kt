package dev.vyoma.connect.designsystem

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * A horizontal row that scrolls under soft edges instead of being cut off. [bleed] widens it past
 * its slot on both sides (a row in the page's gutter runs to the screen edge, like iOS); give the
 * content the same padding so it still starts in line. Each edge fades only while there's more to
 * scroll that way, easing in over the first [fade] of travel.
 */
fun Modifier.fadingHorizontalScroll(state: ScrollState, bleed: Dp = 0.dp, fade: Dp = 28.dp): Modifier = this
    .layout { m, c ->
        val b = bleed.roundToPx()
        val wide = if (c.hasBoundedWidth) c.copy(minWidth = c.minWidth + 2 * b, maxWidth = c.maxWidth + 2 * b) else c
        val p = m.measure(wide)
        layout((p.width - 2 * b).coerceAtLeast(0), p.height) { p.place(-b, 0) }
    }
    .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
    .drawWithContent {
        drawContent()
        val f = fade.toPx().coerceAtMost(size.width / 3f)
        val start = (state.value / f).coerceIn(0f, 1f)
        val end = ((state.maxValue - state.value) / f).coerceIn(0f, 1f)
        if (start > 0f) drawRect(
            Brush.horizontalGradient(0f to Color.Black.copy(alpha = 1f - start), 1f to Color.Black, startX = 0f, endX = f),
            size = size.copy(width = f), blendMode = BlendMode.DstIn,
        )
        if (end > 0f) drawRect(
            Brush.horizontalGradient(0f to Color.Black, 1f to Color.Black.copy(alpha = 1f - end), startX = size.width - f, endX = size.width),
            topLeft = Offset(size.width - f, 0f), size = size.copy(width = f), blendMode = BlendMode.DstIn,
        )
    }
    .horizontalScroll(state)

/** The vertical twin of [fadingHorizontalScroll], for a capped list inside a dialog or card. */
fun Modifier.fadingVerticalScroll(state: ScrollState, fade: Dp = 28.dp): Modifier = this
    .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
    .drawWithContent {
        drawContent()
        val f = fade.toPx().coerceAtMost(size.height / 3f)
        val top = (state.value / f).coerceIn(0f, 1f)
        val bottom = ((state.maxValue - state.value) / f).coerceIn(0f, 1f)
        if (top > 0f) drawRect(
            Brush.verticalGradient(0f to Color.Black.copy(alpha = 1f - top), 1f to Color.Black, startY = 0f, endY = f),
            size = size.copy(height = f), blendMode = BlendMode.DstIn,
        )
        if (bottom > 0f) drawRect(
            Brush.verticalGradient(0f to Color.Black, 1f to Color.Black.copy(alpha = 1f - bottom), startY = size.height - f, endY = size.height),
            topLeft = Offset(0f, size.height - f), size = size.copy(height = f), blendMode = BlendMode.DstIn,
        )
    }
    .verticalScroll(state)
