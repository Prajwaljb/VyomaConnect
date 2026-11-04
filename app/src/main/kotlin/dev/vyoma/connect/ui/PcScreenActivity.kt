package dev.vyoma.connect.ui

import android.content.Context
import android.content.Intent
import android.graphics.SurfaceTexture
import android.os.Bundle
import android.view.Surface
import android.view.TextureView
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.Crossfade
import androidx.compose.material.icons.rounded.Screenshot
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import dev.vyoma.connect.designsystem.Backdrop
import dev.vyoma.connect.designsystem.backdropSource
import dev.vyoma.connect.designsystem.glassChrome
import dev.vyoma.connect.designsystem.rememberBackdrop
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.VectorConverter
import androidx.compose.animation.core.VisibilityThreshold
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.sp
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.designsystem.pressable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.runtime.DisposableEffect
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.material.icons.automirrored.rounded.Backspace
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.KeyboardHide
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.material.icons.rounded.Mouse
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.ui.draw.clip
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import kotlinx.coroutines.channels.Channel
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.designsystem.VyomaTheme
import dev.vyoma.connect.platform.ScreenDecoder
import dev.vyoma.connect.protocol.plugin.builtin.CaptureState
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch

/**
 * The PC's screen, live, full screen in landscape. Pinch to zoom and drag to pan (desktop text is
 * small on a phone); double-tap toggles 2x. The cast runs only while this is visible: it stops in
 * onStop, so the PC isn't encoding for a screen nobody sees.
 */
class PcScreenActivity : ComponentActivity() {
    private val phase = MutableStateFlow<Phase>(Phase.Connecting)
    private var decoder: ScreenDecoder? = null
    private var plugin: CompanionPlugin? = null
    private var surface: Surface? = null
    private var pending: CompanionPlugin.ScreenStream? = null
    /** Keeps the phone's Wi-Fi out of power save while watching: dozing radios hold packets for 100+ ms. */
    private val wifiLock by lazy {
        (getSystemService(WIFI_SERVICE) as android.net.wifi.WifiManager)
            .createWifiLock(android.net.wifi.WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "vyoma:screen")
            .apply { setReferenceCounted(false) }
    }

    sealed interface Phase {
        data object Connecting : Phase
        /** [showing]: the first picture is on screen (until then the view stays dark). */
        /** [control]: the PC lets touches work its pointer. */
        data class Live(val width: Int, val height: Int, val showing: Boolean = false, val control: Boolean = false) : Phase
        data class Failed(val reason: String) : Phase
    }

