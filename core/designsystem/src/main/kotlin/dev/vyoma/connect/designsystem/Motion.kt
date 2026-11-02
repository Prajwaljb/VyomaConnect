package dev.vyoma.connect.designsystem

import kotlin.math.roundToInt
import androidx.compose.ui.unit.dp
import androidx.compose.ui.node.requireDensity
import android.content.ContentResolver
import android.content.Context
import android.os.Build
import android.view.HapticFeedbackConstants
import android.view.View
import android.os.VibrationAttributes
import android.os.VibrationEffect
import android.os.VibrationEffect.Composition.PRIMITIVE_CLICK
import android.os.VibrationEffect.Composition.PRIMITIVE_LOW_TICK
import android.os.VibrationEffect.Composition.PRIMITIVE_QUICK_RISE
import android.os.VibrationEffect.Composition.PRIMITIVE_THUD
import android.os.VibrationEffect.Composition.PRIMITIVE_TICK
import android.os.Vibrator
import android.os.VibratorManager
import android.provider.Settings
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
 * What a haptic means, so the same kind of action feels the same everywhere. Played the crispest way
 * the phone can (see [Haptics.engine]): composed from the motor's own primitives (crisp clicks, low
 * ticks, thuds, shaped and paired, like Pixel and One UI); else the maker's tuned click and tick,
 * but only where the phone says they're really tuned; else short pulses sized to the motor's
 * resonance, a cycle or two each, which an LRA plays as a click. The old last resorts buzzed:
 * Android's stand-in for an untuned predefined effect is a long plain vibration (a OnePlus Nord
 * CE3, 2026-11: "no ticks and clicks, only buzzes"), and the view's stock haptics stayed silent.
 * [Haptics] honours the system's touch feedback setting either way.
 */
enum class Haptic(
    internal val type: HapticFeedbackType,
    internal val predefined: Int,
    /** For [HapticEngine.SYSTEM]: the long-standing constants every maker maps (newer ones were silent on a OnePlus). */
    internal val system: Int,
    internal val notes: List<Note>,
) {
    /** A button or card did something. 0.78: at 0.62 the user found taps a little faint (2026-11-01). */
    Tap(HapticFeedbackType.ContextClick, VibrationEffect.EFFECT_CLICK, HapticFeedbackConstants.VIRTUAL_KEY, listOf(Note(PRIMITIVE_CLICK, 0.78f))),
    /** A key on a keyboard-like surface: lighter, for rapid repeats. */
    Key(HapticFeedbackType.KeyboardTap, VibrationEffect.EFFECT_TICK, HapticFeedbackConstants.KEYBOARD_TAP, listOf(Note(PRIMITIVE_TICK, 0.9f))),
    /** Snaps in: a small rise and a firm click. */
    ToggleOn(HapticFeedbackType.ToggleOn, VibrationEffect.EFFECT_CLICK, HapticFeedbackConstants.VIRTUAL_KEY, listOf(Note(PRIMITIVE_TICK, 0.45f), Note(PRIMITIVE_CLICK, 0.85f, 34))),
    /** Lets go: a click falling away into a low tick. */
    ToggleOff(HapticFeedbackType.ToggleOff, VibrationEffect.EFFECT_TICK, HapticFeedbackConstants.KEYBOARD_TAP, listOf(Note(PRIMITIVE_CLICK, 0.72f), Note(PRIMITIVE_LOW_TICK, 0.5f, 38))),
    /** Moving onto a new option: tabs, segmented choices. 1.0: at 0.7 the tab bar's ticks were too faint (2026-11-02). */
    Select(HapticFeedbackType.SegmentTick, VibrationEffect.EFFECT_TICK, HapticFeedbackConstants.CLOCK_TICK, listOf(Note(PRIMITIVE_TICK, 1f))),
    /** A detent while dragging a value: volume steps, scroll notches. */
    Detent(HapticFeedbackType.SegmentFrequentTick, VibrationEffect.EFFECT_TICK, HapticFeedbackConstants.CLOCK_TICK, listOf(Note(PRIMITIVE_LOW_TICK, 0.75f))),
    /** Hitting an end stop or crossing a commit threshold: one firm click. */
    Edge(HapticFeedbackType.GestureThresholdActivate, VibrationEffect.EFFECT_HEAVY_CLICK, HapticFeedbackConstants.VIRTUAL_KEY, listOf(Note(PRIMITIVE_CLICK, 1f))),
    /** A hold turned into a grab: a swell that lands. */
    Hold(HapticFeedbackType.LongPress, VibrationEffect.EFFECT_HEAVY_CLICK, HapticFeedbackConstants.LONG_PRESS, listOf(Note(PRIMITIVE_QUICK_RISE, 0.5f), Note(PRIMITIVE_CLICK, 0.9f, 10))),
    /** Glass forming or bursting (the header's title bubble): barely there. */
    Bubble(HapticFeedbackType.SegmentFrequentTick, VibrationEffect.EFFECT_TICK, HapticFeedbackConstants.TEXT_HANDLE_MOVE, listOf(Note(PRIMITIVE_LOW_TICK, 0.38f))),
    /**
     * One grain of a swipe texture (a drop slid along the tab bar): the faintest tick, fired every
     * few dp of travel so a drag feels like a fine, continuous swipe rather than separate taps.
     */
    Grain(HapticFeedbackType.SegmentFrequentTick, VibrationEffect.EFFECT_TICK, HapticFeedbackConstants.TEXT_HANDLE_MOVE, listOf(Note(PRIMITIVE_LOW_TICK, 0.22f))),
    /** Two rising clicks. */
    Success(HapticFeedbackType.Confirm, VibrationEffect.EFFECT_DOUBLE_CLICK, HapticFeedbackConstants.CONFIRM, listOf(Note(PRIMITIVE_CLICK, 0.66f), Note(PRIMITIVE_CLICK, 1f, 70))),
    /** A heavy thud (this motor's lasts 300 ms, so one is plenty) and a dull tick after it. */
    Failure(HapticFeedbackType.Reject, VibrationEffect.EFFECT_HEAVY_CLICK, HapticFeedbackConstants.REJECT, listOf(Note(PRIMITIVE_THUD, 0.8f), Note(PRIMITIVE_LOW_TICK, 0.7f, 60))),
}

