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
    onClick: () -> Unit,
): Modifier = composed {
    val haptics = LocalHapticFeedback.current
    this
        .pressScale(pressedScale, enabled)
        .clickable(interactionSource = null, indication = null, enabled = enabled, role = role) {
            haptic?.let(haptics::perform)
            onClick()
        }
}

/**
 * Shrinks the moment a finger lands (no tap-timeout lag like interaction-based presses) and
 * springs back with a small overshoot when it lifts. Even a quick tap visibly dips first. If a
 * scrolling parent takes the gesture over, it eases back without the bounce.
 *
 * A modifier node that only touches its own graphics layer: presses never recompose or relayout.
 */
fun Modifier.pressScale(pressedScale: Float = 0.96f, enabled: Boolean = true): Modifier =
    this then PressScaleElement(pressedScale, enabled)

private class PressScaleElement(val pressedScale: Float, val enabled: Boolean) : ModifierNodeElement<PressScaleNode>() {
    override fun create() = PressScaleNode(pressedScale, enabled)
    override fun update(node: PressScaleNode) {
        node.pressedScale = pressedScale
        node.enabled = enabled
    }
    override fun equals(other: Any?) = other is PressScaleElement && other.pressedScale == pressedScale && other.enabled == enabled
    override fun hashCode() = pressedScale.hashCode() * 31 + enabled.hashCode()
}

private class PressScaleNode(var pressedScale: Float, var enabled: Boolean) :
    Modifier.Node(), PointerInputModifierNode, LayoutModifierNode {
    private val scale = Animatable(1f)
    private var held = false
    private var job: Job? = null

    override fun onPointerEvent(pointerEvent: PointerEvent, pass: PointerEventPass, bounds: IntSize) {
        if (!enabled) return
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
        job?.cancel()
        job = coroutineScope.launch { scale.animateTo(pressedScale, PRESS) }
    }

    private fun release(bounce: Boolean) {
        held = false
        job?.cancel()
        job = coroutineScope.launch {
            if (bounce) {
                val depth = 1f - pressedScale
                if (scale.value > pressedScale + depth * 0.4f) scale.animateTo(pressedScale + depth * 0.25f, DIP)
                scale.animateTo(1f, VyomaMotion.release())
            } else {
                scale.animateTo(1f, VyomaMotion.effects())
            }
        }
    }

    override fun MeasureScope.measure(measurable: Measurable, constraints: Constraints): MeasureResult {
        val placeable = measurable.measure(constraints)
        return layout(placeable.width, placeable.height) {
            placeable.placeWithLayer(0, 0) {
                val s = scale.value
                scaleX = s
                scaleY = s
            }
        }
    }

    private companion object {
        val PRESS = spring<Float>(dampingRatio = 1f, stiffness = 1400f)
        val DIP = tween<Float>(70, easing = FastOutLinearInEasing)
    }
}