    /**
     * Touches the moment the screen senses them, not batched to the next frame: up to a frame
     * (4-8 ms) sooner for touch control, as games do.
     */
    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        window.decorView.requestUnbufferedDispatch(android.view.InputDevice.SOURCE_CLASS_POINTER)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
        val pcId = intent.getStringExtra(EXTRA_PC)
        plugin = pcId?.let { id ->
            val hub = application.container.hub.value
            hub?.devices?.value?.values?.firstOrNull { it.identity.companionOf == id && it.state.value.isReachable }
                ?.plugin(CompanionPlugin.ID)
        }
        setContent {
            val theme by application.container.settings.theme.collectAsStateWithLifecycle(ThemeSettings())
            // Video wants a dark room whatever the app's theme.
            VyomaTheme(theme.copy(darkMode = DarkMode.DARK)) {
                val p by phase.collectAsStateWithLifecycle()
                ScreenView(
                    phase = p,
                    plugin = plugin,
                    stats = { decoder },
                    onSurface = ::surfaceChanged,
                    onClose = ::finish,
                    onRetry = ::start,
                )
            }
        }
    }

    override fun onStart() {
        super.onStart()
        runCatching { wifiLock.acquire() }
        start()
    }

    override fun onStop() {
        super.onStop()
        stop()
        runCatching { wifiLock.release() }
    }

    private fun start() {
        val plugin = plugin ?: run {
            phase.value = Phase.Failed("Vyoma's companion isn't connected on the PC")
            return
        }
        if (decoder != null || pending != null) return
        phase.value = Phase.Connecting
        lifecycleScope.launch {
            plugin.openScreen(ScreenDecoder.maxFps(display)).fold(
                onSuccess = { stream ->
                    pending = stream
                    phase.value = Phase.Live(stream.width, stream.height, control = stream.control)
                    play()
                },
                onFailure = { phase.value = Phase.Failed(it.message ?: "Couldn't show the PC's screen") },
            )
        }
    }

    /** Starts decoding once both the stream and the surface are ready, whichever comes last. */
    private fun play() {
        val stream = pending ?: return
        val target = surface ?: return
        pending = null
        decoder = ScreenDecoder(
            stream.video, target,
            onSize = { w, h -> phase.value = Phase.Live(w, h, control = stream.control) },
            onFirstFrame = { (phase.value as? Phase.Live)?.let { phase.value = it.copy(showing = true) } },
            onEnd = { error ->
                runOnUiThread {
                    decoder = null
                    if (phase.value is Phase.Live && !isFinishing && lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)) {
                        phase.value = Phase.Failed(error?.message?.let { "The stream stopped: $it" } ?: "The PC stopped sharing its screen")
                    }
                }
            },
        ).also { it.start() }
    }

    private fun surfaceChanged(s: Surface?) {
        surface = s
        if (s == null) {
            decoder?.stop()
            decoder = null
        } else {
            play()
        }
    }

    private fun stop() {
        pending?.video?.let { runCatching { it.close() } }
        pending = null
        decoder?.stop()
        decoder = null
        plugin?.let { p -> application.container.appScope.launch { p.stopScreen() } }
    }

    companion object {
        const val EXTRA_PC = "pc"

        fun open(context: Context, pcId: String) =
            context.startActivity(Intent(context, PcScreenActivity::class.java).putExtra(EXTRA_PC, pcId))
    }
}