/** Primitives that can stand in for [primitive], closest first. */
internal fun nearest(primitive: Int): List<Int> = when (primitive) {
    // Closest first. A rise is only a lead-in, so it's dropped rather than replaced.
    PRIMITIVE_LOW_TICK -> listOf(PRIMITIVE_LOW_TICK, PRIMITIVE_TICK, PRIMITIVE_CLICK)
    PRIMITIVE_TICK -> listOf(PRIMITIVE_TICK, PRIMITIVE_LOW_TICK, PRIMITIVE_CLICK)
    PRIMITIVE_THUD -> listOf(PRIMITIVE_THUD, PRIMITIVE_CLICK)
    else -> listOf(primitive)
}

/** How this phone plays haptics; see [Haptic]. [label] names it in Settings. */
enum class HapticEngine(val label: String) {
    /** The maker's own touch feedback, through the view: what their keyboard and system UI use. */
    SYSTEM("System"),
    PRIMITIVES("Crisp"),
    TUNED("Classic"),
    PULSES("Short pulses"),
    PLAIN_PULSES("Plain pulses"),
}

/**
 * OPPO's ColorOS family (OnePlus, OPPO, realme) claims the click and tick primitives but plays
 * them through its own motor driver as a buzz (a Nord CE 3, 2026-11: "only buzzes" with
 * primitives, tuned click and tick all reported). Its own view feedback is the crisp one there.
 */
private val SYSTEM_FIRST = setOf("oneplus", "oppo", "realme")

internal fun engineFor(v: Vibrator, system: Boolean = true): HapticEngine = when {
    system && Build.MANUFACTURER.lowercase() in SYSTEM_FIRST -> HapticEngine.SYSTEM
    // Clicks and ticks are what everything is made of; a motor with only some primitives (or
    // only rises) can't play the set the same way.
    v.areAllPrimitivesSupported(PRIMITIVE_CLICK, PRIMITIVE_TICK) -> HapticEngine.PRIMITIVES
    // UNKNOWN is what most phones say: then the effect may be Android's long stand-in buzz.
    v.areAllEffectsSupported(VibrationEffect.EFFECT_CLICK, VibrationEffect.EFFECT_TICK, VibrationEffect.EFFECT_HEAVY_CLICK, VibrationEffect.EFFECT_DOUBLE_CLICK) ==
        Vibrator.VIBRATION_EFFECT_SUPPORT_YES -> HapticEngine.TUNED
    v.hasAmplitudeControl() -> HapticEngine.PULSES
    else -> HapticEngine.PLAIN_PULSES
}

