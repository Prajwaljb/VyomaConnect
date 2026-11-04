package dev.vyoma.connect.ui

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.VibrationEffect
import android.os.VibratorManager
import android.view.Surface
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.FilterNone
import androidx.compose.material.icons.rounded.Menu
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.SportsEsports
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.painter.Painter
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
import dev.vyoma.connect.designsystem.DialogButton
import dev.vyoma.connect.designsystem.GlassDialog
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.LiquidSlider
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.SegmentedControl
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.designsystem.VyomaSwitch
import dev.vyoma.connect.designsystem.VyomaTheme
import dev.vyoma.connect.designsystem.aurora
import dev.vyoma.connect.designsystem.glassCard
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.gamepad.GyroAim
import dev.vyoma.connect.gamepad.GyroMode
import dev.vyoma.connect.gamepad.GyroTarget
import dev.vyoma.connect.gamepad.PadControl
import dev.vyoma.connect.gamepad.PadProfile
import dev.vyoma.connect.gamepad.PadProfiles
import dev.vyoma.connect.gamepad.Place
import dev.vyoma.connect.gamepad.UsbAccessoryLink
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.GamepadButton
import dev.vyoma.connect.protocol.plugin.builtin.GamepadRoute
import dev.vyoma.connect.protocol.plugin.builtin.GamepadState
import dev.vyoma.connect.protocol.plugin.builtin.PadKind
import dev.vyoma.connect.protocol.plugin.builtin.Rumble
import kotlinx.coroutines.delay
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/**
 * The phone as a game controller for the PC: the companion makes a virtual Xbox 360 pad or DualSense
 * there (so Steam and games take it as the real thing) and passes the game's rumble back to the
 * phone's vibrator. The gyro can aim (a mouse or the right stick). Each game can have its own
 * profile: the pad, gyro, triggers and the layout, moved and resized in place. Landscape, full
 * screen, screen kept on. Sends only while visible: leaving (or the screen going off) closes the pad,
 * and the PC lets go of every button.
 */
class GamepadActivity : ComponentActivity() {
    /** The connected companion's plugin and its PC's name; a new link (the companion restarting) brings a new one. */
    private val companion = MutableStateFlow<Pair<CompanionPlugin, String?>?>(null)
    /** What the fingers hold; [state] adds the gyro and the profile's pad. */
    private var touch = GamepadState()
    private val state = MutableStateFlow(GamepadState())
    /** The settings or the layout editor is open: nothing is sent as held. */
    private var paused = false
    private val profiles by lazy { PadProfiles(applicationContext) }
    private val gyro by lazy {
        GyroAim(this, rotation = { display?.rotation ?: Surface.ROTATION_90 }, onMove = ::recompute)
    }
    /** The USB cable without USB debugging, for the plugin (accessory mode, see UsbAccessoryLink). */
    private val cable = object : CompanionPlugin.CableChannel {
        override fun plugged() = UsbAccessoryLink.cablePlugged(applicationContext)
        override fun open() = UsbAccessoryLink.output(applicationContext)
        override fun close() = UsbAccessoryLink.close()
    }
    private val vibrator by lazy { (getSystemService(VIBRATOR_MANAGER_SERVICE) as VibratorManager).defaultVibrator }

