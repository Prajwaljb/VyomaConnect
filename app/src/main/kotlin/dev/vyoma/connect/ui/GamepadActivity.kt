package dev.vyoma.connect.ui

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.VibrationEffect
import android.os.VibratorManager
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.FilterNone
import androidx.compose.material.icons.rounded.Menu
import androidx.compose.material.icons.rounded.SportsEsports
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.input.pointer.PointerId
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import dev.vyoma.connect.container
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.designsystem.VyomaTheme
import dev.vyoma.connect.designsystem.aurora
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.GamepadButton
import dev.vyoma.connect.protocol.plugin.builtin.GamepadState
import dev.vyoma.connect.protocol.plugin.builtin.Rumble
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.roundToInt

/**
 * The phone as a game controller for the PC: the companion makes a virtual Xbox 360 pad there (so
 * Steam and games take it as the real thing) and passes the game's rumble back to the phone's
 * vibrator. Landscape, full screen, screen kept on. Sends only while visible: leaving (or the
 * screen going off) closes the pad, and the PC lets go of every button.
 */
class GamepadActivity : ComponentActivity() {
    private var plugin: CompanionPlugin? = null
    private var pcName: String? = null
    private val state = MutableStateFlow(GamepadState())
    private val vibrator by lazy { (getSystemService(VIBRATOR_MANAGER_SERVICE) as VibratorManager).defaultVibrator }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
        // The PC asked for, else whichever PC has its companion connected.
        val wanted = intent.getStringExtra(EXTRA_PC)
        val devices = application.container.hub.value?.devices?.value?.values.orEmpty()
        val companion = devices.firstOrNull { d ->
            d.state.value.isReachable && d.identity.companionOf != null && (wanted == null || d.identity.companionOf == wanted)
        }
        plugin = companion?.plugin(CompanionPlugin.ID)
        pcName = companion?.identity?.companionOf?.let { pc -> devices.firstOrNull { it.identity.deviceId == pc }?.identity?.deviceName }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                val p = plugin ?: return@repeatOnLifecycle
                launch { p.rumble.collect(::rumble) }
                p.streamGamepad(state)
            }
        }
        setContent {
            val theme by application.container.settings.theme.collectAsStateWithLifecycle(ThemeSettings())
            VyomaTheme(theme.copy(darkMode = DarkMode.DARK)) {
                val error = plugin?.gamepadError?.collectAsStateWithLifecycle()?.value
                val status = when {
                    plugin == null -> "Needs Vyoma's companion running on the PC"
                    error != null -> error
                    else -> pcName?.let { "Xbox controller on $it" } ?: "Xbox controller on the PC"
                }
                GamepadScreen(enabled = plugin != null, status = status, failed = plugin == null || error != null, onState = { state.value = it }, onClose = ::finish)
            }
        }
    }

    override fun onStop() {
        super.onStop()
        // The fingers are gone with the screen: nothing stays held.
        state.value = GamepadState()
        vibrator.cancel()
    }

    /** The game's rumble on the phone: the stronger motor sets the strength (the weak one feels half as strong). */
    private fun rumble(r: Rumble) {
        val level = maxOf(r.strong, r.weak / 2)
        if (level <= 0) return vibrator.cancel()
        val amplitude = if (vibrator.hasAmplitudeControl()) (level * 255L / 65535).toInt().coerceIn(1, 255) else VibrationEffect.DEFAULT_AMPLITUDE
        // "Until the next one" ends at the next rumble, which every game sends to stop it.
        val effect = VibrationEffect.createOneShot(if (r.ms > 0) r.ms.toLong() else 5_000L, amplitude)
        // As media, not touch feedback: turning off touch vibration mustn't silence the game.
        if (android.os.Build.VERSION.SDK_INT >= 33) vibrator.vibrate(effect, android.os.VibrationAttributes.createForUsage(android.os.VibrationAttributes.USAGE_MEDIA))
        else @Suppress("DEPRECATION") vibrator.vibrate(effect, android.media.AudioAttributes.Builder().setUsage(android.media.AudioAttributes.USAGE_GAME).build())
    }

    companion object {
        private const val EXTRA_PC = "pc"

        /** [pcId] null: whichever PC has the companion connected. */
        fun open(context: Context, pcId: String?) =
            context.startActivity(Intent(context, GamepadActivity::class.java).putExtra(EXTRA_PC, pcId))
    }
}

