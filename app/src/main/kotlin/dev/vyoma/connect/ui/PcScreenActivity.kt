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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.sp
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.designsystem.pressable
import androidx.compose.foundation.background
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
        data class Live(val width: Int, val height: Int, val showing: Boolean = false) : Phase
        data class Failed(val reason: String) : Phase
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
            plugin.openScreen().fold(
                onSuccess = { stream ->
                    pending = stream
                    phase.value = Phase.Live(stream.width, stream.height)
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
            onSize = { w, h -> phase.value = Phase.Live(w, h) },
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
    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black)
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
                detectTransformGestures { centroid, delta, factor, _ ->
                    val before = zoom.value
                    val next = (before * factor).coerceIn(1f, 5f)
                    val centre = Offset(size.width / 2f, size.height / 2f)
                    // Zoom about the fingers, then follow them; never drift off the picture.
                    val o = pan.value + (centroid - centre - pan.value) * (1 - next / before) + delta
                    scope.launch { zoom.snapTo(next); pan.snapTo(bounded(o, next, size.width, size.height)) }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        val ratio = live?.takeIf { it.width > 0 && it.height > 0 }?.let { it.width.toFloat() / it.height } ?: (16f / 9f)
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
            if (plugin != null && live?.showing == true) {
                // Top right, level with Close: the corners hold the controls, the middle the picture.
                AnimatedVisibility(
                    shown,
                    enter = fadeIn(tween(180)) + slideInVertically(CHROME_SPRING) { -it / 2 },
                    exit = fadeOut(tween(160)) + slideOutVertically(tween(200)) { -it / 2 },
                    modifier = Modifier.align(Alignment.TopEnd),
                ) {
                    ScreenshotButton(backdrop, plugin, onFlash = { flash++ })
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
            .glassChrome(backdrop, CircleShape, blur = 8.dp, tint = FROST, bevel = 16.dp, interactive = true)
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
            modifier = Modifier.glassChrome(backdrop, RoundedCornerShape(50), blur = 8.dp, tint = FROST, bevel = 12.dp).padding(horizontal = 14.dp, vertical = 9.dp),
        )
    }
}

/** A full-resolution screenshot, saved to Pictures; the view flashes when it lands. */
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
                Toast.makeText(context, "Screenshot saved to Pictures", Toast.LENGTH_SHORT).show()
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