    /**
     * Touches the moment the screen senses them, not batched to the next frame: up to a frame
     * (4-8 ms) sooner for the controller, as games do.
     */
    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        window.decorView.requestUnbufferedDispatch(android.view.InputDevice.SOURCE_TOUCHSCREEN)
    }

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
        lifecycleScope.launch {
            @OptIn(ExperimentalCoroutinesApi::class)
            application.container.hub.filterNotNull().flatMapLatest { hub ->
                hub.devices.flatMapLatest { all ->
                    val each = all.values.map { d ->
                        d.state.map { st ->
                            val pc = d.identity.companionOf
                            if (!st.isReachable || pc == null || (wanted != null && pc != wanted)) return@map null
                            d.plugin<CompanionPlugin>(CompanionPlugin.ID)?.let { it to all[pc]?.identity?.deviceName }
                        }
                    }
                    if (each.isEmpty()) flowOf(null) else combine(each) { found -> found.firstOrNull { it != null } }
                }
            }.distinctUntilChanged { a, b -> a?.first === b?.first && a?.second == b?.second }.collect { companion.value = it }
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                // The gyro runs only while the profile uses it and the controller is on screen.
                launch {
                    try {
                        profiles.currentId.collect {
                            applyProfile()
                            if (profiles.current().gyro != GyroMode.OFF) gyro.start() else gyro.stop()
                        }
                    } finally {
                        gyro.stop()
                    }
                }
                launch { profiles.all.collect { applyProfile() } }
                // Streams to whichever companion link is up, moving over when it's replaced.
                companion.collectLatest { c ->
                    val p = c?.first ?: return@collectLatest
                    p.cable = cable
                    coroutineScope {
                        launch { p.rumble.collect(::rumble) }
                        p.streamGamepad(state)
                    }
                }
            }
        }
        setContent {
            val theme by application.container.settings.theme.collectAsStateWithLifecycle(ThemeSettings())
            VyomaTheme(theme.copy(darkMode = DarkMode.DARK)) {
                val linked by companion.collectAsStateWithLifecycle()
                val plugin = linked?.first
                val pcName = linked?.second
                val error = plugin?.gamepadError?.collectAsStateWithLifecycle()?.value
                val route = plugin?.gamepadRoute?.collectAsStateWithLifecycle()?.value
                val all by profiles.all.collectAsStateWithLifecycle()
                val currentId by profiles.currentId.collectAsStateWithLifecycle()
                val profile = all.firstOrNull { it.id == currentId } ?: all.first()
                val padName = if (profile.kind == PadKind.DUALSENSE) "DualSense" else "Xbox controller"
                val status = when {
                    plugin == null -> "Needs Vyoma's companion running on the PC"
                    error != null -> error
                    else -> "$padName on ${pcName ?: "the PC"}" + (when (route) {
                        GamepadRoute.USB -> " · cable"
                        // Wi-Fi can be fine or stutter: say so, and what's smoother.
                        GamepadRoute.WIFI -> " · Wi-Fi (a cable is smoother)"
                        null -> ""
                    }) +
                        (if (all.size > 1) " · ${profile.name}" else "")
                }
                GamepadScreen(
                    enabled = plugin != null,
                    status = status,
                    failed = plugin == null || error != null,
                    profile = profile,
                    profiles = all,
                    gyroAvailable = gyro.available,
                    onTouch = { synchronized(this@GamepadActivity) { touch = it }; recompute() },
                    onPaused = { synchronized(this@GamepadActivity) { paused = it }; recompute() },
                    onSave = profiles::save,
                    onSelect = profiles::select,
                    onCreate = { profiles.create(it) },
                    onDelete = profiles::delete,
                    onClose = ::finish,
                )
            }
        }
    }

    private fun applyProfile() {
        val p = profiles.current()
        gyro.speed = p.gyroSpeed
        gyro.invertY = p.invertGyroY
        if (p.gyro != GyroMode.OFF) {
            if (lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) gyro.start()
        } else gyro.stop()
        recompute()
    }

    /**
     * Wi-Fi power saving holds outgoing packets back and sends them in bursts; this lock (made for
     * games) keeps the radio awake while the controller is on screen.
     */
    private val wifiLock by lazy {
        (applicationContext.getSystemService(WIFI_SERVICE) as android.net.wifi.WifiManager)
            .createWifiLock(android.net.wifi.WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "vyoma:controller")
            .apply { setReferenceCounted(false) }
    }

    override fun onStart() {
        super.onStart()
        runCatching { wifiLock.acquire() }
    }

    /** The fingers, plus the gyro (when it's aiming), as the profile's pad. Called from the main and gyro threads. */
    @Synchronized
    private fun recompute() {
        val p = profiles.current()
        val t = if (paused) GamepadState() else touch
        gyro.active = !paused && when (p.gyro) {
            GyroMode.OFF -> false
            GyroMode.ALWAYS -> true
            GyroMode.AIMING -> t.lt > 0
        }
        var rx = t.rx
        var ry = t.ry
        if (gyro.active && p.gyroTarget == GyroTarget.STICK) {
            rx = (rx + gyro.stickX * 32767).roundToInt().coerceIn(-32768, 32767)
            ry = (ry + gyro.stickY * 32767).roundToInt().coerceIn(-32768, 32767)
        }
        state.value = t.copy(kind = p.kind, rx = rx, ry = ry, mx = gyro.mouseX, my = gyro.mouseY)
    }

    override fun onStop() {
        super.onStop()
        runCatching { wifiLock.release() }
        // The fingers are gone with the screen: nothing stays held.
        touch = GamepadState()
        recompute()
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

/**
 * Where everything sits, for a landscape screen of [w] x [h] px: each control's usual spot, unless
 * the profile moved it ([places], fractions of the screen) or resized it.
 */
private class PadLayout(
    val w: Float, val h: Float, d: Density, private val places: Map<PadControl, Place>, val analog: Boolean,
    /** A DualSense's shape: D-pad up top on the left, both sticks low and level (an Xbox pad swaps them). */
    private val ps: Boolean,
) {
    val u = with(d) { 1.dp.toPx() }
    private val edge = 20 * u

    fun usual(c: PadControl): Offset = when (c) {
        PadControl.LT -> Offset(edge + 52 * u, edge + 21 * u)
        PadControl.LB -> Offset(edge + 166 * u, edge + 21 * u)
        PadControl.RB -> Offset(w - edge - 166 * u, edge + 21 * u)
        PadControl.RT -> Offset(w - edge - 52 * u, edge + 21 * u)
        PadControl.VIEW -> Offset(w / 2 - 78 * u, h * 0.34f)
        PadControl.MENU -> Offset(w / 2 + 78 * u, h * 0.34f)
        PadControl.GUIDE -> Offset(w / 2, h * 0.34f)
        PadControl.FACE -> Offset(w - edge - 78 * u, h * 0.56f)
        PadControl.DPAD -> if (ps) Offset(edge + 92 * u, h * 0.56f) else Offset(w * 0.32f, h - edge - 58 * u)
        PadControl.LSTICK -> if (ps) Offset(w * 0.32f, h - edge - 58 * u) else Offset(edge + 92 * u, h * 0.56f)
        PadControl.RSTICK -> Offset(w * 0.68f, h - edge - 58 * u)
    }

    fun centre(c: PadControl): Offset = places[c]?.let { Offset(it.x * w, it.y * h) } ?: usual(c)
    fun scale(c: PadControl): Float = places[c]?.scale ?: 1f

    private fun capsuleAt(c: PadControl, width: Float, height: Float): Rect {
        val s = scale(c)
        return Rect(centre(c) - Offset(width * s / 2, height * s / 2), Size(width * s, height * s))
    }

    /** Analog triggers are slots that run on down below the capsule: deeper down, harder pressed. */
    private fun triggerAt(c: PadControl): Rect {
        val r = capsuleAt(c, 104 * u, 42 * u)
        return if (analog) Rect(r.left, r.top, r.right, r.bottom + 54 * u * scale(c)) else r
    }

    val lt = triggerAt(PadControl.LT)
    val rt = triggerAt(PadControl.RT)
    val lb = capsuleAt(PadControl.LB, 104 * u, 42 * u)
    val rb = capsuleAt(PadControl.RB, 104 * u, 42 * u)
    val back = capsuleAt(PadControl.VIEW, 58 * u, 34 * u)
    val start = capsuleAt(PadControl.MENU, 58 * u, 34 * u)
    val guide = centre(PadControl.GUIDE)
    val guideR = 24 * u * scale(PadControl.GUIDE)
    // Close and settings stay put between the shoulders; they can't be moved off screen.
    val close = Offset(w / 2 - 26 * u, edge + 21 * u)
    val settings = Offset(w / 2 + 26 * u, edge + 21 * u)
    val roundR = 18 * u
    val leftR = 62 * u * scale(PadControl.LSTICK)
    val rightR = 62 * u * scale(PadControl.RSTICK)
    val leftKnob = 30 * u * scale(PadControl.LSTICK)
    val rightKnob = 30 * u * scale(PadControl.RSTICK)
    val leftHome = centre(PadControl.LSTICK)
    val rightHome = centre(PadControl.RSTICK)
    val dpad = centre(PadControl.DPAD)
    val dpadR = 56 * u * scale(PadControl.DPAD)
    val faceR = 28 * u * scale(PadControl.FACE)
    private val faceGap = 50 * u * scale(PadControl.FACE)
    val faceCentre = centre(PadControl.FACE)
    /** A (bottom), B (right), X (left), Y (top), as on an Xbox pad (cross, circle, square, triangle). */
    val face = listOf(
        GamepadButton.A to faceCentre + Offset(0f, faceGap),
        GamepadButton.B to faceCentre + Offset(faceGap, 0f),
        GamepadButton.X to faceCentre + Offset(-faceGap, 0f),
        GamepadButton.Y to faceCentre + Offset(0f, -faceGap),
    )

    /** What the editor grabs and outlines. */
    fun bounds(c: PadControl): Rect = when (c) {
        PadControl.LT -> lt
        PadControl.RT -> rt
        PadControl.LB -> lb
        PadControl.RB -> rb
        PadControl.VIEW -> back
        PadControl.MENU -> start
        PadControl.GUIDE -> Rect(guide, guideR)
        PadControl.FACE -> Rect(faceCentre, faceGap + faceR)
        PadControl.DPAD -> Rect(dpad, dpadR)
        PadControl.LSTICK -> Rect(leftHome, leftR)
        PadControl.RSTICK -> Rect(rightHome, rightR)
    }

    /** The control under [p] for the editor: the smallest one holding it, else the nearest close by. */
    fun controlAt(p: Offset): PadControl? {
        val pad = 10 * u
        PadControl.entries.filter { bounds(it).inflate(pad).contains(p) }.minByOrNull { bounds(it).width * bounds(it).height }?.let { return it }
        return PadControl.entries.minByOrNull { (bounds(it).center - p).getDistance() }?.takeIf { (bounds(it).center - p).getDistance() < 80 * u }
    }

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

    /** How hard a thumb at [p] presses the trigger [r]: all the way at a touch, or by depth in an analog slot. */
    fun triggerValue(r: Rect, p: Offset): Int {
        if (!analog) return 255
        val depth = ((p.y - r.top) / (r.height * 0.85f)).coerceIn(0.1f, 1f)
        return (depth * 255).roundToInt()
    }
}

/** What one finger is holding. */
private sealed interface Grip {
    class Button(val bits: Int) : Grip
    class Trigger(val left: Boolean, var value: Int) : Grip
    object Face : Grip
    object Dpad : Grip
    class Stick(val left: Boolean, var origin: Offset, val downAt: Long, var knob: Offset, var travel: Float = 0f) : Grip
    object Close : Grip
    object Settings : Grip
}

@Composable
private fun GamepadScreen(
    enabled: Boolean,
    status: String,
    failed: Boolean,
    profile: PadProfile,
    profiles: List<PadProfile>,
    gyroAvailable: Boolean,
    onTouch: (GamepadState) -> Unit,
    onPaused: (Boolean) -> Unit,
    onSave: (PadProfile) -> Unit,
    onSelect: (String) -> Unit,
    onCreate: (String) -> Unit,
    onDelete: (String) -> Unit,
    onClose: () -> Unit,
) {
    val haptics = LocalHapticFeedback.current
    val density = LocalDensity.current
    val scope = rememberCoroutineScope()
    val measurer = rememberTextMeasurer()
    val accent = MaterialTheme.colorScheme.primary
    val ds = profile.kind == PadKind.DUALSENSE
    val faceColors = mapOf(
        GamepadButton.A to (if (ds) SystemColor.BLUE else SystemColor.GREEN).color,
        GamepadButton.B to SystemColor.RED.color,
        GamepadButton.X to (if (ds) SystemColor.PINK else SystemColor.BLUE).color,
        GamepadButton.Y to (if (ds) SystemColor.GREEN else SystemColor.YELLOW).color,
    )
    val guideColor = if (ds) SystemColor.BLUE.color else SystemColor.GREEN.color
    val guideIcon = rememberVectorPainter(Icons.Rounded.SportsEsports)
    val viewIcon = rememberVectorPainter(Icons.Rounded.FilterNone)
    val menuIcon = rememberVectorPainter(Icons.Rounded.Menu)
    val closeIcon = rememberVectorPainter(Icons.Rounded.Close)
    val settingsIcon = rememberVectorPainter(Icons.Rounded.Settings)
    val letter = TextStyle(fontSize = 20.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
    val small = TextStyle(fontSize = 14.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
    val labels = remember(measurer) {
        listOf("A", "B", "X", "Y").associateWith { measurer.measure(it, letter) } +
            listOf("LT", "LB", "RB", "RT", "L2", "L1", "R1", "R2").associateWith { measurer.measure(it, small) }
    }
    val shoulder = if (ds) listOf("L2", "L1", "R1", "R2") else listOf("LT", "LB", "RB", "RT")
    // What's drawn: read only in the draw phase, so a move redraws without recomposing.
    // The sticks move in place, so a counter tells the canvas something changed.
    var frame by remember { mutableIntStateOf(0) }
    var held by remember { mutableIntStateOf(0) }
    var triggers by remember { mutableStateOf(0 to 0) }
    var sticks by remember { mutableStateOf<Pair<Grip.Stick?, Grip.Stick?>>(null to null) }

    var showSettings by remember { mutableStateOf(false) }
    var editing by remember { mutableStateOf(false) }
    val edits = remember { mutableStateMapOf<PadControl, Place>() }
    var chosen by remember { mutableStateOf<PadControl?>(null) }
    fun pause(on: Boolean) {
        held = 0
        triggers = 0 to 0
        sticks = null to null
        onTouch(GamepadState())
        onPaused(on)
    }
    fun closeEditor(save: Boolean) {
        if (save) onSave(profile.copy(places = edits.toMap()))
        editing = false
        chosen = null
        pause(false)
    }
    BackHandler(editing) { closeEditor(save = false) }
    BackHandler(showSettings && !editing) { showSettings = false; pause(false) }

    Box(Modifier.fillMaxSize().aurora()) {
        BoxWithConstraints(Modifier.fillMaxSize().safeDrawingPadding()) {
            val w = with(density) { maxWidth.toPx() }
            val h = with(density) { maxHeight.toPx() }
            val places: Map<PadControl, Place> = if (editing) edits.toMap() else profile.places
            val layout = remember(w, h, density, places, profile.analogTriggers, profile.kind) {
                PadLayout(w, h, density, places, profile.analogTriggers, profile.kind == PadKind.DUALSENSE)
            }
            val input = if (editing) {
                Modifier.pointerInput(w, h, density, profile.analogTriggers) {
                    // Drag a control to move it; a second finger pinches it bigger or smaller.
                    awaitPointerEventScope {
                        var pinchFrom = 0f
                        var scaleFrom = 1f
                        while (true) {
                            val event = awaitPointerEvent()
                            val down = event.changes.filter { it.pressed }
                            val now = PadLayout(w, h, density, edits.toMap(), profile.analogTriggers, profile.kind == PadKind.DUALSENSE)
                            val first = event.changes.firstOrNull { it.pressed && !it.previousPressed }
                            if (first != null && down.size == 1) {
                                chosen = now.controlAt(first.position)
                                if (chosen != null) haptics.perform(Haptic.Key)
                            }
                            val c = chosen
                            if (c != null && down.size >= 2) {
                                val span = (down[0].position - down[1].position).getDistance()
                                if (pinchFrom == 0f || first != null) { pinchFrom = span; scaleFrom = now.scale(c) }
                                val s = (scaleFrom * span / pinchFrom).coerceIn(MIN_SCALE, MAX_SCALE)
                                val at = now.centre(c)
                                edits[c] = Place(at.x / w, at.y / h, s)
                            } else if (c != null && down.size == 1) {
                                pinchFrom = 0f
                                val ch = down[0]
                                val at = now.centre(c) + (ch.position - ch.previousPosition)
                                edits[c] = Place((at.x / w).coerceIn(0.03f, 0.97f), (at.y / h).coerceIn(0.04f, 0.96f), now.scale(c))
                            }
                            if (down.isEmpty()) pinchFrom = 0f
                            event.changes.forEach { it.consume() }
                            frame++
                        }
                    }
                }
            } else {
                Modifier.pointerInput(layout, enabled) {
                    val grips = HashMap<PointerId, Pair<Grip, Int>>()
                    var pulses = 0
                    var sent = GamepadState()
                    fun publish() {
                        var bits = pulses
                        var left: Grip.Stick? = null
                        var right: Grip.Stick? = null
                        var lt = 0
                        var rt = 0
                        for ((g, b) in grips.values) {
                            bits = bits or b
                            if (g is Grip.Stick) if (g.left) left = g else right = g
                            if (g is Grip.Trigger) if (g.left) lt = maxOf(lt, g.value) else rt = maxOf(rt, g.value)
                        }
                        fun axis(s: Grip.Stick?): Pair<Int, Int> {
                            s ?: return 0 to 0
                            val v = (s.knob - s.origin) / (if (s.left) layout.leftR else layout.rightR)
                            return (v.x * 32767).roundToInt().coerceIn(-32768, 32767) to (v.y * 32767).roundToInt().coerceIn(-32768, 32767)
                        }
                        val (lx, ly) = axis(left)
                        val (rx, ry) = axis(right)
                        val next = GamepadState(buttons = bits, lx = lx, ly = ly, rx = rx, ry = ry, lt = lt, rt = rt)
                        // A light key tick for each new press (sticks and releases stay silent).
                        val pressing = bits or (if (lt > 0) LT_BIT else 0) or (if (rt > 0) RT_BIT else 0)
                        if (pressing and held.inv() and (LT_BIT or RT_BIT or 0xFFFF) != 0) haptics.perform(Haptic.Key)
                        held = pressing
                        triggers = lt to rt
                        sticks = left to right
                        frame++
                        if (next != sent) {
                            sent = next
                            if (enabled) onTouch(next)
                        }
                    }
                    fun grab(p: Offset, at: Long): Pair<Grip, Int> = with(layout) {
                        fun inCapsule(r: Rect) = r.inflate(6 * u).contains(p)
                        when {
                            (p - close).getDistance() < roundR * 1.4f -> Grip.Close to 0
                            (p - settings).getDistance() < roundR * 1.4f -> Grip.Settings to 0
                            inCapsule(lt) -> Grip.Trigger(true, triggerValue(lt, p)) to 0
                            inCapsule(rt) -> Grip.Trigger(false, triggerValue(rt, p)) to 0
                            inCapsule(lb) -> Grip.Button(GamepadButton.LB.bit).let { it to it.bits }
                            inCapsule(rb) -> Grip.Button(GamepadButton.RB.bit).let { it to it.bits }
                            inCapsule(back) -> Grip.Button(GamepadButton.BACK.bit).let { it to it.bits }
                            inCapsule(start) -> Grip.Button(GamepadButton.START.bit).let { it to it.bits }
                            (p - guide).getDistance() < guideR * 1.3f -> Grip.Button(GamepadButton.GUIDE.bit).let { it to it.bits }
                            faceBits(p) != 0 -> Grip.Face to faceBits(p)
                            (p - dpad).getDistance() < dpadR * 1.25f -> Grip.Dpad to dpadBits(p)
                            else -> {
                                // A stick centres where the thumb lands, kept whole on its half of the screen.
                                val left = p.x < w / 2
                                val r = if (left) leftR else rightR
                                val o = Offset(
                                    if (left) p.x.coerceIn(r, w / 2 - r) else p.x.coerceIn(w / 2 + r, w - r),
                                    p.y.coerceIn(minOf(lt.bottom, rt.bottom) + r, h - r),
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
                                            is Grip.Trigger -> {
                                                g.value = layout.triggerValue(if (g.left) layout.lt else layout.rt, p)
                                                0
                                            }
                                            is Grip.Stick -> {
                                                g.travel += (c.position - c.previousPosition).getDistance()
                                                // Past the rim the base follows the thumb, so turning back answers at once.
                                                val r = if (g.left) layout.leftR else layout.rightR
                                                val v = p - g.origin
                                                val len = v.getDistance()
                                                if (len > r) g.origin = p - v / len * r
                                                g.knob = p
                                                0
                                            }
                                            is Grip.Button, Grip.Close, Grip.Settings -> grips[c.id]!!.second
                                        }
                                    }
                                    !c.pressed && c.previousPressed -> grips.remove(c.id)?.let { (g, _) ->
                                        when (g) {
                                            is Grip.Close -> if ((p - layout.close).getDistance() < layout.roundR * 2f) onClose()
                                            is Grip.Settings -> if ((p - layout.settings).getDistance() < layout.roundR * 2f) {
                                                grips.clear()
                                                showSettings = true
                                                pause(true)
                                            }
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
                            if (!showSettings) publish()
                        }
                    }
                }
            }
            Canvas(Modifier.fillMaxSize().then(input)) {
                frame // redraws on every input
                val l = layout
                val bits = held
                val dim = if (enabled || editing) 1f else 0.4f
                fun on(b: Int) = bits and b != 0
                val (lt, rt) = triggers
                trigger(l.lt, lt, l.analog, accent, labels[shoulder[0]], dim)
                capsule(l.lb, on(GamepadButton.LB.bit), accent, labels[shoulder[1]], dim)
                capsule(l.rb, on(GamepadButton.RB.bit), accent, labels[shoulder[2]], dim)
                trigger(l.rt, rt, l.analog, accent, labels[shoulder[3]], dim)
                capsule(l.back, on(GamepadButton.BACK.bit), accent, null, dim)
                icon(viewIcon, l.back.center, 18 * l.u * l.scale(PadControl.VIEW), dim)
                capsule(l.start, on(GamepadButton.START.bit), accent, null, dim)
                icon(menuIcon, l.start.center, 18 * l.u * l.scale(PadControl.MENU), dim)
                disc(l.guide, l.guideR, on(GamepadButton.GUIDE.bit), guideColor, null, dim)
                icon(guideIcon, l.guide, l.guideR, dim)
                if (!editing) {
                    disc(l.close, l.roundR, false, Color.White, null, 0.7f)
                    icon(closeIcon, l.close, 18 * l.u, 0.7f)
                    disc(l.settings, l.roundR, false, Color.White, null, 0.7f)
                    icon(settingsIcon, l.settings, 18 * l.u, 0.7f)
                }
                for ((b, c) in l.face) {
                    val tint = faceColors.getValue(b)
                    disc(c, l.faceR, on(b.bit), tint, if (ds) null else labels[b.name], dim)
                    if (ds) psGlyph(b, c, l.faceR * 0.42f, if (on(b.bit)) Color.White else tint.copy(alpha = dim))
                }
                dpad(l, bits, dim)
                val (left, right) = sticks
                stick(left?.origin ?: l.leftHome, left?.knob ?: l.leftHome, l.leftR, l.leftKnob, left != null, on(GamepadButton.LS.bit), accent, dim)
                stick(right?.origin ?: l.rightHome, right?.knob ?: l.rightHome, l.rightR, l.rightKnob, right != null, on(GamepadButton.RS.bit), accent, dim)
                if (editing) {
                    // Every movable part outlined; the chosen one solid.
                    for (c in PadControl.entries) {
                        val r = l.bounds(c).inflate(6 * l.u)
                        val picked = c == chosen
                        drawRoundRect(
                            if (picked) accent else Color.White.copy(alpha = 0.35f), r.topLeft, r.size, CornerRadius(14 * l.u),
                            style = Stroke(if (picked) 2.5f * l.u else 1.2f * l.u, pathEffect = if (picked) null else PathEffect.dashPathEffect(floatArrayOf(8 * l.u, 6 * l.u))),
                        )
                    }
                }
            }
            val belowGuide = with(density) { (layout.guide.y + layout.guideR).toDp() } + 14.dp
            if (editing) {
                EditBar(
                    chosen = chosen,
                    scale = chosen?.let { layout.scale(it) },
                    onScale = { s -> chosen?.let { c -> val at = layout.centre(c); edits[c] = Place(at.x / w, at.y / h, s) } },
                    onReset = { edits.clear(); chosen = null },
                    onCancel = { closeEditor(save = false) },
                    onDone = { closeEditor(save = true) },
                    // Away from what's being moved: low while it's up top (or nothing is chosen), else high.
                    up = chosen?.let { layout.centre(it).y > h / 2 } == true,
                    modifier = Modifier.align(if (chosen?.let { layout.centre(it).y > h / 2 } == true) Alignment.TopCenter else Alignment.BottomCenter)
                        .padding(vertical = 8.dp),
                )
            } else {
                Text(
                    status,
                    style = MaterialTheme.typography.labelMedium,
                    color = if (failed) SystemColor.ORANGE.color else MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.align(Alignment.TopCenter).offset(y = belowGuide).padding(horizontal = 120.dp),
                )
            }
            if (showSettings && !editing) {
                SettingsPanel(
                    profile = profile,
                    profiles = profiles,
                    gyroAvailable = gyroAvailable,
                    onSave = onSave,
                    onSelect = onSelect,
                    onCreate = onCreate,
                    onDelete = onDelete,
                    onEditLayout = {
                        edits.clear()
                        edits.putAll(profile.places)
                        chosen = null
                        showSettings = false
                        editing = true
                    },
                    onDismiss = { showSettings = false; pause(false) },
                )
            }
        }
    }
}

private const val MIN_SCALE = 0.6f
private const val MAX_SCALE = 1.6f

/** Internal bits for the triggers, beyond the pad's own buttons (for the key tick and the glow). */
private const val LT_BIT = 1 shl 20
private const val RT_BIT = 1 shl 21

/** The editor's bar: the chosen part's size, Reset, Cancel and Done. */
@Composable
private fun EditBar(
    chosen: PadControl?,
    scale: Float?,
    onScale: (Float) -> Unit,
    onReset: () -> Unit,
    onCancel: () -> Unit,
    onDone: () -> Unit,
    up: Boolean,
    modifier: Modifier = Modifier,
) {
    val hint: @Composable () -> Unit = {
        Text(
            chosen?.label?.let { "$it · drag to move, pinch to resize" } ?: "Touch a control to move it",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(vertical = 6.dp),
        )
    }
    Column(modifier.widthIn(max = 460.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        if (!up) hint()
        Row(
            Modifier.glassCard(RoundedCornerShape(24.dp), fill = Color(0xFF1C1D21).copy(alpha = 0.94f)).padding(horizontal = 6.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            BarButton("Reset", onReset)
            BarButton("−", onClick = { scale?.let { onScale((it - 0.1f).coerceAtLeast(MIN_SCALE)) } }, enabled = scale != null)
            Text(
                scale?.let { "${(it * 100).roundToInt()}%" } ?: "Size",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurface.copy(alpha = if (scale != null) 1f else 0.5f),
                textAlign = TextAlign.Center,
                maxLines = 1,
                softWrap = false,
                modifier = Modifier.widthIn(min = 52.dp),
            )
            BarButton("+", onClick = { scale?.let { onScale((it + 0.1f).coerceAtMost(MAX_SCALE)) } }, enabled = scale != null)
            BarButton("Cancel", onCancel)
            BarButton("Done", onDone, prominent = true)
        }
        if (up) hint()
    }
}

@Composable
private fun BarButton(text: String, onClick: () -> Unit, enabled: Boolean = true, prominent: Boolean = false) {
    val accent = MaterialTheme.colorScheme.primary
    Box(
        Modifier
            .clip(RoundedCornerShape(18.dp))
            .background(if (prominent) accent else Color.White.copy(alpha = 0.08f))
            .clickable(enabled = enabled, interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text,
            style = MaterialTheme.typography.labelLarge,
            color = (if (prominent) Color.White else MaterialTheme.colorScheme.onSurface).copy(alpha = if (enabled) 1f else 0.4f),
        )
    }
}

/** The controller's settings: which profile (one per game), the pad the PC makes, triggers, gyro, layout. */
@Composable
private fun SettingsPanel(
    profile: PadProfile,
    profiles: List<PadProfile>,
    gyroAvailable: Boolean,
    onSave: (PadProfile) -> Unit,
    onSelect: (String) -> Unit,
    onCreate: (String) -> Unit,
    onDelete: (String) -> Unit,
    onEditLayout: () -> Unit,
    onDismiss: () -> Unit,
) {
    var naming by remember { mutableStateOf<String?>(null) }
    var renaming by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }
    var speed by remember(profile.id) { mutableFloatStateOf(profile.gyroSpeed) }
    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black.copy(alpha = 0.6f))
            .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
        contentAlignment = Alignment.Center,
    ) {
        BoxWithConstraints(Modifier.fillMaxSize().padding(vertical = 12.dp), contentAlignment = Alignment.Center) {
            Column(
                Modifier
                    // Two columns side by side: landscape is wide and short.
                    .widthIn(max = 820.dp)
                    .fillMaxWidth(0.9f)
                    .heightIn(max = maxHeight)
                    // Nearly solid: the controls under it would read through clear glass.
                    .glassCard(RoundedCornerShape(28.dp), fill = Color(0xFF1C1D21).copy(alpha = 0.94f))
                    // Taps inside the card don't close it.
                    .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {}
                    .verticalScroll(rememberScrollState())
                    .padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Controller", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.weight(1f))
                    BarButton("Done", onDismiss, prominent = true)
                }
                Row(horizontalArrangement = Arrangement.spacedBy(24.dp)) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                Section("Profile") {
                    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        for (p in profiles) Chip(p.name, selected = p.id == profile.id) { onSelect(p.id) }
                        Chip("+ New game", selected = false) { naming = "" }
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        BarButton("Rename", { renaming = true; naming = profile.name })
                        if (profiles.size > 1) BarButton("Delete", { deleting = true })
                    }
                }
                Section("The PC sees") {
                    SegmentedControl(listOf("Xbox", "DualSense"), if (profile.kind == PadKind.DUALSENSE) 1 else 0, { i ->
                        onSave(profile.copy(kind = if (i == 1) PadKind.DUALSENSE else PadKind.XBOX))
                    })
                }
                Section("Triggers") {
                    SegmentedControl(listOf("Full press", "Analog"), if (profile.analogTriggers) 1 else 0, { i ->
                        onSave(profile.copy(analogTriggers = i == 1))
                    })
                    Text(
                        if (profile.analogTriggers) "Slide down a trigger to press it deeper." else "A touch presses a trigger all the way.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                PillButton("Edit layout", onEditLayout, modifier = Modifier.fillMaxWidth())
                Text(
                    "Tip: for the least lag, plug the phone into the PC with a USB cable. Over Wi-Fi the controller can stutter when the network is busy.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                if (!gyroAvailable) Text("This phone has no gyroscope.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (gyroAvailable) {
                    Section("Gyro aim") {
                        val lt = if (profile.kind == PadKind.DUALSENSE) "L2" else "LT"
                        SegmentedControl(GyroMode.entries.map { if (it == GyroMode.AIMING) "$lt held" else it.label }, profile.gyro.ordinal, { i -> onSave(profile.copy(gyro = GyroMode.entries[i])) })
                        if (profile.gyro == GyroMode.AIMING) Text(
                            "Aims only while you hold $lt, as when aiming down sights.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    if (profile.gyro != GyroMode.OFF) {
                        Section("Gyro moves") {
                            SegmentedControl(GyroTarget.entries.map { it.label }, profile.gyroTarget.ordinal, { i -> onSave(profile.copy(gyroTarget = GyroTarget.entries[i])) })
                            Text(
                                if (profile.gyroTarget == GyroTarget.MOUSE) "Most PC games aim with the mouse, and take it alongside the controller."
                                else "For games that only take a controller.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Section("Gyro speed · ${"%.1f".format(speed)}×") {
                            LiquidSlider(
                                value = { (speed - 0.25f) / 2.75f },
                                onChange = { speed = 0.25f + it * 2.75f },
                                onDone = { onSave(profile.copy(gyroSpeed = speed)) },
                                fill = MaterialTheme.colorScheme.primary,
                            )
                        }
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text("Invert up and down", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.weight(1f))
                            VyomaSwitch(profile.invertGyroY, { onSave(profile.copy(invertGyroY = it)) })
                        }
                    }
                }
                }
                }
            }
        }
    }
    naming?.let { draft0 ->
        var draft by remember { mutableStateOf(draft0) }
        GlassDialog(
            onDismissRequest = { naming = null; renaming = false },
            title = { Text(if (renaming) "Rename profile" else "New game profile") },
            text = { OutlinedTextField(draft, { draft = it }, singleLine = true, placeholder = { Text("The game's name") }) },
            confirmButton = {
                DialogButton(enabled = draft.isNotBlank(), onClick = {
                    if (renaming) onSave(profile.copy(name = draft.trim())) else onCreate(draft.trim())
                    naming = null
                    renaming = false
                }) { Text(if (renaming) "Done" else "Create") }
            },
            dismissButton = { DialogButton(onClick = { naming = null; renaming = false }) { Text("Cancel") } },
        )
    }
    if (deleting) {
        GlassDialog(
            onDismissRequest = { deleting = false },
            title = { Text("Delete ${profile.name}?") },
            text = { Text("Its layout and settings go with it.") },
            confirmButton = { DialogButton(destructive = true, onClick = { onDelete(profile.id); deleting = false }) { Text("Delete") } },
            dismissButton = { DialogButton(onClick = { deleting = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        content()
    }
}

@Composable
private fun Chip(text: String, selected: Boolean, onClick: () -> Unit) {
    val accent = MaterialTheme.colorScheme.primary
    Box(
        Modifier
            .clip(CircleShape)
            .background(if (selected) accent else Color.White.copy(alpha = 0.08f))
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 8.dp),
    ) {
        Text(text, style = MaterialTheme.typography.labelLarge, color = if (selected) Color.White else MaterialTheme.colorScheme.onSurface)
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

/** A DualSense face symbol: cross, circle, square or triangle, outlined. */
private fun DrawScope.psGlyph(b: GamepadButton, c: Offset, r: Float, color: Color) {
    val stroke = Stroke(2.2.dp.toPx(), cap = StrokeCap.Round)
    when (b) {
        GamepadButton.A -> {
            val k = r * 0.8f
            drawLine(color, c + Offset(-k, -k), c + Offset(k, k), stroke.width, StrokeCap.Round)
            drawLine(color, c + Offset(-k, k), c + Offset(k, -k), stroke.width, StrokeCap.Round)
        }
        GamepadButton.B -> drawCircle(color, r, c, style = stroke)
        GamepadButton.X -> drawRect(color, c - Offset(r * 0.85f, r * 0.85f), Size(r * 1.7f, r * 1.7f), style = stroke)
        else -> drawPath(
            Path().apply {
                moveTo(c.x, c.y - r)
                lineTo(c.x + r * 0.95f, c.y + r * 0.7f)
                lineTo(c.x - r * 0.95f, c.y + r * 0.7f)
                close()
            },
            color, style = stroke,
        )
    }
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

/**
 * A trigger. Digital: a capsule, lit while held. Analog: a taller slot whose fill runs down as far as
 * it's pressed, the label in the capsule-sized top.
 */
private fun DrawScope.trigger(r: Rect, value: Int, analog: Boolean, tint: Color, label: TextLayoutResult?, alpha: Float) {
    if (!analog) return capsule(r, value > 0, tint, label, alpha)
    val corner = CornerRadius(r.width * 0.2f)
    drawRoundRect(Color.White.copy(alpha = 0.07f * alpha), r.topLeft, r.size, corner)
    if (value > 0) {
        val depth = r.height * value / 255f
        drawRoundRect(tint.copy(alpha = 0.3f), r.topLeft - Offset(5.dp.toPx(), 5.dp.toPx()), Size(r.width + 10.dp.toPx(), depth + 10.dp.toPx()), CornerRadius(corner.x + 5.dp.toPx()))
        drawRoundRect(tint.copy(alpha = 0.75f), r.topLeft, Size(r.width, depth), corner)
    }
    drawRoundRect(
        Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.4f * alpha), Color.White.copy(alpha = 0.06f * alpha)), r.top, r.bottom),
        r.topLeft + Offset(0.5.dp.toPx(), 0.5.dp.toPx()), Size(r.width - 1.dp.toPx(), r.height - 1.dp.toPx()), corner, style = Stroke(1.dp.toPx()),
    )
    // Notches down the slot: a quarter, a half, three quarters.
    for (q in 1..3) {
        val y = r.top + r.height * q / 4f
        drawLine(Color.White.copy(alpha = 0.18f * alpha), Offset(r.left + r.width * 0.38f, y), Offset(r.right - r.width * 0.38f, y), 1.dp.toPx())
    }
    label?.let { drawText(it, Color.White.copy(alpha = 0.9f * alpha), Offset(r.center.x - it.size.width / 2f, r.top + r.width * 0.2f - it.size.height / 2f + 8.dp.toPx())) }
}

private fun DrawScope.icon(painter: Painter, c: Offset, size: Float, alpha: Float) {
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
        val path = Path().apply {
            moveTo(p.x + dx * tip, p.y + dy * tip)
            lineTo(p.x - dx * tip + dy * tip, p.y - dy * tip + dx * tip)
            lineTo(p.x - dx * tip - dy * tip, p.y - dy * tip - dx * tip)
            close()
        }
        drawPath(path, arrow)
    }
}

/** A stick: its base where the thumb landed (resting at home, faint), the knob under the thumb. */
private fun DrawScope.stick(origin: Offset, knob: Offset, stickR: Float, knobR: Float, active: Boolean, clicked: Boolean, tint: Color, alpha: Float) {
    val a = if (active) 1f else 0.6f
    drawCircle(Color.White.copy(alpha = 0.05f * alpha * a), stickR, origin)
    drawCircle(Color.White.copy(alpha = 0.18f * alpha * a), stickR - 0.5.dp.toPx(), origin, style = Stroke(1.dp.toPx()))
    val v = knob - origin
    val len = v.getDistance()
    val k = if (len > stickR) origin + v / len * stickR else knob
    if (active) drawCircle(Brush.radialGradient(listOf(tint.copy(alpha = 0.3f), Color.Transparent), k, knobR * 2f), knobR * 2f, k)
    drawCircle(if (clicked) tint.copy(alpha = 0.8f) else Color.White.copy(alpha = (if (active) 0.28f else 0.16f) * alpha), knobR, k)
    drawCircle(
        Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.5f * alpha), Color.White.copy(alpha = 0.1f * alpha)), k.y - knobR, k.y + knobR),
        knobR - 0.5.dp.toPx(), k, style = Stroke(1.dp.toPx()),
    )
}
