package dev.vyoma.connect.designsystem

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * [style] at the largest size, at most its own, at which every one of [texts] fits on one line in
 * [width]; never below [floor] on screen. One size for a set of labels (Control Center tiles, accent
 * swatches): a long one shrinking alone looked like a different control. At larger text and display
 * sizes (a OnePlus at its larger defaults cut "Commands") they all step down together.
 */
@Composable
fun fitTogether(texts: List<String>, style: TextStyle, width: Dp, floor: Dp = 8.dp): TextStyle {
    val measurer = rememberTextMeasurer(cacheSize = 0)
    val density = LocalDensity.current
    return remember(texts, style, width, density) {
        val room = with(density) { width.roundToPx() }.coerceAtLeast(1)
        val min = with(density) { floor.toSp() }.value
        var size = style.fontSize.value
        fun fits(s: Float) = texts.all {
            measurer.measure(
                it, style.copy(fontSize = s.sp), overflow = TextOverflow.Clip, softWrap = false, maxLines = 1,
                constraints = Constraints(), density = density,
            ).size.width <= room
        }
        while (size > min && !fits(size)) size -= 0.5f
        style.copy(fontSize = maxOf(size, min).sp)
    }
}