/** Where everything sits, for a landscape screen of [w] x [h] px. */
private class PadLayout(val w: Float, val h: Float, d: Density) {
    val u = with(d) { 1.dp.toPx() }
    private val edge = 20 * u
    private val shoulderSize = Size(104 * u, 42 * u)
    val lt = Rect(Offset(edge, edge), shoulderSize)
    val lb = Rect(Offset(lt.right + 10 * u, edge), shoulderSize)
    val rt = Rect(Offset(w - edge - shoulderSize.width, edge), shoulderSize)
    val rb = Rect(Offset(rt.left - 10 * u - shoulderSize.width, edge), shoulderSize)
    val close = Offset(w / 2, edge + 21 * u)
    val closeR = 18 * u
    private val midY = h * 0.34f
    private val smallSize = Size(58 * u, 34 * u)
    val back = Rect(Offset(w / 2 - 78 * u - smallSize.width / 2, midY - smallSize.height / 2), smallSize)
    val start = Rect(Offset(w / 2 + 78 * u - smallSize.width / 2, midY - smallSize.height / 2), smallSize)
    val guide = Offset(w / 2, midY)
    val guideR = 24 * u
    val stickR = 62 * u
    val knobR = 30 * u
    val leftHome = Offset(edge + 92 * u, h * 0.56f)
    val rightHome = Offset(w * 0.68f, h - edge - 58 * u)
    val dpad = Offset(w * 0.32f, h - edge - 58 * u)
    val dpadR = 56 * u
    val faceR = 28 * u
    private val faceGap = 50 * u
    val faceCentre = Offset(w - edge - faceGap - faceR, h * 0.56f)
    /** A (bottom), B (right), X (left), Y (top), as on an Xbox pad. */
    val face = listOf(
        GamepadButton.A to faceCentre + Offset(0f, faceGap),
        GamepadButton.B to faceCentre + Offset(faceGap, 0f),
        GamepadButton.X to faceCentre + Offset(-faceGap, 0f),
        GamepadButton.Y to faceCentre + Offset(0f, -faceGap),
    )

    /** A thumb resting between two face buttons presses both (as on a real pad); slid off, the nearest. */
    fun faceBits(p: Offset): Int {
        var bits = 0
        var nearest: GamepadButton? = null
        var best = Float.MAX_VALUE
        for ((b, c) in face) {
            val d = (p - c).getDistance()
            if (d < faceR * 1.2f) bits = bits or b.bit
            if (d < best) { best = d; nearest = b }
        }
        if (bits == 0 && best < faceR * 1.9f) bits = nearest!!.bit
        return bits
    }

    /** Eight ways, by which way the thumb leans from the middle. */
    fun dpadBits(p: Offset): Int {
        val v = p - dpad
        val len = v.getDistance()
        if (len < dpadR * 0.2f) return 0
        var bits = 0
        if (v.x > 0.38f * len) bits = bits or GamepadButton.RIGHT.bit
        if (v.x < -0.38f * len) bits = bits or GamepadButton.LEFT.bit
        if (v.y > 0.38f * len) bits = bits or GamepadButton.DOWN.bit
        if (v.y < -0.38f * len) bits = bits or GamepadButton.UP.bit
        return bits
    }
}

/** What one finger is holding. */
private sealed interface Grip {
    class Button(val bits: Int) : Grip
    object Face : Grip
    object Dpad : Grip
    class Stick(val left: Boolean, var origin: Offset, val downAt: Long, var knob: Offset, var travel: Float = 0f) : Grip
    object Close : Grip
}

