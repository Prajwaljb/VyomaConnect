package dev.vyoma.connect.designsystem

import androidx.compose.foundation.shape.CornerBasedShape
import androidx.compose.foundation.shape.CornerSize
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.sin

/**
 * Apple's continuous ("squircle") corners, as on macOS/iOS 26 tiles, cards and icons: the curve starts
 * further along the edge and eases into it, so there's no visible point where a straight side meets a
 * circular arc. Apple's corner (Figma's 60% "iOS" smoothing: a circular arc of the radius over the middle 36°,
 * eased into each side over 1.6x the radius) is matched within 0.6% of the radius by a superellipse quarter
 * ([EXPONENT] 2.75) spanning [SPAN] times the radius; each corner is one. A squarer exponent (4.2 was tried)
 * pinches the apex into a visible point. Corners are
 * kept within half the short side (so a small radius reads like a rounded rect, never a capsule).
 */
class SmoothCornerShape(
    topStart: CornerSize,
    topEnd: CornerSize,
    bottomEnd: CornerSize,
    bottomStart: CornerSize,
) : CornerBasedShape(topStart, topEnd, bottomEnd, bottomStart) {

    constructor(radius: Dp) : this(CornerSize(radius), CornerSize(radius), CornerSize(radius), CornerSize(radius))

    override fun copy(topStart: CornerSize, topEnd: CornerSize, bottomEnd: CornerSize, bottomStart: CornerSize) =
        SmoothCornerShape(topStart, topEnd, bottomEnd, bottomStart)

    override fun createOutline(
        size: Size, topStart: Float, topEnd: Float, bottomEnd: Float, bottomStart: Float, layoutDirection: LayoutDirection,
    ): Outline {
        if (topStart + topEnd + bottomEnd + bottomStart == 0f) return Outline.Rectangle(Rect(0f, 0f, size.width, size.height))
        val rtl = layoutDirection == LayoutDirection.Rtl
        val tl = span(if (rtl) topEnd else topStart, size)
        val tr = span(if (rtl) topStart else topEnd, size)
        val br = span(if (rtl) bottomStart else bottomEnd, size)
        val bl = span(if (rtl) bottomEnd else bottomStart, size)
        val w = size.width
        val h = size.height
        val path = Path()
        path.moveTo(tl, 0f)
        path.lineTo(w - tr, 0f)
        corner(path, w - tr, tr, tr, 1f, -1f, fromVertical = true)
        path.lineTo(w, h - br)
        corner(path, w - br, h - br, br, 1f, 1f, fromVertical = false)
        path.lineTo(bl, h)
        corner(path, bl, h - bl, bl, -1f, 1f, fromVertical = true)
        path.lineTo(0f, tl)
        corner(path, tl, tl, tl, -1f, -1f, fromVertical = false)
        path.close()
        return Outline.Generic(path)
    }

    private fun span(r: Float, size: Size) = (r * SPAN).coerceAtMost(size.minDimension / 2)

    /**
     * One superellipse quarter about ([cx], [cy]) with half-size [r], in the quadrant given by the signs.
     * Walks clockwise: from the horizontal edge down/up to the vertical one, or the other way round.
     */
    private fun corner(path: Path, cx: Float, cy: Float, r: Float, sx: Float, sy: Float, fromVertical: Boolean) {
        if (r <= 0f) return
        for (i in 1..STEPS) {
            val k = i.toFloat() / STEPS
            // θ runs from the edge we arrived on to the next one.
            val a = (if (fromVertical) 1f - k else k) * (Math.PI / 2)
            // (cos(π/2) comes out a hair below zero, and a fractional power of that is NaN.)
            val x = cos(a).toFloat().coerceAtLeast(0f).pow(2f / EXPONENT)
            val y = sin(a).toFloat().coerceAtLeast(0f).pow(2f / EXPONENT)
            path.lineTo(cx + sx * r * x, cy + sy * r * y)
        }
    }

    /** The corner's extent in px (where its curve starts), for glass that shades the edge with [EXPONENT]. */
    fun radiusPx(size: Size, density: Density): Float = span(topStart.toPx(size, density), size)

    override fun equals(other: Any?) = other is SmoothCornerShape && topStart == other.topStart && topEnd == other.topEnd &&
        bottomEnd == other.bottomEnd && bottomStart == other.bottomStart

    override fun hashCode() = ((topStart.hashCode() * 31 + topEnd.hashCode()) * 31 + bottomEnd.hashCode()) * 31 + bottomStart.hashCode()

    companion object {
        const val EXPONENT = 2.75f
        private const val SPAN = 1.3f
        private const val STEPS = 14
    }
}