/** [h] the way [engine] plays it. */
internal fun compose(v: Vibrator, h: Haptic, engine: HapticEngine): VibrationEffect = when (engine) {
    HapticEngine.PRIMITIVES -> {
        val notes = h.notes.mapNotNull { n ->
            nearest(n.primitive).firstOrNull { v.areAllPrimitivesSupported(it) }?.let { Note(it, n.scale, n.delayMs) }
        }
        VibrationEffect.startComposition().apply { notes.forEach { addPrimitive(it.primitive, it.scale, it.delayMs) } }.compose()
    }
    HapticEngine.TUNED -> VibrationEffect.createPredefined(h.predefined)
    HapticEngine.PULSES, HapticEngine.PLAIN_PULSES -> pulses(v, h, amplitude = engine == HapticEngine.PULSES)
    HapticEngine.SYSTEM -> error("played by the view")
}

/**
 * [h] as a waveform of short pulses: a click is about two cycles of the motor's resonance (~10 ms
 * at 170-235 Hz), a tick about one and a half; anything longer stops being a click and becomes a
 * buzz. Without amplitude control a lighter note is a shorter pulse instead of a softer one.
 */
internal fun pulses(v: Vibrator, h: Haptic, amplitude: Boolean): VibrationEffect {
    val hz = resonance(v).takeIf { it.isFinite() && it in 80f..400f } ?: 175f
    val cycle = 1000f / hz
    val timings = mutableListOf<Long>()
    val amps = mutableListOf<Int>()
    for (n in h.notes) {
        val cycles = when (n.primitive) {
            PRIMITIVE_QUICK_RISE -> continue // a lead-in; a pulse can't swell
            PRIMITIVE_THUD -> 3f
            PRIMITIVE_CLICK -> 2f
            PRIMITIVE_LOW_TICK -> 1.5f
            else -> 1.5f // TICK
        }
        val strength = (n.scale * if (n.primitive == PRIMITIVE_LOW_TICK) 0.8f else 1f).coerceIn(0.2f, 1f)
        val on = if (amplitude) cycles * cycle else cycles * cycle * (0.6f + 0.4f * strength)
        timings += n.delayMs.toLong().coerceAtLeast(if (timings.isEmpty()) 0 else 12)
        amps += 0
        timings += on.roundToInt().coerceIn(4, 18).toLong()
        amps += if (amplitude) (90 + 165 * strength).roundToInt().coerceIn(1, 255) else 255
    }
    if (timings.isEmpty()) return VibrationEffect.createOneShot(8, VibrationEffect.DEFAULT_AMPLITUDE)
    return if (amplitude) VibrationEffect.createWaveform(timings.toLongArray(), amps.toIntArray(), -1)
    else VibrationEffect.createWaveform(timings.toLongArray(), -1)
}

/**
 * The motor's resonance, NaN when unknown. Android 14 API: reading it on 13 threw on every tap
 * (a moto g52, 2026-11: "crash when pressing on anything").
 */
internal fun resonance(v: Vibrator): Float = if (Build.VERSION.SDK_INT >= 34) v.resonantFrequency else Float.NaN

/** One primitive in a [Haptic], [delayMs] after the previous one ends. */
internal class Note(val primitive: Int, val scale: Float, val delayMs: Int = 0)

/** Plays [Haptic]s on the vibrator directly; [VyomaTheme] attaches it. */
object Haptics {
    private var vibrator: Vibrator? = null
    private var resolver: ContentResolver? = null
    private val playable = HashMap<Haptic, VibrationEffect?>()
    private val touch by lazy {
        if (Build.VERSION.SDK_INT >= 33) VibrationAttributes.createForUsage(VibrationAttributes.USAGE_TOUCH) else null
    }