/** Internal bits for the triggers, beyond the pad's own buttons. */
private const val LT_BIT = 1 shl 20
private const val RT_BIT = 1 shl 21

@Composable
private fun GamepadScreen(enabled: Boolean, status: String, failed: Boolean, onState: (GamepadState) -> Unit, onClose: () -> Unit) {
    val haptics = LocalHapticFeedback.current
    val density = LocalDensity.current
    val scope = rememberCoroutineScope()
    val measurer = rememberTextMeasurer()
    val green = SystemColor.GREEN.color
    val red = SystemColor.RED.color
    val blue = SystemColor.BLUE.color
    val yellow = SystemColor.YELLOW.color
    val accent = MaterialTheme.colorScheme.primary
    val guideIcon = rememberVectorPainter(Icons.Rounded.SportsEsports)
    val viewIcon = rememberVectorPainter(Icons.Rounded.FilterNone)
    val menuIcon = rememberVectorPainter(Icons.Rounded.Menu)
    val closeIcon = rememberVectorPainter(Icons.Rounded.Close)
    val letter = TextStyle(fontSize = 20.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
    val small = TextStyle(fontSize = 14.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
    val labels = remember(measurer) {
        mapOf(
            "A" to measurer.measure("A", letter), "B" to measurer.measure("B", letter),
            "X" to measurer.measure("X", letter), "Y" to measurer.measure("Y", letter),
            "LT" to measurer.measure("LT", small), "LB" to measurer.measure("LB", small),
            "RB" to measurer.measure("RB", small), "RT" to measurer.measure("RT", small),
        )
    }
    // What's drawn: read only in the draw phase, so a move redraws without recomposing.
    // The sticks move in place, so a counter tells the canvas something changed.
    var frame by remember { mutableIntStateOf(0) }
    var held by remember { mutableIntStateOf(0) }
    var sticks by remember { mutableStateOf<Pair<Grip.Stick?, Grip.Stick?>>(null to null) }

    Box(Modifier.fillMaxSize().aurora()) {
        BoxWithConstraints(Modifier.fillMaxSize().safeDrawingPadding()) {
            val w = with(density) { maxWidth.toPx() }
            val h = with(density) { maxHeight.toPx() }
            val layout = remember(w, h, density) { PadLayout(w, h, density) }
            Canvas(
                Modifier.fillMaxSize().pointerInput(layout, enabled) {
                    val grips = HashMap<PointerId, Pair<Grip, Int>>()
                    var pulses = 0
                    var sent = GamepadState()
                    fun publish() {
                        var bits = pulses
                        var left: Grip.Stick? = null
                        var right: Grip.Stick? = null
                        for ((g, b) in grips.values) {
                            bits = bits or b
                            if (g is Grip.Stick) if (g.left) left = g else right = g
                        }
                        fun axis(s: Grip.Stick?): Pair<Int, Int> {
                            s ?: return 0 to 0
                            val v = (s.knob - s.origin) / layout.stickR
                            return (v.x * 32767).roundToInt().coerceIn(-32768, 32767) to (v.y * 32767).roundToInt().coerceIn(-32768, 32767)
                        }
                        val (lx, ly) = axis(left)
                        val (rx, ry) = axis(right)
                        val next = GamepadState(
                            buttons = bits and (LT_BIT or RT_BIT).inv(),
                            lx = lx, ly = ly, rx = rx, ry = ry,
                            lt = if (bits and LT_BIT != 0) 255 else 0,
                            rt = if (bits and RT_BIT != 0) 255 else 0,
                        )
                        // A light key tick for each new press (sticks and releases stay silent).
                        if (bits and held.inv() and (LT_BIT or RT_BIT or 0xFFFF) != 0) haptics.perform(Haptic.Key)
                        held = bits
                        sticks = left to right
                        frame++
                        if (next != sent) {
                            sent = next
                            if (enabled) onState(next)
                        }
                    }
                    fun grab(p: Offset, at: Long): Pair<Grip, Int> = with(layout) {
                        fun inCapsule(r: Rect) = r.inflate(6 * u).contains(p)
                        when {
                            (p - close).getDistance() < closeR * 1.5f -> Grip.Close to 0
                            inCapsule(lt) -> Grip.Button(LT_BIT).let { it to it.bits }
                            inCapsule(lb) -> Grip.Button(GamepadButton.LB.bit).let { it to it.bits }
                            inCapsule(rb) -> Grip.Button(GamepadButton.RB.bit).let { it to it.bits }
                            inCapsule(rt) -> Grip.Button(RT_BIT).let { it to it.bits }
                            inCapsule(back) -> Grip.Button(GamepadButton.BACK.bit).let { it to it.bits }
                            inCapsule(start) -> Grip.Button(GamepadButton.START.bit).let { it to it.bits }
                            (p - guide).getDistance() < guideR * 1.3f -> Grip.Button(GamepadButton.GUIDE.bit).let { it to it.bits }
                            faceBits(p) != 0 -> Grip.Face to faceBits(p)
                            (p - dpad).getDistance() < dpadR * 1.25f -> Grip.Dpad to dpadBits(p)
                            else -> {
                                // A stick centres where the thumb lands, kept whole on its half of the screen.
                                val left = p.x < w / 2
                                val o = Offset(
                                    if (left) p.x.coerceIn(stickR, w / 2 - stickR) else p.x.coerceIn(w / 2 + stickR, w - stickR),
                                    p.y.coerceIn(lt.bottom + stickR, h - stickR),
                                )
                                Grip.Stick(left, o, at, p) to 0
                            }
                        }
                    }
                    awaitPointerEventScope {
                        while (true) {
                            val event = awaitPointerEvent()
                            for (c in event.changes) {
                                val p = c.position
                                when {
                                    c.pressed && !c.previousPressed -> grips[c.id] = grab(p, c.uptimeMillis)
                                    c.pressed -> grips[c.id]?.let { (g, _) ->
                                        grips[c.id] = g to when (g) {
                                            is Grip.Face -> layout.faceBits(p)
                                            is Grip.Dpad -> layout.dpadBits(p)
                                            is Grip.Stick -> {
                                                g.travel += (c.position - c.previousPosition).getDistance()
                                                // Past the rim the base follows the thumb, so turning back answers at once.
                                                val v = p - g.origin
                                                val len = v.getDistance()
                                                if (len > layout.stickR) g.origin = p - v / len * layout.stickR
                                                g.knob = p
                                                0
                                            }
                                            is Grip.Button, Grip.Close -> grips[c.id]!!.second
                                        }
                                    }
                                    !c.pressed && c.previousPressed -> grips.remove(c.id)?.let { (g, _) ->
                                        when (g) {
                                            is Grip.Close -> if ((p - layout.close).getDistance() < layout.closeR * 2f) onClose()
                                            // A quick tap on a stick clicks it (L3 / R3).
                                            is Grip.Stick -> if (c.uptimeMillis - g.downAt < 220 && g.travel < 12 * layout.u) {
                                                val bit = if (g.left) GamepadButton.LS.bit else GamepadButton.RS.bit
                                                pulses = pulses or bit
                                                scope.launch {
                                                    delay(80)
                                                    pulses = pulses and bit.inv()
                                                    publish()
                                                }
                                            }
                                            else -> Unit
                                        }
                                    }
                                }
                                c.consume()
                            }
                            publish()
                        }
                    }
                },
            ) {
                frame // redraws on every input
                val l = layout
                val bits = held
                val dim = if (enabled) 1f else 0.4f
                fun on(b: Int) = bits and b != 0
                capsule(l.lt, on(LT_BIT), accent, labels["LT"], dim)
                capsule(l.lb, on(GamepadButton.LB.bit), accent, labels["LB"], dim)
                capsule(l.rb, on(GamepadButton.RB.bit), accent, labels["RB"], dim)
                capsule(l.rt, on(RT_BIT), accent, labels["RT"], dim)
                capsule(l.back, on(GamepadButton.BACK.bit), accent, null, dim)
                icon(viewIcon, l.back.center, 18 * l.u, dim)
                capsule(l.start, on(GamepadButton.START.bit), accent, null, dim)
                icon(menuIcon, l.start.center, 18 * l.u, dim)
                disc(l.guide, l.guideR, on(GamepadButton.GUIDE.bit), green, null, dim)
                icon(guideIcon, l.guide, 24 * l.u, dim)
                disc(l.close, l.closeR, false, Color.White, null, 0.7f)
                icon(closeIcon, l.close, 18 * l.u, 0.7f)
                val colors = mapOf(GamepadButton.A to green, GamepadButton.B to red, GamepadButton.X to blue, GamepadButton.Y to yellow)
                for ((b, c) in l.face) disc(c, l.faceR, on(b.bit), colors.getValue(b), labels[b.name], dim)
                dpad(l, bits, dim)
                val (left, right) = sticks
                stick(left?.origin ?: l.leftHome, left?.knob ?: l.leftHome, l, left != null, on(GamepadButton.LS.bit), accent, dim)
                stick(right?.origin ?: l.rightHome, right?.knob ?: l.rightHome, l, right != null, on(GamepadButton.RS.bit), accent, dim)
            }
            Text(
                status,
                style = MaterialTheme.typography.labelMedium,
                color = if (failed) SystemColor.ORANGE.color else MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.align(Alignment.TopCenter).offset(y = with(density) { (layout.guide.y + layout.guideR).toDp() } + 14.dp).padding(horizontal = 120.dp),
            )
        }
    }
}

/** A frosted glass disc; held, it fills with its colour and glows. */
private fun DrawScope.disc(c: Offset, r: Float, pressed: Boolean, tint: Color, label: TextLayoutResult?, alpha: Float) {
    if (pressed) drawCircle(Brush.radialGradient(listOf(tint.copy(alpha = 0.4f), Color.Transparent), c, r * 1.9f), r * 1.9f, c)
    drawCircle(if (pressed) tint.copy(alpha = 0.7f) else Color.White.copy(alpha = 0.09f * alpha), r, c)
    drawCircle(
        Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.4f * alpha), Color.White.copy(alpha = 0.08f * alpha)), c.y - r, c.y + r),
        r - 0.5.dp.toPx(), c, style = Stroke(1.dp.toPx()),
    )
    label?.let { drawText(it, if (pressed) Color.White else tint.copy(alpha = alpha), c - Offset(it.size.width / 2f, it.size.height / 2f)) }
}

