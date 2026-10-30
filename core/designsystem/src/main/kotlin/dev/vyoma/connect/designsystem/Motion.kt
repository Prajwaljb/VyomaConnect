package dev.vyoma.connect.designsystem

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutLinearInEasing
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.clickable
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.changedToDownIgnoreConsumed
import androidx.compose.ui.input.pointer.positionChangedIgnoreConsumed
import androidx.compose.ui.layout.Measurable
import androidx.compose.ui.layout.MeasureResult
import androidx.compose.ui.layout.MeasureScope
import androidx.compose.ui.node.LayoutModifierNode
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.node.PointerInputModifierNode
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.node.DrawModifierNode
import androidx.compose.ui.node.CompositionLocalConsumerModifierNode
import androidx.compose.ui.node.currentValueOf
import androidx.compose.ui.node.requireGraphicsContext
import androidx.compose.ui.graphics.drawscope.ContentDrawScope
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.node.invalidateDraw
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.material3.LocalContentColor
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * Shared springs. Spatial motion slightly overshoots, color/alpha motion never does, and things
 * you let go of ([release]) bounce back like iOS: a quick settle with one visible wobble.
 */
object VyomaMotion {
    fun <T> spatial() = spring<T>(dampingRatio = 0.78f, stiffness = 420f)
    fun <T> spatialSlow() = spring<T>(dampingRatio = 0.82f, stiffness = Spring.StiffnessLow)
    fun <T> effects() = spring<T>(dampingRatio = Spring.DampingRatioNoBouncy, stiffness = Spring.StiffnessMediumLow)
    /** Letting go of something pressed or stretched. */
    fun <T> release() = spring<T>(dampingRatio = 0.48f, stiffness = 520f)
    /** Page pushes and pops: quick, no overshoot, so the incoming page never wobbles. */
    fun <T> page() = spring<T>(dampingRatio = 0.92f, stiffness = 380f)
    /** A gel that trails the leading edge, for the tab lens and the switch thumb. */
    fun <T> trailing() = spring<T>(dampingRatio = 0.7f, stiffness = 260f)
    fun <T> leading() = spring<T>(dampingRatio = 0.8f, stiffness = 900f)
    /** The second axis of a released jelly: softer than [release], so width and height wobble out of step. */
    fun <T> jelly() = spring<T>(dampingRatio = 0.42f, stiffness = 360f)
}

/**
 * What a haptic means, so the same kind of action feels the same everywhere. Goes through the
 * view's haptics, which respect the user's "touch feedback" setting and cost nothing to call.
 */
enum class Haptic(internal val type: HapticFeedbackType) {
    /** A button or card did something. */
    Tap(HapticFeedbackType.ContextClick),
    /** A key on a keyboard-like surface: lighter, for rapid repeats. */
    Key(HapticFeedbackType.KeyboardTap),
    ToggleOn(HapticFeedbackType.ToggleOn),
    ToggleOff(HapticFeedbackType.ToggleOff),
    /** Moving onto a new option: tabs, segmented choices. */
    Select(HapticFeedbackType.SegmentTick),
    /** A detent while dragging a value: volume steps, scroll notches. */
    Detent(HapticFeedbackType.SegmentFrequentTick),
    /** Hitting an end stop or crossing a commit threshold. */
    Edge(HapticFeedbackType.GestureThresholdActivate),
    /** A hold turned into a grab. */
    Hold(HapticFeedbackType.LongPress),
    Success(HapticFeedbackType.Confirm),
    Failure(HapticFeedbackType.Reject),
}

fun HapticFeedback.perform(h: Haptic) = performHapticFeedback(h.type)

/**
 * Clickable that squishes on press and bounces back on release, with a [haptic] on click: the
 * One UI / iOS feel. See [pressScale].
 */
fun Modifier.pressable(
    enabled: Boolean = true,
    pressedScale: Float = 0.96f,
    role: Role? = Role.Button,
    haptic: Haptic? = Haptic.Tap,
    /** Capsules and circles only: iOS 26 interactive glass. Instead of dipping it swells under the
     *  finger, clears into a drop of liquid glass and lights up where it's touched. [pressedScale]
     *  is ignored then. */
    glass: Boolean = false,
    onClick: () -> Unit,
): Modifier = composed {
    val haptics = LocalHapticFeedback.current
    this
        .pressScale(pressedScale, enabled, glass)
        .clickable(interactionSource = null, indication = null, enabled = enabled, role = role) {
            haptic?.let(haptics::perform)
            onClick()
        }
}

/**
 * Shrinks the moment a finger lands (no tap-timeout lag like interaction-based presses) and
 * springs back when it lifts like a drop of water: width and height settle on different springs,
 * so it wobbles out of step instead of just scaling. Even a quick tap visibly dips first. If a
 * scrolling parent takes the gesture over, it eases back without the bounce.
 *
 * A modifier node that only touches its own graphics layer: presses never recompose or relayout.
 */
fun Modifier.pressScale(pressedScale: Float = 0.96f, enabled: Boolean = true, glass: Boolean = false): Modifier =
    this then PressScaleElement(pressedScale, enabled, glass && liquidGlassSupported)

private data class PressScaleElement(val pressedScale: Float, val enabled: Boolean, val glass: Boolean) : ModifierNodeElement<PressScaleNode>() {
    override fun create() = PressScaleNode(pressedScale, enabled, glass)
    override fun update(node: PressScaleNode) {
        node.pressedScale = pressedScale
        node.enabled = enabled
        node.glass = glass
    }
}