@Composable
private fun ScreenView(
    phase: PcScreenActivity.Phase,
    plugin: CompanionPlugin?,
    stats: () -> ScreenDecoder?,
    onSurface: (Surface?) -> Unit,
    onClose: () -> Unit,
    onRetry: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val zoom = remember { Animatable(1f) }
    val pan = remember { Animatable(Offset.Zero, Offset.VectorConverter) }
    var chrome by remember { mutableStateOf(true) }
    var flash by remember { mutableIntStateOf(0) }
    val live = phase as? PcScreenActivity.Phase.Live
    // The controls step aside a few seconds after the picture appears; a tap brings them back.
    LaunchedEffect(chrome, live?.showing) {
        if (chrome && live?.showing == true) { delay(3_000); chrome = false }
    }
    val videoAlpha by animateFloatAsState(if (live?.showing == true) 1f else 0f, tween(260), label = "video")
    // The video is recorded as the backdrop the controls' liquid glass bends and blurs.
    val backdrop = rememberBackdrop()
    var controlling by remember { mutableStateOf(false) }
    var typing by remember { mutableStateOf(false) }
    // Ctrl and Alt, armed for the next key (tap again to disarm).
    var mods by remember { mutableStateOf(emptySet<String>()) }
    var hint by remember { mutableIntStateOf(0) }
    val ratio = live?.takeIf { it.width > 0 && it.height > 0 }?.let { it.width.toFloat() / it.height } ?: (16f / 9f)
    // Touches go out in order, one after another.
    val outbox = remember { Channel<Touch>(Channel.UNLIMITED) }
    val canControl = live?.control == true
    LaunchedEffect(plugin, canControl) {
        val p = plugin ?: return@LaunchedEffect
        // Touches go as a live stream (over the cable when it's in); packets on the link if the
        // stream isn't up.
        var stream: dev.vyoma.connect.webcam.LiveStream? = null
        if (canControl) {
            val s = dev.vyoma.connect.webcam.LiveStream(64 * 1024)
            stream = s
            launch(kotlinx.coroutines.Dispatchers.IO) {
                try {
                    p.screenTouches(s)
                } catch (e: java.io.IOException) {
                    VLog.w("PcScreen", "touch stream: ${e.message}") // packets carry on
                } finally {
                    if (stream === s) stream = null
                    s.close()
                }
            }
        }
        suspend fun deliver(t: Touch) {
            val line = org.json.JSONObject().apply {
                put("kind", t.kind); put("x", t.x.toDouble()); put("y", t.y.toDouble()); put("dx", t.dx.toDouble()); put("dy", t.dy.toDouble())
                t.text?.let { put("text", it) }
                t.key?.let { put("key", it) }
                if (t.mods.isNotEmpty()) put("mods", org.json.JSONArray(t.mods))
            }.toString() + "\n"
            if (stream?.offer(line.toByteArray()) != true) p.screenInput(t.kind, t.x, t.y, t.dx, t.dy, t.text, t.key, t.mods)
        }
        try {
            for (first in outbox) {
                var t = first
                // Behind? Only the newest of a run of moves matters.
                while (t.kind == "move") {
                    val next = outbox.tryReceive().getOrNull() ?: break
                    if (next.kind == "move") { t = next; continue }
                    deliver(t)
                    t = next
                    break
                }
                deliver(t)
            }
        } finally {
            stream?.close()
        }
    }
    val haptics = LocalHapticFeedback.current
    fun zoomBy(centroid: Offset, delta: Offset, factor: Float, width: Int, height: Int) {
        val before = zoom.value
        val next = (before * factor).coerceIn(1f, 5f)
        val centre = Offset(width / 2f, height / 2f)
        // Zoom about the fingers, then follow them; never drift off the picture.
        val o = pan.value + (centroid - centre - pan.value) * (1 - next / before) + delta
        scope.launch { zoom.snapTo(next); pan.snapTo(bounded(o, next, width, height)) }
    }
    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black)
            .then(
                if (controlling) Modifier.pointerInput(ratio) {
                    // Where a touch is on the PC's screen (0..1 inside the picture), through the letterbox, zoom and pan.
                    fun onPcRaw(at: Offset): Offset {
                        val w = size.width.toFloat(); val h = size.height.toFloat()
                        val (vw, vh) = if (w / h > ratio) h * ratio to h else w to w / ratio
                        val z = zoom.value
                        val left = w / 2 + pan.value.x - vw * z / 2
                        val top = h / 2 + pan.value.y - vh * z / 2
                        return Offset((at.x - left) / (vw * z), (at.y - top) / (vh * z))
                    }
                    fun onPc(at: Offset): Offset = onPcRaw(at).let { Offset(it.x.coerceIn(0f, 1f), it.y.coerceIn(0f, 1f)) }
                    /** On the black bars beside the picture, not the PC's screen. */
                    fun outside(at: Offset): Boolean = onPcRaw(at).let { it.x < 0f || it.x > 1f || it.y < 0f || it.y > 1f }
                    fun send(kind: String, at: Offset) = onPc(at).let { outbox.trySend(Touch(kind, it.x, it.y)) }
                    awaitEachGesture {
                        val first = awaitFirstDown(requireUnconsumed = false)
                        val start = first.position
                        // Touches on the bars are the viewer's own: a tap there shows the buttons.
                        val onBars = outside(start)
                        // A button over the picture (the chrome, the keyboard's keys) took it: not for the PC.
                        var byButton = first.isConsumed
                        var dragging = false
                        var multi = false
                        var moved = false
                        var scrolled = false
                        var last = start
                        // Held still: a right-click, at once (the finger may stay down).
                        val press = scope.launch {
                            delay(LONG_PRESS_MS)
                            if (onBars || byButton) return@launch
                            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                            send("right", start)
                        }
                        while (true) {
                            val event = awaitPointerEvent()
                            if (!dragging && !multi && event.changes.any { it.isConsumed }) byButton = true
                            val down = event.changes.filter { it.pressed }
                            if (down.isEmpty()) break
                            if (down.size >= 2 && !dragging) {
                                multi = true
                                press.cancel()
                            }
                            if (multi) {
                                val factor = event.calculateZoom()
                                val shift = event.calculatePan()
                                if (shift.getDistance() > 0.5f || kotlin.math.abs(factor - 1f) > 0.002f) moved = true
                                if (zoom.value > 1.01f || kotlin.math.abs(factor - 1f) > 0.01f) {
                                    zoomBy(event.calculateCentroid(), shift, factor, size.width, size.height)
                                } else if (shift.getDistance() > 0f) {
                                    // Two fingers at 1x: the PC scrolls with them (content follows the fingers).
                                    val (vw, vh) = size.width.toFloat().let { w -> val h = size.height.toFloat(); if (w / h > ratio) h * ratio to h else w to w / ratio }
                                    val pcW = live?.width?.toFloat() ?: vw
                                    val pcH = live?.height?.toFloat() ?: vh
                                    scrolled = true
                                    val c = onPc(event.calculateCentroid())
                                    outbox.trySend(Touch("scroll", c.x, c.y, -shift.x / vw * pcW, -shift.y / vh * pcH))
                                }
                                event.changes.forEach { it.consume() }
                            } else {
                                val at = down.first().position
                                if (!onBars && !byButton && !dragging && !press.isCompleted && (at - start).getDistance() > viewConfiguration.touchSlop) {
                                    dragging = true
                                    press.cancel()
                                    send("down", start)
                                }
                                if (dragging && at != last) send("move", at)
                                last = at
                                down.first().consume()
                            }
                        }
                        val rightClicked = press.isCompleted && !press.isCancelled
                        press.cancel()
                        when {
                            byButton && !dragging -> Unit
                            multi -> {
                                if (scrolled) outbox.trySend(Touch("scrollend"))
                                // A quick two-finger tap shows the buttons.
                                if (!moved && last == start) chrome = !chrome
                            }
                            dragging -> send("up", last)
                            onBars -> if ((last - start).getDistance() < viewConfiguration.touchSlop) chrome = !chrome
                            !rightClicked -> send("click", start)
                        }
                    }
                } else Modifier
                    .pointerInput(Unit) {
                        detectTapGestures(
                            onTap = { chrome = !chrome },
                            onDoubleTap = { at ->
                                val centre = Offset(size.width / 2f, size.height / 2f)
                                val (z, o) = if (zoom.value > 1.01f) 1f to Offset.Zero else 2.5f to bounded((centre - at) * 1.5f, 2.5f, size.width, size.height)
                                scope.launch { zoom.animateTo(z, ZOOM_SPRING) }
                                scope.launch { pan.animateTo(o, spring(dampingRatio = 0.85f, stiffness = Spring.StiffnessMediumLow)) }
                            },
                        )
                    }
                    .pointerInput(Unit) {
                        detectTransformGestures { centroid, delta, factor, _ -> zoomBy(centroid, delta, factor, size.width, size.height) }
                    },
            ),
        contentAlignment = Alignment.Center,
    ) {
        // TextureView, not SurfaceView: it scales and pans with the zoom like any other view, and it can
        // be recorded for the glass (a SurfaceView bypasses the app's drawing and would leave it bending black).
        Box(Modifier.fillMaxSize().backdropSource(backdrop), contentAlignment = Alignment.Center) { AndroidView(
            factory = { context ->
                TextureView(context).apply {
                    surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                        private var surface: Surface? = null
                        override fun onSurfaceTextureAvailable(st: SurfaceTexture, w: Int, h: Int) {
                            surface = Surface(st).also(onSurface)
                        }
                        override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
                            onSurface(null)
                            surface?.release()
                            surface = null
                            return true
                        }
                        override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, w: Int, h: Int) = Unit
                        override fun onSurfaceTextureUpdated(st: SurfaceTexture) = Unit
                    }
                }
            },
            modifier = Modifier
                .aspectRatio(ratio)
                .graphicsLayer {
                    scaleX = zoom.value; scaleY = zoom.value
                    translationX = pan.value.x; translationY = pan.value.y
                    alpha = videoAlpha
                },
        ) }

        if (typing && live?.control == true) {
            fun key(name: String) {
                outbox.trySend(Touch("key", key = name, mods = mods.toList()))
                mods = emptySet()
            }
            PcKeyboard(
                onText = { text ->
                    // An armed Ctrl/Alt makes a single character a shortcut (Ctrl+C), not typing.
                    if (mods.isNotEmpty() && text.length == 1) key(text) else outbox.trySend(Touch("text", text = text))
                },
                onKey = ::key,
                onClosed = { typing = false },
            )
            Box(Modifier.fillMaxSize().imePadding().safeDrawingPadding().padding(horizontal = 12.dp, vertical = 8.dp), contentAlignment = Alignment.BottomCenter) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    @Composable
                    fun Key(label: String, onClick: () -> Unit, armed: Boolean = false, icon: ImageVector? = null) =
                        GlassButton(backdrop, label, icon ?: Icons.Rounded.Keyboard, size = 44.dp, onClick = onClick, content = if (icon != null) null else ({
                            Text(label, color = if (armed) Color(0xFF6FB6FF) else Color.White, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, style = androidx.compose.ui.text.TextStyle(shadow = GLYPH_SHADOW))
                        }))
                    Key("Esc", { key("Escape") })
                    Key("Tab", { key("Tab") })
                    for ((label, mod) in listOf("Ctrl" to "ctrl", "Alt" to "alt")) {
                        Key(label, { mods = if (mod in mods) mods - mod else mods + mod }, armed = mod in mods)
                    }
                    Key("Super", { key("Super") }, icon = Icons.Rounded.Apps)
                    Key("Left", { key("Left") }, icon = Icons.AutoMirrored.Rounded.KeyboardArrowLeft)
                    Key("Up", { key("Up") }, icon = Icons.Rounded.KeyboardArrowUp)
                    Key("Down", { key("Down") }, icon = Icons.Rounded.KeyboardArrowDown)
                    Key("Right", { key("Right") }, icon = Icons.AutoMirrored.Rounded.KeyboardArrowRight)
                    Key("Delete", { key("Delete") }, icon = Icons.AutoMirrored.Rounded.Backspace)
                    Key("Hide the keyboard", { typing = false }, icon = Icons.Rounded.KeyboardHide)
                }
            }
        }

        // Screenshot flash: a white veil that fades out, like the system screenshot.
        val veil = remember { Animatable(0f) }
        LaunchedEffect(flash) {
            if (flash == 0) return@LaunchedEffect
            veil.snapTo(0.55f)
            veil.animateTo(0f, tween(320))
        }
        if (veil.value > 0f) Box(Modifier.fillMaxSize().graphicsLayer { alpha = veil.value }.background(Color.White))

        when (phase) {
            PcScreenActivity.Phase.Connecting -> Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(14.dp)) {
                CircularProgressIndicator(color = Color.White, strokeWidth = 2.5.dp, modifier = Modifier.size(30.dp))
                Text(
                    "Connecting to the PC's screen…\nIf the PC asks which screen to share, choose one there.",
                    color = Color.White.copy(alpha = 0.75f),
                    fontSize = 14.sp,
                    lineHeight = 20.sp,
                    textAlign = TextAlign.Center,
                )
            }
            is PcScreenActivity.Phase.Failed -> Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(18.dp)) {
                Text(phase.reason, color = Color.White, fontSize = 15.sp, lineHeight = 21.sp, textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = 48.dp))
                PillButton("Try again", onClick = onRetry)
            }
            is PcScreenActivity.Phase.Live -> Unit
        }

        val shown = chrome || live?.showing != true
        Box(Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 16.dp, vertical = 12.dp)) {
            AnimatedVisibility(
                shown,
                enter = fadeIn(tween(180)) + slideInVertically(CHROME_SPRING) { -it / 2 },
                exit = fadeOut(tween(160)) + slideOutVertically(tween(200)) { -it / 2 },
                modifier = Modifier.align(Alignment.TopStart),
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                    GlassButton(backdrop, "Close", Icons.Rounded.Close, onClick = onClose)
                    if (live?.showing == true) RateReadout(backdrop, stats)
                }
            }
            // How touch control works, for a few seconds after turning it on.
            var hintShown by remember { mutableStateOf(false) }
            LaunchedEffect(hint) {
                if (hint == 0) return@LaunchedEffect
                hintShown = true
                delay(5_000)
                hintShown = false
            }
            AnimatedVisibility(
                hintShown && controlling,
                enter = fadeIn(tween(180)),
                exit = fadeOut(tween(220)),
                modifier = Modifier.align(Alignment.BottomCenter),
            ) {
                Text(
                    "Tap to click · hold to right-click · drag to drag · two fingers scroll or zoom · tap the black bars for the buttons",
                    color = Color.White,
                    fontSize = 13.sp,
                    textAlign = TextAlign.Center,
                    style = androidx.compose.ui.text.TextStyle(shadow = GLYPH_SHADOW),
                    modifier = Modifier
                        .clip(RoundedCornerShape(50))
                        .background(Color.Black.copy(alpha = 0.55f))
                        .padding(horizontal = 16.dp, vertical = 10.dp),
                )
            }
            if (plugin != null && live?.showing == true) {
                // Top right, level with Close: the corners hold the controls, the middle the picture.
                AnimatedVisibility(
                    shown,
                    enter = fadeIn(tween(180)) + slideInVertically(CHROME_SPRING) { -it / 2 },
                    exit = fadeOut(tween(160)) + slideOutVertically(tween(200)) { -it / 2 },
                    modifier = Modifier.align(Alignment.TopEnd),
                ) {
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                        if (live.control) {
                            GlassButton(backdrop, if (typing) "Hide the keyboard" else "Type on the PC", Icons.Rounded.Keyboard, onClick = { typing = !typing })
                            GlassButton(
                                backdrop, if (controlling) "Stop controlling" else "Control the PC",
                                if (controlling) Icons.Rounded.TouchApp else Icons.Rounded.Mouse,
                                onClick = {
                                    controlling = !controlling
                                    if (controlling) hint++
                                },
                            )
                        }
                        ScreenshotButton(backdrop, plugin, onFlash = { flash++ })
                    }
                }
            }
        }
    }
}