private fun DrawScope.capsule(r: Rect, pressed: Boolean, tint: Color, label: TextLayoutResult?, alpha: Float) {
    val corner = CornerRadius(r.height / 2)
    if (pressed) drawRoundRect(tint.copy(alpha = 0.35f), r.topLeft - Offset(6.dp.toPx(), 6.dp.toPx()), Size(r.width + 12.dp.toPx(), r.height + 12.dp.toPx()), CornerRadius(r.height / 2 + 6.dp.toPx()))
    drawRoundRect(if (pressed) tint.copy(alpha = 0.75f) else Color.White.copy(alpha = 0.09f * alpha), r.topLeft, r.size, corner)
    drawRoundRect(
        Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.4f * alpha), Color.White.copy(alpha = 0.08f * alpha)), r.top, r.bottom),
        r.topLeft + Offset(0.5.dp.toPx(), 0.5.dp.toPx()), Size(r.width - 1.dp.toPx(), r.height - 1.dp.toPx()), corner, style = Stroke(1.dp.toPx()),
    )
    label?.let { drawText(it, Color.White.copy(alpha = 0.9f * alpha), r.center - Offset(it.size.width / 2f, it.size.height / 2f)) }
}

private fun DrawScope.icon(painter: androidx.compose.ui.graphics.painter.Painter, c: Offset, size: Float, alpha: Float) {
    translate(c.x - size / 2, c.y - size / 2) {
        with(painter) { draw(Size(size, size), alpha = 0.9f * alpha, colorFilter = ColorFilter.tint(Color.White)) }
    }
}