    /** How this phone plays them (diagnostics show it); null without a vibrator. */
    var engine: HapticEngine? = null
        private set

    /** The window's view, for [HapticEngine.SYSTEM]; [VyomaTheme] sets it. */
    var view: java.lang.ref.WeakReference<View>? = null

    /** What this phone would pick by itself. */
    var auto: HapticEngine? = null
        private set

    /** The user's choice from Settings, or null for [auto]. */
    var chosen: HapticEngine? = null
        set(value) {
            field = value
            engine = if (vibrator == null) null else value ?: auto
            playable.clear()
        }

    fun attach(context: Context) {
        if (vibrator != null) return
        vibrator = context.getSystemService(VibratorManager::class.java)?.defaultVibrator?.takeIf { it.hasVibrator() }
        resolver = context.applicationContext.contentResolver
        auto = vibrator?.let(::engineFor)
        chosen = chosen
    }

    /** The styles this phone can play, for the Settings picker. */
    fun choices(): List<HapticEngine> {
        val v = vibrator ?: return emptyList()
        return HapticEngine.entries.filter {
            when (it) {
                HapticEngine.PRIMITIVES -> v.areAllPrimitivesSupported(PRIMITIVE_CLICK, PRIMITIVE_TICK)
                HapticEngine.PULSES -> v.hasAmplitudeControl()
                else -> true
            }
        }
    }

    /** What the motor reports, for a diagnostics log from a phone we can't hold. */
    fun describe(): String {
        val v = vibrator ?: return "no vibrator"
        fun yes(r: Int) = when (r) { Vibrator.VIBRATION_EFFECT_SUPPORT_YES -> "yes"; Vibrator.VIBRATION_EFFECT_SUPPORT_NO -> "no"; else -> "unknown" }
        val prims = listOf("click" to PRIMITIVE_CLICK, "tick" to PRIMITIVE_TICK, "lowTick" to PRIMITIVE_LOW_TICK, "thud" to PRIMITIVE_THUD, "rise" to PRIMITIVE_QUICK_RISE)
            .joinToString { (n, p) -> "$n=${v.areAllPrimitivesSupported(p)}" }
        val effects = listOf("click" to VibrationEffect.EFFECT_CLICK, "tick" to VibrationEffect.EFFECT_TICK, "heavy" to VibrationEffect.EFFECT_HEAVY_CLICK)
            .joinToString { (n, e) -> "$n=${yes(v.areEffectsSupported(e)[0])}" }
        return "engine=$engine (auto $auto); maker=${Build.MANUFACTURER}; primitives: $prims; tuned effects: $effects; amplitude control=${v.hasAmplitudeControl()}; resonance=${resonance(v)} Hz"
    }

    /** True when it played (or the user turned touch feedback off); false: use the fallback. */
    internal fun play(h: Haptic): Boolean {
        val v = vibrator ?: return false
        var engine = engine ?: return false
        if (engine == HapticEngine.SYSTEM) {
            // The view honours the touch feedback setting itself. Detached (no window): the next best.
            if (view?.get()?.performHapticFeedback(h.system) == true) return true
            engine = engineFor(v, system = false)
        }
        val effect = playable.getOrPut(h) { compose(v, h, engine) } ?: return false
        // Android 13+ applies the touch feedback setting to USAGE_TOUCH itself; before that, read it.
        val attrs = touch
        if (attrs != null && Build.VERSION.SDK_INT >= 33) v.vibrate(effect, attrs)
        else if (resolver?.let { Settings.System.getInt(it, Settings.System.HAPTIC_FEEDBACK_ENABLED, 1) } != 0) v.vibrate(effect)
        return true
    }
}

/** How far every edge moves in on a press (a held tile's menu starts from there too). */
internal const val PRESS_TRAVEL_DP = 3f

/** [pressable]/[pressScale]'s default: dip by the same distance at any size. */
const val PressAuto = Float.NaN

fun HapticFeedback.perform(h: Haptic) {
    if (!Haptics.play(h)) performHapticFeedback(h.type)
}