/**
 * The controls' glass: a mid-grey frost rather than the app's dark chrome. It lifts the black bands
 * beside a picture that doesn't fill the screen (nothing there to bend) and tones down a white one,
 * so the glass and its white glyphs read over anything the PC shows.
 */
private val FROST = Color(0xFF7A7A7A).copy(alpha = 0.32f)
/** Keeps white glyphs legible over the brightest pictures. */
private val GLYPH_SHADOW = androidx.compose.ui.graphics.Shadow(Color.Black.copy(alpha = 0.45f), Offset(0f, 1f), 6f)

private val CHROME_SPRING = spring(dampingRatio = 0.78f, stiffness = Spring.StiffnessMediumLow, visibilityThreshold = IntOffset.VisibilityThreshold)
private val ZOOM_SPRING = spring(dampingRatio = 0.85f, stiffness = Spring.StiffnessMediumLow, visibilityThreshold = 0.001f)

/** Past this much typed, the hidden field starts over (between keys, telling the keyboard). */
private const val FIELD_MAX = 2_000

/**
 * The phone's keyboard, typing on the PC: an invisible field the keyboard writes into. Each change
 * is compared with the one before (deleted → Backspaces, added → typing); the field is never
 * rewritten under the keyboard mid-edit (resetting it after every key made Gboard repeat letters,
 * 2026-11-04). Backspace on the empty field comes as a key. Plain input (no suggestions), so every
 * key goes at once; no full-screen landscape editor, so the PC stays in view.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PcKeyboard(onText: (String) -> Unit, onKey: (String) -> Unit, onClosed: () -> Unit) {
    val ime = WindowInsets.isImeVisible
    var shown by remember { mutableStateOf(false) }
    // The keyboard swiped away with Back: done typing.
    LaunchedEffect(ime) {
        if (ime) shown = true else if (shown) onClosed()
    }
    // Closing from our own button: hide the keyboard through a view that's still attached.
    val host = androidx.compose.ui.platform.LocalView.current
    DisposableEffect(Unit) {
        onDispose {
            host.context.getSystemService(android.view.inputmethod.InputMethodManager::class.java).hideSoftInputFromWindow(host.windowToken, 0)
            androidx.core.view.ViewCompat.getWindowInsetsController(host)?.hide(androidx.core.view.WindowInsetsCompat.Type.ime())
        }
    }
    AndroidView(
        factory = { context ->
            android.widget.EditText(context).apply {
                inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or
                    android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                imeOptions = android.view.inputmethod.EditorInfo.IME_FLAG_NO_EXTRACT_UI or android.view.inputmethod.EditorInfo.IME_FLAG_NO_FULLSCREEN or
                    android.view.inputmethod.EditorInfo.IME_ACTION_NONE
                background = null
                alpha = 0f
                var before = ""
                var clearing = false
                addTextChangedListener(object : android.text.TextWatcher {
                    override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
                    override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) = Unit
                    override fun afterTextChanged(s: android.text.Editable) {
                        val now = s.toString()
                        if (clearing) { before = now; return }
                        if (now == before) return
                        val same = now.commonPrefixWith(before).length
                        repeat(before.length - same) { onKey("BackSpace") }
                        now.substring(same).takeIf { it.isNotEmpty() }?.let(onText)
                        before = now
                        if (now.length > FIELD_MAX) post {
                            clearing = true
                            text.clear()
                            clearing = false
                            context.getSystemService(android.view.inputmethod.InputMethodManager::class.java).restartInput(this@apply)
                        }
                    }
                })
                // Keys the keyboard sends as keys: Enter, and Backspace once the field is empty.
                setOnKeyListener { _, code, event ->
                    if (event.action != android.view.KeyEvent.ACTION_DOWN) return@setOnKeyListener false
                    when (code) {
                        android.view.KeyEvent.KEYCODE_ENTER, android.view.KeyEvent.KEYCODE_NUMPAD_ENTER -> { onKey("Return"); true }
                        android.view.KeyEvent.KEYCODE_DEL -> if (text.isEmpty()) { onKey("BackSpace"); true } else false
                        android.view.KeyEvent.KEYCODE_FORWARD_DEL -> { onKey("Delete"); true }
                        else -> false
                    }
                }
                setOnEditorActionListener { _, _, _ -> onKey("Return"); true }
                post {
                    requestFocus()
                    context.getSystemService(android.view.inputmethod.InputMethodManager::class.java).showSoftInput(this, 0)
                }
            }
        },
        modifier = Modifier.size(1.dp),
    )
}

/** One touch for the PC's pointer (see [CompanionPlugin.screenInput]). */
private class Touch(
    val kind: String,
    val x: Float = 0f,
    val y: Float = 0f,
    val dx: Float = 0f,
    val dy: Float = 0f,
    val text: String? = null,
    val key: String? = null,
    val mods: List<String> = emptyList(),
)