/** A cross of two frosted arms; the held directions light up. */
private fun DrawScope.dpad(l: PadLayout, bits: Int, alpha: Float) {
    val c = l.dpad
    val r = l.dpadR
    val t = r * 0.7f
    val corner = CornerRadius(t * 0.28f)
    val fill = Color.White.copy(alpha = 0.09f * alpha)
    drawRoundRect(fill, Offset(c.x - r, c.y - t / 2), Size(2 * r, t), corner)
    drawRoundRect(fill, Offset(c.x - t / 2, c.y - r), Size(t, 2 * r), corner)
    val rim = Color.White.copy(alpha = 0.22f * alpha)
    drawRoundRect(rim, Offset(c.x - r, c.y - t / 2), Size(2 * r, t), corner, style = Stroke(1.dp.toPx()))
    drawRoundRect(rim, Offset(c.x - t / 2, c.y - r), Size(t, 2 * r), corner, style = Stroke(1.dp.toPx()))
    val lit = Color.White.copy(alpha = 0.5f)
    val arm = r - t / 2
    fun light(on: Boolean, topLeft: Offset, size: Size) { if (on) drawRoundRect(lit, topLeft, size, corner) }
    light(bits and GamepadButton.UP.bit != 0, Offset(c.x - t / 2, c.y - r), Size(t, arm))
    light(bits and GamepadButton.DOWN.bit != 0, Offset(c.x - t / 2, c.y + t / 2), Size(t, arm))
    light(bits and GamepadButton.LEFT.bit != 0, Offset(c.x - r, c.y - t / 2), Size(arm, t))
    light(bits and GamepadButton.RIGHT.bit != 0, Offset(c.x + t / 2, c.y - t / 2), Size(arm, t))
    // Small arrowheads in each arm.
    val tip = t * 0.18f
    val arrow = Color.White.copy(alpha = 0.55f * alpha)
    for ((dx, dy) in listOf(0f to -1f, 0f to 1f, -1f to 0f, 1f to 0f)) {
        val p = c + Offset(dx, dy) * (r - t * 0.45f)
        val path = androidx.compose.ui.graphics.Path().apply {
            moveTo(p.x + dx * tip, p.y + dy * tip)
            lineTo(p.x - dx * tip + dy * tip, p.y - dy * tip + dx * tip)
            lineTo(p.x - dx * tip - dy * tip, p.y - dy * tip - dx * tip)
            close()
        }
        drawPath(path, arrow)
    }
}