private class PressScaleNode(var pressedScale: Float, var enabled: Boolean, var glass: Boolean) :
    Modifier.Node(), PointerInputModifierNode, LayoutModifierNode, DrawModifierNode, CompositionLocalConsumerModifierNode {
    private val sx = Animatable(1f)
    private val sy = Animatable(1f)
    /** How clear the press drop is: up while held, eases out after. Only animated with [glass]. */
    private val clear = Animatable(0f)
    private var painter: LensPainter? = null
    private var held = false
    private var job: Job? = null
    private var glassJob: Job? = null
    /** Where the finger is, for the glow; and the node's size, for how much it may swell. */
    private var touch = Offset.Unspecified
    private var extent = 0f

    override fun ContentDrawScope.draw() {
        val a = if (glass) clear.value else 0f
        if (a < 0.01f || !liquidGlassSupported) {
            drawContent()
            return
        }
        val dark = currentValueOf(LocalContentColor).luminance() > 0.5f
        val p = painter ?: LensPainter(requireGraphicsContext()).also { painter = it }
        with(p) { paint(a, Rect(Offset.Zero, size), 1f + 0.06f * a, LensStyle.Drop, Color.Transparent, dark, null, Offset.Zero) }
        // Light gathers under the finger and follows it, inside the capsule.
        val at = if (touch.isSpecified) touch else center
        val r = size.minDimension / 2
        clipPath(Path().apply { addRoundRect(RoundRect(0f, 0f, size.width, size.height, CornerRadius(r))) }) {
            drawCircle(
                Brush.radialGradient(listOf(Color.White.copy(alpha = (if (dark) 0.22f else 0.4f) * a), Color.Transparent), at, size.maxDimension * 0.7f),
                radius = size.maxDimension * 0.7f, center = at,
            )
        }
    }

    override fun onDetach() {
        painter?.release()
        painter = null
    }

    private fun clearTo(target: Float) {
        if (!glass) return
        glassJob?.cancel()
        glassJob = coroutineScope.launch { clear.animateTo(target, if (target > 0f) VyomaMotion.spatial() else VyomaMotion.effects()) }
    }

    override fun onPointerEvent(pointerEvent: PointerEvent, pass: PointerEventPass, bounds: IntSize) {
        if (!enabled) return
        if (glass && pass == PointerEventPass.Initial) {
            pointerEvent.changes.firstOrNull { it.pressed }?.let {
                touch = it.position
                if (held) invalidateDraw()
            }
        }
        when (pass) {
            PointerEventPass.Initial ->
                if (!held && pointerEvent.changes.any { it.changedToDownIgnoreConsumed() }) press()
            PointerEventPass.Final -> if (held) {
                // Moves consumed by someone else mean a scroll or drag owns this gesture now.
                val stolen = pointerEvent.changes.any { it.pressed && it.isConsumed && it.positionChangedIgnoreConsumed() }
                if (stolen) release(bounce = false)
                else if (pointerEvent.changes.none { it.pressed }) release(bounce = true)
            }
            else -> Unit
        }
    }

    override fun onCancelPointerInput() {
        if (held) release(bounce = false)
    }

    private fun press() {
        held = true
        clearTo(1f)
        job?.cancel()
        // Height gives a beat later than width: a squash, not a shrink.
        val target = if (glass) 1f + (GLASS_GROW_DP / extent.coerceAtLeast(1f)).coerceIn(0.03f, 0.13f) else pressedScale
        job = coroutineScope.launch {
            if (glass) {
                // Swells like a drop being lifted: springy, a little ahead in width.
                launch { sy.animateTo(target, VyomaMotion.spatial()) }
                sx.animateTo(target, VyomaMotion.spatial())
            } else {
                launch { sy.animateTo(target, PRESS_SOFT) }
                sx.animateTo(target, PRESS)
            }
        }
    }

    private fun release(bounce: Boolean) {
        held = false
        clearTo(0f)
        job?.cancel()
        job = coroutineScope.launch {
            if (glass) {
                // Set down: it settles back with a wobble, width and height out of step.
                launch { sy.animateTo(1f, VyomaMotion.jelly()) }
                sx.animateTo(1f, VyomaMotion.release())
            } else if (bounce) {
                val depth = 1f - pressedScale
                val dip = pressedScale + depth * 0.25f
                if (sx.value > pressedScale + depth * 0.4f) {
                    launch { sy.animateTo(dip, DIP) }
                    sx.animateTo(dip, DIP)
                }
                // Thrown back out: overshoots past full size, then wobbles in (width and height out of step).
                val kick = depth * 34f
                launch { sy.animateTo(1f, VyomaMotion.jelly(), initialVelocity = kick * 0.8f) }
                sx.animateTo(1f, VyomaMotion.release(), initialVelocity = kick)
            } else {
                launch { sy.animateTo(1f, VyomaMotion.effects()) }
                sx.animateTo(1f, VyomaMotion.effects())
            }
        }
    }

    override fun MeasureScope.measure(measurable: Measurable, constraints: Constraints): MeasureResult {
        val placeable = measurable.measure(constraints)
        extent = maxOf(placeable.width, placeable.height) / density
        return layout(placeable.width, placeable.height) {
            placeable.placeWithLayer(0, 0) {
                scaleX = sx.value
                scaleY = sy.value
            }
        }
    }

    private companion object {
        val PRESS = spring<Float>(dampingRatio = 1f, stiffness = 1400f)
        val PRESS_SOFT = spring<Float>(dampingRatio = 0.9f, stiffness = 800f)
        val DIP = tween<Float>(70, easing = FastOutLinearInEasing)
        /** How far interactive glass swells past its edges, in dp: a circle grows more than a wide pill. */
        const val GLASS_GROW_DP = 7f
    }
}