/**
 * Clickable that squishes on press and bounces back on release, with a [haptic] on click: the
 * One UI / iOS feel. See [pressScale].
 */
fun Modifier.pressable(
    enabled: Boolean = true,
    /** [PressAuto]: dips by the same distance whatever the size (see [pressScale]); 1f: doesn't dip. */
    pressedScale: Float = PressAuto,
    role: Role? = Role.Button,
    haptic: Haptic? = Haptic.Tap,
    /** Capsules and circles only: glass that lights up softly under the finger as it dips. */
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
 * scrolling parent takes the gesture over, or the finger slides off, it flows back with a softer
 * wobble; sliding back on dips it again.
 *
 * By default ([PressAuto]) everything dips by the same few dp at its edges, so a 48 dp button and a
 * full-width row feel like one material (iOS's presses work this way): hand-picked scales per
 * element (0.84 to 0.985) made the same touch feel different from one control to the next.
 *
 * A modifier node that only touches its own graphics layer: presses never recompose or relayout.
 */
fun Modifier.pressScale(pressedScale: Float = PressAuto, enabled: Boolean = true, glass: Boolean = false): Modifier =
    this then PressScaleElement(pressedScale, enabled, glass, glow = glass || !(pressedScale >= 1f))

private data class PressScaleElement(val pressedScale: Float, val enabled: Boolean, val glass: Boolean, val glow: Boolean) : ModifierNodeElement<PressScaleNode>() {
    override fun create() = PressScaleNode(pressedScale, enabled, glass, glow)
    override fun update(node: PressScaleNode) {
        node.pressedScale = pressedScale
        node.enabled = enabled
        node.glass = glass
        node.glow = glow
    }
}

/**
 * [glass]: a capsule or circle that dips even when asked not to scale (the chrome handles it).
 * [glow]: light gathers under the finger: everything that dips does (tiles, rows and cards too,
 * not only pills), so every surface answers a touch the same way.
 */
private class PressScaleNode(var pressedScale: Float, var enabled: Boolean, var glass: Boolean, var glow: Boolean) :
    Modifier.Node(), PointerInputModifierNode, LayoutModifierNode, DrawModifierNode, CompositionLocalConsumerModifierNode {
    private val sx = Animatable(1f)
    private val sy = Animatable(1f)
    /** How lit the glass is under the finger: up while held, eases out after. Only with [glass]. */
    private val clear = Animatable(0f)
    private var held = false
    /** A finger that landed here and still owns the gesture (no scroll took it), held or slid off. */
    private var tracking = false
    private var job: Job? = null
    private var glassJob: Job? = null
    /** Where the finger is, for the glow. */
    private var touch = Offset.Unspecified
    /** The element's size at the last touch, for [PressAuto]. */
    private var bounds = IntSize.Zero

    /** The scale a press dips to: the caller's, or the one that moves the edges [PRESS_TRAVEL_DP] in. */
    private fun pressed(): Float {
        val s = if (!pressedScale.isNaN()) pressedScale else {
            val longest = maxOf(bounds.width, bounds.height).toFloat()
            if (longest <= 0f) 0.94f else (1f - 2f * PRESS_TRAVEL_DP * requireDensity().density / longest).coerceIn(0.86f, 0.985f)
        }
        return if (glass && s >= 1f) GLASS_PRESSED else s
    }

    override fun ContentDrawScope.draw() {
        drawContent()
        val a = if (glow) clear.value.coerceIn(0f, 1f) else 0f
        if (a < 0.01f) return
        val dark = currentValueOf(LocalContentColor).luminance() > 0.5f
        // Light gathers under the finger and follows it, inside the capsule. No refraction: a lens
        // over a button's own label only ever looked like a glitch.
        val at = if (touch.isSpecified) touch else center
        // Round for capsules and circles; a rounded rectangle (tile-like) for anything larger.
        val r = minOf(size.minDimension / 2, 20.dp.toPx())
        clipPath(Path().apply { addRoundRect(RoundRect(0f, 0f, size.width, size.height, CornerRadius(r))) }) {
            drawCircle(
                // White light can't show on light glass: there the finger leaves a faint shade instead.
                Brush.radialGradient(listOf((if (dark) Color.White.copy(alpha = 0.16f * a) else Color(0xFF1A2B4C).copy(alpha = 0.08f * a)), Color.Transparent), at, size.maxDimension * 0.7f),
                radius = size.maxDimension * 0.7f, center = at,
            )
        }
    }


    private fun clearTo(target: Float) {
        if (!glow) return
        glassJob?.cancel()
        glassJob = coroutineScope.launch { clear.animateTo(target, if (target > 0f) VyomaMotion.spatial() else VyomaMotion.effects()) }
    }

    override fun onPointerEvent(pointerEvent: PointerEvent, pass: PointerEventPass, bounds: IntSize) {
        if (!enabled) return
        this.bounds = bounds
        if (glow && pass == PointerEventPass.Initial) {
            pointerEvent.changes.firstOrNull { it.pressed }?.let {
                touch = it.position
                if (held) invalidateDraw()
            }
        }
        when (pass) {
            PointerEventPass.Initial ->
                if (!held && pointerEvent.changes.any { it.changedToDownIgnoreConsumed() }) {
                    tracking = true
                    press()
                }
            PointerEventPass.Final -> if (tracking) {
                // Moves consumed by someone else mean a scroll or drag owns this gesture now.
                val stolen = pointerEvent.changes.any { it.pressed && it.isConsumed && it.positionChangedIgnoreConsumed() }
                val finger = pointerEvent.changes.firstOrNull { it.pressed }
                when {
                    stolen -> { tracking = false; if (held) release(bounce = false) }
                    finger == null -> { tracking = false; if (held) release(bounce = true) }
                    // Slid off (on purpose, to not tap): it lets go, and dips again if the finger
                    // comes back, like iOS. A margin keeps a wobbly thumb on the edge from flickering.
                    else -> {
                        val m = SLIDE_MARGIN_DP * requireDensity().density
                        val p = finger.position
                        val inside = p.x >= -m && p.y >= -m && p.x <= bounds.width + m && p.y <= bounds.height + m
                        if (held && !inside) release(bounce = false)
                        else if (!held && inside) press()
                    }
                }
            }
            else -> Unit
        }
    }

    override fun onCancelPointerInput() {
        tracking = false
        if (held) release(bounce = false)
    }

    private fun press() {
        held = true
        clearTo(1f)
        job?.cancel()
        // Height gives a beat later than width: a squash, not a shrink.
        val target = pressed()
        job = coroutineScope.launch {
            launch { sy.animateTo(target, PRESS_SOFT) }
            sx.animateTo(target, PRESS)
        }
    }

    private fun release(bounce: Boolean) {
        held = false
        clearTo(0f)
        job?.cancel()
        job = coroutineScope.launch {
            if (bounce) {
                val pressed = pressed()
                val depth = 1f - pressed
                val dip = pressed + depth * 0.25f
                if (sx.value > pressed + depth * 0.4f) {
                    launch { sy.animateTo(dip, DIP) }
                    sx.animateTo(dip, DIP)
                }
                // Thrown back out: overshoots past full size, then wobbles in (width and height out of step).
                val kick = depth * 34f
                launch { sy.animateTo(1f, VyomaMotion.jelly(), initialVelocity = kick * 0.8f) }
                sx.animateTo(1f, VyomaMotion.release(), initialVelocity = kick)
            } else {
                // Let go without a tap (a scroll took it, or the finger slid off): it still flows
                // home like water, a softer wobble from wherever the dip had got to. A flat ease
                // here read as the press stalling halfway.
                val kick = (1f - minOf(sx.value, sy.value)) * 14f
                launch { sy.animateTo(1f, VyomaMotion.jelly(), initialVelocity = kick * 0.8f) }
                sx.animateTo(1f, VyomaMotion.release(), initialVelocity = kick)
            }
        }
    }

    override fun MeasureScope.measure(measurable: Measurable, constraints: Constraints): MeasureResult {
        val placeable = measurable.measure(constraints)
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
        /** Glass buttons whose caller asked for no scale (the parent handles the click) still dip. */
        const val GLASS_PRESSED = 0.9f
        /** How far past its edge a finger can stray before the press lets go. */
        const val SLIDE_MARGIN_DP = 12f
    }
}