/** A stick: its base where the thumb landed (resting at home, faint), the knob under the thumb. */
private fun DrawScope.stick(origin: Offset, knob: Offset, l: PadLayout, active: Boolean, clicked: Boolean, tint: Color, alpha: Float) {
    val a = if (active) 1f else 0.6f
    drawCircle(Color.White.copy(alpha = 0.05f * alpha * a), l.stickR, origin)
    drawCircle(Color.White.copy(alpha = 0.18f * alpha * a), l.stickR - 0.5.dp.toPx(), origin, style = Stroke(1.dp.toPx()))
    val v = knob - origin
    val len = v.getDistance()
    val k = if (len > l.stickR) origin + v / len * l.stickR else knob
    if (active) drawCircle(Brush.radialGradient(listOf(tint.copy(alpha = 0.3f), Color.Transparent), k, l.knobR * 2f), l.knobR * 2f, k)
    drawCircle(if (clicked) tint.copy(alpha = 0.8f) else Color.White.copy(alpha = (if (active) 0.28f else 0.16f) * alpha), l.knobR, k)
    drawCircle(
        Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.5f * alpha), Color.White.copy(alpha = 0.1f * alpha)), k.y - l.knobR, k.y + l.knobR),
        l.knobR - 0.5.dp.toPx(), k, style = Stroke(1.dp.toPx()),
    )
}