/** Held this long without moving: a right-click. */
private const val LONG_PRESS_MS = 450L

/** Keeps a zoomed picture covering its frame: no panning past its edges. */
private fun bounded(o: Offset, scale: Float, width: Int, height: Int): Offset {
    val maxX = width * (scale - 1) / 2
    val maxY = height * (scale - 1) / 2
    return Offset(o.x.coerceIn(-maxX, maxX), o.y.coerceIn(-maxY, maxY))
}

/** A round liquid-glass button over the video, like the tab bar's touchpad button. */
@Composable
private fun GlassButton(backdrop: Backdrop, label: String, icon: ImageVector, size: Dp = 48.dp, enabled: Boolean = true, onClick: () -> Unit, content: (@Composable () -> Unit)? = null) {
    Box(
        Modifier
            .size(size)
            .glassChrome(backdrop, CircleShape, blur = 6.dp, tint = FROST, bevel = 16.dp, interactive = true)
            .pressable(enabled = enabled, pressedScale = 1f, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        content?.invoke() ?: Icon(icon, null, tint = Color.White, modifier = Modifier.size(22.dp))
    }
}

/** Frames per second, bitrate, and the phone's decode time per frame: what the cast is really doing. */
@Composable
private fun RateReadout(backdrop: Backdrop, stats: () -> ScreenDecoder?) {
    var text by remember { mutableStateOf("") }
    LaunchedEffect(Unit) {
        var lastFrames = -1L
        var lastBytes = 0L
        var lastHeld = 0L
        while (true) {
            val d = stats()
            if (d != null) {
                val f = d.frames.get()
                val b = d.bytes.get()
                val held = d.heldUs.get()
                if (lastFrames >= 0) {
                    val n = f - lastFrames
                    val decodeMs = if (n > 0) (held - lastHeld) / n / 1000 else 0
                    text = "$n fps · %.1f Mb/s · %d ms".format((b - lastBytes) * 8 / 1e6, decodeMs)
                }
                lastFrames = f; lastBytes = b; lastHeld = held
            }
            delay(1_000)
        }
    }
    if (text.isNotEmpty()) {
        Text(
            text,
            color = Color.White.copy(alpha = 0.9f),
            fontSize = 12.sp,
            fontWeight = FontWeight.Medium,
            // Tabular digits: the pill doesn't twitch as the numbers change.
            style = TextStyle(fontFeatureSettings = "tnum", shadow = GLYPH_SHADOW),
            modifier = Modifier.glassChrome(backdrop, RoundedCornerShape(50), blur = 6.dp, tint = FROST, bevel = 12.dp).padding(horizontal = 14.dp, vertical = 9.dp),
        )
    }
}

/** A full-resolution screenshot, saved to Pictures/Vyoma; the view flashes when it lands. */
@Composable
private fun ScreenshotButton(backdrop: Backdrop, plugin: CompanionPlugin, onFlash: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val haptics = LocalHapticFeedback.current
    val shot by plugin.screenshot.state.collectAsStateWithLifecycle()
    var asked by remember { mutableStateOf(false) }
    LaunchedEffect(shot) {
        if (!asked) return@LaunchedEffect
        when (val s = shot) {
            is CaptureState.Done -> {
                asked = false
                onFlash()
                haptics.perform(Haptic.Success)
                Toast.makeText(context, "Screenshot saved to Pictures/Vyoma", Toast.LENGTH_SHORT).show()
            }
            is CaptureState.Failed -> {
                asked = false
                haptics.perform(Haptic.Failure)
                Toast.makeText(context, s.reason, Toast.LENGTH_SHORT).show()
            }
            else -> Unit
        }
    }
    val busy = shot == CaptureState.Capturing
    val scope = rememberCoroutineScope()
    GlassButton(backdrop, "Take a screenshot", Icons.Rounded.Screenshot, enabled = !busy, onClick = { asked = true; scope.launch { plugin.screenshot.request() } }) {
        Crossfade(busy, label = "shot") { b ->
            if (b) CircularProgressIndicator(color = Color.White, strokeWidth = 2.dp, modifier = Modifier.size(22.dp))
            else Icon(Icons.Rounded.Screenshot, null, tint = Color.White, modifier = Modifier.size(22.dp))
        }
    }
}
