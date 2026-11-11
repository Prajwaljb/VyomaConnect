package dev.vyoma.connect.ui

import android.annotation.SuppressLint
import android.graphics.RuntimeShader
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.TrendingUp
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.NightsStay
import androidx.compose.material.icons.rounded.Phone
import androidx.compose.material.icons.rounded.Photo
import androidx.compose.material.icons.rounded.Smartphone
import androidx.compose.material.icons.rounded.SportsEsports
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.material.icons.rounded.Videocam
import androidx.compose.material.icons.rounded.WifiOff
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import dev.vyoma.connect.designsystem.Backdrop
import dev.vyoma.connect.designsystem.Blob
import dev.vyoma.connect.designsystem.GlassQuality
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.SpecularLight
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.backdropSource
import dev.vyoma.connect.designsystem.glassChrome
import dev.vyoma.connect.designsystem.liquidPress
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.designsystem.pressable
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.exp
import kotlin.math.floor
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** How many scenes the show has; the day runs through them, and setup carries on from night into morning. */
private const val SCENES = 6

/**
 * Dunes under a sky, drawn rather than shipped: the glass needs something vivid and detailed to bend
 * (clear glass over flat colour looks like nothing), and no photograph can change with the time of
 * day or ripple when you pair. Layers of sine ridges, each with a lit and a shaded face and haze with
 * distance. [wave] pushes the whole picture outward in a ring.
 */
private const val DUNES_AGSL = """
uniform float2 size;
uniform float time;
uniform float pan;
uniform float3 skyTop;
uniform float3 skyLow;
uniform float3 sun;
uniform float3 lit;
uniform float3 shade;
uniform float2 sunAt;
uniform float3 wave;
uniform float waveAmp;

float ridge(float x, float s) {
    return sin(x * 1.3 + s) * 0.5 + sin(x * 2.9 + s * 2.1) * 0.22 + sin(x * 6.1 + s * 0.7) * 0.07;
}

half4 main(float2 at) {
    float2 p = at;
    if (waveAmp > 0.01) {
        float2 d = at - wave.xy;
        float r = length(d);
        float e = r - wave.z;
        float w = sin(e * 0.03) * exp(-abs(e) * 0.007) * waveAmp;
        if (r > 0.5) p += d / r * w;
    }
    float ay = size.y / size.x;
    float2 uv = p / size.x;
    float y = uv.y / ay;
    float3 col = mix(skyTop, skyLow, pow(clamp(y / 0.62, 0.0, 1.0), 2.4));
    float g = length(uv - sunAt * float2(1.0, ay));
    col += sun * (0.5 * exp(-g * 3.2) + 0.7 * exp(-g * 13.0));
    // The brightest thing in the picture: only the halo at full, the disc was darker than its own glow.
    col = mix(col, min(col + sun * 0.7, float3(1.0)), smoothstep(0.062, 0.056, g));
    for (int i = 0; i < 4; i++) {
        float k = float(i);
        float depth = k / 3.0;
        float drift = pan * (0.05 + 0.13 * depth) + time * (0.006 + 0.012 * depth);
        float x = uv.x * (2.6 - depth * 0.9) + drift * 3.0;
        float h = 0.54 + 0.115 * k + ridge(x, k * 3.1) * (0.04 + 0.035 * depth);
        float dy = y - h;
        float edge = smoothstep(0.0, 0.0016, dy);
        if (edge > 0.0) {
            // The crest between a dune's two faces: an S running down the slope, sharp on the shaded
            // side and easing away on the lit one, as wind leaves it.
            float bend = sin(dy * 7.0 + x * 0.8 + k * 1.7) * 0.9 + sin(dy * 19.0 - x * 1.3) * 0.18;
            float c = sin(x * 1.55 + k * 2.3 + bend - dy * 9.0);
            float face = smoothstep(-0.2, -0.04, c);
            float3 d = mix(shade, lit * (0.86 + 0.2 * c), face * (1.0 - depth * 0.2));
            d += lit * 0.018 * sin(dy * 520.0 + x * 34.0 + bend * 6.0) * face;
            d = mix(d, skyLow, (1.0 - depth) * 0.4);
            d += lit * 0.34 * exp(-dy * 150.0);
            d *= 1.0 - clamp(dy * 1.5, 0.0, 0.62) * depth;
            col = mix(col, d, edge);
        }
    }
    col += (fract(sin(dot(at, float2(12.9898, 78.233))) * 43758.5453) - 0.5) / 255.0;
    return half4(col, 1.0);
}
"""

private class DayColours(val top: Color, val low: Color, val sun: Color, val lit: Color, val shade: Color, val x: Float, val y: Float)

// Blue over orange is the picture that sells clear glass; no purple, by the app's rules.
private val DAY = listOf(
    DayColours(Color(0xFF0E4A86), Color(0xFFF9A65A), Color(0xFFFFE3B0), Color(0xFFF28B3C), Color(0xFF5E2438), 0.72f, 0.40f), // golden hour
    DayColours(Color(0xFF0757C9), Color(0xFF79C6FF), Color(0xFFFFFBE6), Color(0xFFE9892A), Color(0xFF9C3F14), 0.86f, 0.17f), // noon
    DayColours(Color(0xFF00707C), Color(0xFF86DCCB), Color(0xFFFFF4D2), Color(0xFFE88A33), Color(0xFF99402A), 0.60f, 0.17f), // afternoon
    DayColours(Color(0xFF14406B), Color(0xFFFF6F3A), Color(0xFFFFD08A), Color(0xFFFF8A3E), Color(0xFF701F2D), 0.25f, 0.42f), // sunset
    DayColours(Color(0xFF0A2547), Color(0xFFC6553B), Color(0xFFFF9D6B), Color(0xFFCE5D3A), Color(0xFF331A23), 0.80f, 0.50f), // dusk
    DayColours(Color(0xFF020816), Color(0xFF123A63), Color(0xFFDDE9F5), Color(0xFF5B8FBC), Color(0xFF091A2E), 0.82f, 0.17f), // night
)

/** Time for loops the demos draw: read only while drawing, so nothing recomposes for it. */
@Stable
internal class ShowClock {
    var seconds by mutableFloatStateOf(0f)
}

/** The ring that runs through the picture when a PC pairs. */
@Stable
internal class Ripple {
    val radius = Animatable(0f)
    val strength = Animatable(0f)
}

/** Runs while [running]. It stands still with Lighter glass, or with animations off in Android. */
@Composable
internal fun rememberShowClock(running: Boolean): ShowClock {
    val clock = remember { ShowClock() }
    val context = LocalContext.current
    val still = !running || GlassQuality.lite || remember { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
    LaunchedEffect(still) {
        if (still) return@LaunchedEffect
        val start = withFrameNanos { it } - (clock.seconds * 1e9f).toLong()
        while (true) withFrameNanos { clock.seconds = (it - start) / 1e9f }
    }
    return clock
}

/** The picture itself, and whatever's laid in it, as what [backdrop] bends. */
@Composable
internal fun DuneScene(backdrop: Backdrop, clock: ShowClock, ripple: Ripple, day: () -> Float, modifier: Modifier = Modifier, content: @Composable () -> Unit = {}) {
    var area by remember { mutableStateOf(Size.Zero) }
    // The glass over the picture is lit by the sun in it, wherever that's got to (see SpecularLight.source).
    DisposableEffect(Unit) {
        val light = SpecularLight.Source(
            at = { val (x, y) = sunAt(day()); Offset(x * area.width, y * area.height) },
            colour = { lightOf(day()) },
            across = { hypot(area.width, area.height) },
        )
        SpecularLight.source = light
        onDispose { if (SpecularLight.source === light) SpecularLight.source = null }
    }
    Box(modifier.fillMaxSize().onSizeChanged { area = Size(it.width.toFloat(), it.height.toFloat()) }.backdropSource(backdrop).dunes(clock, ripple, day)) { content() }
}

/** Between which two scenes [day] falls, and how far along. */
private fun <T> atDay(day: Float, of: (DayColours, DayColours, Float) -> T): T {
    val at = day.mod(SCENES.toFloat())
    return of(DAY[floor(at).toInt() % SCENES], DAY[(floor(at).toInt() + 1) % SCENES], at - floor(at))
}

/** Where the sun (or at night, the moon) is, as fractions of the picture. */
private fun sunAt(day: Float): Pair<Float, Float> = atDay(day) { a, b, f -> a.x + (b.x - a.x) * f to a.y + (b.y - a.y) * f }

/** The light it gives: its own colour, towards white so the glass still reads as lit rather than painted. */
private fun lightOf(day: Float): Color = atDay(day) { a, b, f -> lerp(lerp(a.sun, b.sun, f), Color.White, 0.25f) }

@SuppressLint("NewApi")
private fun Modifier.dunes(clock: ShowClock, ripple: Ripple, day: () -> Float): Modifier = composed {
    val shader = remember { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) RuntimeShader(DUNES_AGSL) else null }
    val brush = remember(shader) { shader?.let { ShaderBrush(it) } }
    // Lighter glass draws it at half size and scales it up: a quarter of the shader's work, for a
    // picture soft enough not to show it.
    val half = rememberGraphicsLayer()
    drawBehind {
        val at = day().mod(SCENES.toFloat())
        val from = DAY[floor(at).toInt() % SCENES]
        val to = DAY[(floor(at).toInt() + 1) % SCENES]
        val f = at - floor(at)
        fun mix(a: Color, b: Color) = lerp(a, b, f)
        val (top, low, sun) = Triple(mix(from.top, to.top), mix(from.low, to.low), mix(from.sun, to.sun))
        val (lit, shade) = mix(from.lit, to.lit) to mix(from.shade, to.shade)
        if (shader == null || brush == null) {
            // Android 12 has no runtime shaders: the same sky and sand as a plain gradient.
            drawRect(Brush.verticalGradient(0f to top, 0.6f to low, 0.62f to lit, 1f to shade))
            return@drawBehind
        }
        shader.setFloatUniform("size", size.width, size.height)
        shader.setFloatUniform("time", clock.seconds)
        shader.setFloatUniform("pan", day() * 2.2f)
        shader.setFloatUniform("skyTop", top.red, top.green, top.blue)
        shader.setFloatUniform("skyLow", low.red, low.green, low.blue)
        shader.setFloatUniform("sun", sun.red, sun.green, sun.blue)
        shader.setFloatUniform("lit", lit.red, lit.green, lit.blue)
        shader.setFloatUniform("shade", shade.red, shade.green, shade.blue)
        val (sunX, sunY) = sunAt(day())
        shader.setFloatUniform("sunAt", sunX, sunY)
        shader.setFloatUniform("wave", size.width / 2, size.height * 0.42f, ripple.radius.value)
        shader.setFloatUniform("waveAmp", ripple.strength.value)
        if (!GlassQuality.lite) return@drawBehind drawRect(brush)
        val full = size
        half.record(IntSize((full.width / 2).roundToInt().coerceAtLeast(1), (full.height / 2).roundToInt().coerceAtLeast(1))) {
            scale(0.5f, Offset.Zero) { drawRect(brush, size = full) }
        }
        scale(2f, Offset.Zero) { drawLayer(half) }
    }
}

/** A loop's motion: a spring's step from 0 to 1 over [t] seconds, for things drawn straight from the clock. */
private fun settle(t: Float): Float = if (t <= 0f) 0f else 1f - exp(-5.5f * t) * cos(9f * t)

internal val Soft = Shadow(Color.Black.copy(alpha = 0.55f), blurRadius = 12f)

/** What the show has to say. Text sits on glass, or on the picture with a soft shadow. */
private class Scene(val title: String, val body: String)

private val SCENE_WORDS = listOf(
    Scene("", ""),
    Scene("Your PC, in your hand", "Watch its screen live, see how it's doing, lock it or put it to sleep from anywhere in the house."),
    Scene("", ""), // says what the chosen mode does
    Scene("Copy here, paste there", "What you copy is ready to paste on the other side. Photos, files and whole folders go both ways."),
    Scene("It looks after itself", "Rules run while you get on with your day. Choose what happens, and when."),
    Scene("Between you and your PC", "No account and no cloud. It's encrypted, and each PC has to match a code on your screen before it's trusted."),
)

private val CONTROL_MODES = listOf(
    Triple(Icons.Rounded.TouchApp, "Touchpad and keyboard", "Your thumb moves its cursor, with the gestures you know. Type from the sofa."),
    Triple(Icons.Rounded.SportsEsports, "A real game controller", "Sticks, triggers and buttons that the PC's games see as a controller."),
    Triple(Icons.Rounded.Videocam, "A better webcam", "This phone's camera and mic become the PC's, for any app and any call."),
)

/**
 * The first scene's entrance, shared by its words (in the picture) and its buttons (over it). The
 * name is there from the first frame; once the glass has dropped in, the line comes up a half at a
 * time, then the hint, then the way on: Show me, Skip and the dots together (user, 2026-11-09).
 * With animations off it's all just there.
 */
@Stable
internal class WelcomeIntro(still: Boolean) {
    val line = Animatable(if (still) 1f else 0f)
    val more = Animatable(if (still) 1f else 0f)
    val hint = Animatable(if (still) 1f else 0f)
    val controls = Animatable(if (still) 1f else 0f)
}

@Composable
internal fun rememberWelcomeIntro(): WelcomeIntro {
    val context = LocalContext.current
    val intro = remember { WelcomeIntro(Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f) }
    LaunchedEffect(intro) {
        val rise = LinearOutSlowInEasing
        // The lens takes about 1.3 s to fall in and come to rest over the name.
        launch { delay(1300); intro.line.animateTo(1f, tween(1000, easing = rise)) }
        launch { delay(2000); intro.more.animateTo(1f, tween(1000, easing = rise)) }
        // The hint's fade is the one the user liked; only its turn moved.
        launch { delay(2900); intro.hint.animateTo(1f, tween(1600, easing = rise)) }
        launch { delay(3900); intro.controls.animateTo(1f, tween(900, easing = rise)) }
    }
    return intro
}

/** The show before setup. [onDone] leads on to it; so does Skip. */
@Composable
internal fun WelcomeShow(backdrop: Backdrop, clock: ShowClock, intro: WelcomeIntro, page: (Float) -> Unit, onLensTouched: () -> Unit, onDone: () -> Unit) {
    val pager = rememberPagerState { SCENES }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    LaunchedEffect(pager) { snapshotFlow { pager.currentPage + pager.currentPageOffsetFraction }.collect { page(it) } }
    LaunchedEffect(pager) { snapshotFlow { pager.settledPage }.collect { haptics.perform(Haptic.Select) } }
    BackHandler(pager.currentPage > 0) { scope.launch { pager.animateScrollToPage(pager.currentPage - 1) } }
    var mode by remember { mutableIntStateOf(0) }
    val atFirst = { 1f - abs(pager.currentPage + pager.currentPageOffsetFraction).coerceAtMost(1f) }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val width = constraints.maxWidth.toFloat()
        // No swiping on before Show me is there: the intro plays first, then the button says what's next.
        val canSwipe by remember { derivedStateOf { intro.controls.value >= 0.5f } }
        HorizontalPager(pager, Modifier.fillMaxSize(), beyondViewportPageCount = 1, userScrollEnabled = canSwipe) { p ->
            Column(
                Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().padding(start = 20.dp, end = 20.dp, top = 56.dp, bottom = 118.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                // The first scene's words are in the picture (WelcomeTitle), so that the lens bends them.
                if (p != 0) {
                    Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                        when (p) {
                            1 -> ScreenDemo(backdrop, clock)
                            2 -> ControlsDemo(backdrop, clock, mode) { mode = it }
                            3 -> FlowDemo(backdrop, clock)
                            4 -> RulesDemo(backdrop, clock)
                            else -> PrivateDemo(backdrop, clock)
                        }
                    }
                    Spacer(Modifier.height(16.dp))
                    val words = SCENE_WORDS[p]
                    WordsPanel(
                        backdrop,
                        if (p == 2) CONTROL_MODES[mode].second else words.title,
                        if (p == 2) CONTROL_MODES[mode].third else words.body,
                    )
                }
            }
        }
        // The lens on the first scene, leaving with it. It belongs over the pager so a drag reaches it first.
        // Not on lighter glass: without the shader it was a still grey disc (user, 2026-11-07).
        if (!GlassQuality.lite) DragLens(backdrop, clock, onLensTouched, Modifier.graphicsLayer { translationX = -(1f - atFirst()) * width; alpha = atFirst() })

        // Skip stays out of the way; there's nothing it skips that setup needs.
        GlassPill(
            "Skip", backdrop,
            Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(top = 8.dp, end = 16.dp).graphicsLayer { alpha = if (pager.currentPage == SCENES - 1) 0f else intro.controls.value; compositingStrategy = CompositingStrategy.ModulateAlpha },
            onClick = { if (intro.controls.value > 0.5f) onDone() },
        )
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().navigationBarsPadding().padding(start = 20.dp, end = 20.dp, bottom = 16.dp)
                // Alpha per draw, not through an offscreen layer: that cut the glass's shadow to a square
                // while it faded in (user, 2026-11-09).
                .graphicsLayer { alpha = intro.controls.value; translationY = (1f - intro.controls.value) * 14.dp.toPx(); compositingStrategy = CompositingStrategy.ModulateAlpha },
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            PageDots(SCENES) { pager.currentPage + pager.currentPageOffsetFraction }
            Spacer(Modifier.height(16.dp))
            val last = pager.currentPage == SCENES - 1
            GlassPill(
                if (last) "Set up Vyoma" else if (pager.currentPage == 0) "Show me" else "Next", backdrop,
                Modifier.fillMaxWidth(), large = true,
                onClick = { if (intro.controls.value < 0.5f) Unit else if (last) onDone() else scope.launch { pager.animateScrollToPage(pager.currentPage + 1, animationSpec = VyomaMotion.page()) } },
            )
        }
    }
}

/** The first scene's words, drawn into the picture so that the lens bends them too (over it, they vanished under the lens). */
@Composable
internal fun WelcomeTitle(visible: () -> Float, touched: Boolean, intro: WelcomeIntro) {
    // The name is there from the start; the rest follows the glass in (WelcomeIntro).
    BoxWithConstraints(Modifier.fillMaxSize().graphicsLayer { alpha = visible(); translationX = -(1f - visible()) * size.width }) {
        Text(
            "Vyoma",
            style = TextStyle(fontSize = 84.sp, fontWeight = FontWeight.Bold, letterSpacing = (-2).sp, color = Color.White, shadow = Soft),
            modifier = Modifier.align(Alignment.TopCenter).padding(top = maxHeight * 0.27f),
        )
        val hint by animateFloatAsState(if (touched || GlassQuality.lite) 0f else 1f, VyomaMotion.effects(), label = "hint")
        Text(
            "Go on, drag the glass",
            style = MaterialTheme.typography.labelLarge.copy(shadow = Soft),
            color = Color.White,
            modifier = Modifier.align(Alignment.TopCenter).padding(top = maxHeight * 0.46f).graphicsLayer { alpha = hint * intro.hint.value * 0.9f },
        )
        // Set like a keynote line: big, tight, the second half a step back. Two texts, so each half
        // comes up on its own turn, rising a little as it does.
        val tagline = MaterialTheme.typography.headlineMedium.copy(fontSize = 28.sp, lineHeight = 32.sp, letterSpacing = (-0.025).em, shadow = Soft)
        Column(
            Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(start = 20.dp, end = 20.dp, bottom = 122.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                "Your phone and your PC,", style = tagline, color = Color.White, textAlign = TextAlign.Center,
                modifier = Modifier.graphicsLayer { alpha = intro.line.value; translationY = (1f - intro.line.value) * 10.dp.toPx() },
            )
            Text(
                "working as one.", style = tagline, color = Color.White.copy(alpha = 0.72f), textAlign = TextAlign.Center,
                modifier = Modifier.graphicsLayer { alpha = intro.more.value; translationY = (1f - intro.more.value) * 10.dp.toPx() },
            )
        }
    }
}

/**
 * A drop of the real glass, to play with: it falls in and drifts across the name, sticks to a finger,
 * and thrown, glides, bounces off the edges and floats back (see LensBody). There's nothing to learn
 * here; the point is to feel what the whole app is made of.
 */
@Composable
private fun DragLens(backdrop: Backdrop, clock: ShowClock, onTouched: () -> Unit, modifier: Modifier) {
    BoxWithConstraints(modifier.fillMaxSize()) {
        val density = LocalDensity.current
        val lens = 168.dp
        val lensPx = with(density) { lens.toPx() }
        val home = Offset(constraints.maxWidth / 2f, constraints.maxHeight * 0.235f + lensPx / 2)
        val body = remember { LensBody(Offset(home.x, -lensPx)) }
        val haptics = LocalHapticFeedback.current
        LaunchedEffect(home) {
            var last = withFrameNanos { it }
            while (true) withFrameNanos { now ->
                val dt = ((now - last) / 1e9f).coerceIn(0f, 1f / 30f)
                last = now
                // Where it floats to when nobody holds it: a slow wander round the name. It's only a
                // pull, never a snap: let go, it carries on, glides, and drifts back here on its own.
                val wander = home + Offset(sin(clock.seconds * 0.55f) * lensPx * 0.62f, sin(clock.seconds * 0.9f) * lensPx * 0.09f)
                body.step(dt, wander, Size(constraints.maxWidth.toFloat(), constraints.maxHeight.toFloat()), lensPx / 2)
            }
        }
        // The glass, room round it for the blob to bulge into; its shape is drawn, never laid out again.
        val room = lens * 1.7f
        Box(
            Modifier
                .offset { IntOffset((body.at.x - room.toPx() / 2).roundToInt(), (body.at.y - room.toPx() / 2).roundToInt()) }
                .size(room)
                // The same glass as the rest of the app, without the blur: the point is to see through it.
                .glassChrome(backdrop, CircleShape, blur = 0.dp, bevel = 16.dp, blob = { body.shape(lensPx / 2) }),
        )
        // What the finger holds: a still circle over it, so a drag reads true however the glass is turned.
        Box(
            Modifier
                .offset { IntOffset((body.at.x - lensPx / 2).roundToInt(), (body.at.y - lensPx / 2).roundToInt()) }
                .size(lens)
                .semantics { contentDescription = "A glass lens you can drag" }
                .pointerInput(Unit) {
                    detectDragGestures(
                        onDragStart = { touch -> onTouched(); body.grab(touch - Offset(lensPx / 2, lensPx / 2)); haptics.perform(Haptic.Tap) },
                        onDragEnd = { body.release() },
                        onDragCancel = { body.release() },
                    ) { change, drag ->
                        change.consume()
                        body.finger += drag
                    }
                },
        )
    }
}

/**
 * The lens as a soft body, integrated each frame (in small steps, so a slow frame can't upset it).
 *
 * Its middle: held, it follows the finger on a soft spring, so it has weight; let go, it keeps its
 * speed, glides on air drag, bounces off the screen's edges, and a weak pull floats it back to its
 * wander, so nothing ever snaps.
 *
 * Its edge: [SPOKES] points round it, each how far out from its rest radius it is. Each is held to
 * that radius by a spring and to its neighbours by surface tension (which keeps the edge smooth and
 * makes broad bulges cheaper than ripples), damped, and kept to the same area. They feel the
 * middle's acceleration as the liquid in a drop does: it lags as the drop speeds up and surges on as
 * it stops, turns or hits an edge. And the finger, when it's further out than where it took hold,
 * pulls on the part of the edge facing it. All of it is a matter of where things are and which way
 * they go, so it behaves the same pulled in circles or anywhere at random as in a straight line.
 * Drawn as the edge's first four harmonics, smooth at any bend.
 */
@Stable
private class LensBody(start: Offset) {
    var at by mutableStateOf(start)
        private set
    // Falls in from above the screen.
    private var v = Offset(0f, 2600f)
    private var held = false
    var finger = start
    private var grip = Offset.Zero
    private var falling = true
    private val out = FloatArray(SPOKES)
    private val speed = FloatArray(SPOKES)
    private val cosA = FloatArray(SPOKES) { cos(2 * PI.toFloat() * it / SPOKES) }
    private val sinA = FloatArray(SPOKES) { sin(2 * PI.toFloat() * it / SPOKES) }
    private var drawn by mutableIntStateOf(0)

    /** A finger takes it at [touch] from its middle. */
    fun grab(touch: Offset) {
        finger = at + touch
        grip = touch
        held = true
    }

    fun release() {
        held = false
        val s = v.getDistance()
        if (s > 6000f) v *= 6000f / s
    }

    fun step(dt: Float, wander: Offset, area: Size, radius: Float) {
        if (dt <= 0f) return
        val n = 6
        val h = dt / n
        repeat(n) {
            val before = v
            if (held) {
                // Soft and a little under critical: it trails the finger, a drop with weight.
                v += ((finger - grip - at) * 160f - v * 20f) * h
            } else {
                // A slow, soft pull (about 2 s to come back) and air drag: it floats, it doesn't spring.
                v += ((wander - at) * 2.6f - v * 2.3f) * h
            }
            at += v * h
            if (!held) bounce(area, radius)
            spokes(h, v - before, radius)
        }
        drawn++
    }

    private fun bounce(area: Size, radius: Float) {
        var x = at.x
        var y = at.y
        if (x < radius && v.x < 0f) { x = radius; v = Offset(-v.x * 0.45f, v.y) }
        if (x > area.width - radius && v.x > 0f) { x = area.width - radius; v = Offset(-v.x * 0.45f, v.y) }
        if (y > area.height - radius && v.y > 0f) { y = area.height - radius; v = Offset(v.x, -v.y * 0.45f) }
        if (y < radius && v.y < 0f && !falling) { y = radius; v = Offset(v.x, -v.y * 0.45f) }
        at = Offset(x, y)
        if (falling && at.y > radius) falling = false
    }

    /** [kick] is how much the middle's speed changed this step. */
    private fun spokes(h: Float, kick: Offset, radius: Float) {
        // A bounce is one huge kick: felt in full it tore the edge; capped, it's a splash.
        val dv = kick.getDistance().let { if (it > 2500f) kick * (2500f / it) else kick }
        // How far the finger has run ahead of where it took hold, and the side of the edge facing it.
        val pull = if (held) (finger - grip - at).getDistance() else 0f
        val toFinger = finger - at
        val reach = toFinger.getDistance()
        val fx = if (reach > 1f) toFinger.x / reach else 0f
        val fy = if (reach > 1f) toFinger.y / reach else 0f
        for (i in 0 until SPOKES) {
            val prev = out[(i + SPOKES - 1) % SPOKES]
            val next = out[(i + 1) % SPOKES]
            // Viscosity damps ripples far more than broad bulges, so the edge stays smooth.
            val flow = speed[(i + SPOKES - 1) % SPOKES] + speed[(i + 1) % SPOKES] - 2 * speed[i]
            var f = -RADIAL * out[i] + TENSION * (prev + next - 2 * out[i]) - DAMPING * speed[i] + VISCOSITY * flow
            // The liquid lags the drop's acceleration: the side it's speeding away from swells.
            speed[i] -= (dv.x * cosA[i] + dv.y * sinA[i]) * SLOSH
            // The finger draws the drop out toward it: most on the side facing it, fading smoothly
            // round to none on the far side, so it becomes a teardrop, not a lump.
            if (pull > 0f) {
                val facing = (1f + fx * cosA[i] + fy * sinA[i]) / 2f
                val w = facing * facing
                f += GRIP * (pull * 0.6f * w * facing - out[i]) * w
            }
            speed[i] += f * h
        }
        var mean = 0f
        for (i in 0 until SPOKES) {
            out[i] += speed[i] * h
            mean += out[i]
        }
        // Same area: what one side gains, the rest gives up.
        mean /= SPOKES
        for (i in 0 until SPOKES) out[i] = (out[i] - mean).coerceIn(-radius * 0.38f, radius * 0.6f)
    }

    /** Its shape now, for a lens [radius] across at rest; read while drawing, so the glass redraws each step. */
    fun shape(radius: Float): Blob {
        drawn
        fun wave(k: Int): Offset {
            var a = 0f
            var b = 0f
            for (i in 0 until SPOKES) {
                val ang = 2 * PI.toFloat() * k * i / SPOKES
                a += out[i] * cos(ang)
                b += out[i] * sin(ang)
            }
            return Offset(a, b) * (2f / SPOKES / radius)
        }
        val w1 = wave(1)
        var w2 = wave(2)
        var w3 = wave(3)
        var w4 = wave(4)
        // How sharply the edge bends: past this, it would dent inward somewhere. Scaled back, it never does.
        val bend = 4 * w2.getDistance() + 9 * w3.getDistance() + 16 * w4.getDistance()
        if (bend > 0.7f) {
            val k = 0.7f / bend
            w2 *= k; w3 *= k; w4 *= k
        }
        return Blob(radius, w1, w2, w3, w4)
    }

    private companion object {
        const val SPOKES = 32
        // Per second squared: the edge's pull back to round, its surface tension, its damping and
        // viscosity, how much it sloshes, and how hard a finger draws it out.
        const val RADIAL = 120f
        const val TENSION = 260f
        const val DAMPING = 5f
        const val VISCOSITY = 40f
        const val SLOSH = 0.25f
        const val GRIP = 260f
    }
}

/** The dots, with one small drop that stretches from dot to dot as the page moves. */
@Composable
private fun PageDots(count: Int, position: () -> Float) {
    val gap = 18.dp
    Canvas(Modifier.width(gap * (count - 1) + 16.dp).height(16.dp)) {
        val step = gap.toPx()
        val r = 3.5.dp.toPx()
        val y = size.height / 2
        val at = position()
        for (i in 0 until count) drawCircle(Color.White.copy(alpha = 0.42f), r, Offset(8.dp.toPx() + i * step, y))
        val into = abs(at - floor(at + 0.5f)) * 2 // 0 on a dot, 1 midway
        val stretch = into * step * 0.5f
        val x = 8.dp.toPx() + at * step
        drawRoundRect(Color.White, Offset(x - r - stretch, y - r * 1.15f), Size((r + stretch) * 2, r * 2.3f), CornerRadius(r * 1.15f))
    }
}

/** The show's buttons: clear glass with white words. */
@Composable
internal fun GlassPill(text: String, backdrop: Backdrop, modifier: Modifier = Modifier, large: Boolean = false, onClick: () -> Unit) {
    Box(
        modifier
            .height(if (large) 60.dp else 40.dp)
            .semantics { role = Role.Button }
            .glassChrome(backdrop, CircleShape, blur = 1.5.dp, bevel = 16.dp, interactive = true)
            .liquidPress(pressedScale = 1f, onClick = onClick)
            .padding(horizontal = 20.dp),
        contentAlignment = Alignment.Center,
    ) {
        AnimatedContent(text, label = "pill") { t ->
            Text(t, style = (if (large) MaterialTheme.typography.titleMedium else MaterialTheme.typography.labelLarge).copy(shadow = Soft), color = Color.White, maxLines = 1)
        }
    }
}

/** A scene's words, on the same clear glass as everything else here. */
@Composable
private fun WordsPanel(backdrop: Backdrop, title: String, body: String) {
    Column(
        Modifier.fillMaxWidth()
            .glassChrome(backdrop, RoundedCornerShape(30.dp), blur = 3.dp, bevel = 16.dp)
            .padding(horizontal = 24.dp, vertical = 20.dp),
    ) {
        AnimatedContent(title to body, label = "words") { (t, b) ->
            Column {
                Text(t, style = MaterialTheme.typography.titleLarge.copy(shadow = Soft), color = Color.White)
                Spacer(Modifier.height(6.dp))
                Text(b, style = MaterialTheme.typography.bodyMedium.copy(shadow = Soft), color = Color.White)
            }
        }
    }
}

// ---- The demos. Everything moving is drawn from the clock; every pane is the real glass.

/** A laptop whose desktop is alive, and the PC's live numbers under it. */
@Composable
private fun ScreenDemo(backdrop: Backdrop, clock: ShowClock) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
        Box(Modifier.fillMaxWidth(0.92f).aspectRatio(1.56f).glassChrome(backdrop, RoundedCornerShape(22.dp), blur = 1.dp, bevel = 16.dp)) {
            Canvas(Modifier.fillMaxSize().padding(18.dp)) { drawDesktop(clock.seconds) }
        }
        Box(Modifier.fillMaxWidth().height(14.dp).glassChrome(backdrop, CircleShape, blur = 1.dp, bevel = 7.dp))
        Spacer(Modifier.height(22.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            val cpu by remember { derivedStateOf { (24 + 14 * sin(clock.seconds * 0.8f)).roundToInt() } }
            val gpu by remember { derivedStateOf { (38 + 22 * sin(clock.seconds * 0.5f + 2f)).roundToInt() } }
            val temp by remember { derivedStateOf { (52 + 5 * sin(clock.seconds * 0.3f)).roundToInt() } }
            Stat(backdrop, "CPU", "$cpu%", Modifier.weight(1f))
            Stat(backdrop, "GPU", "$gpu%", Modifier.weight(1f))
            Stat(backdrop, "Temp", "$temp°", Modifier.weight(1f))
        }
    }
}

@Composable
private fun Stat(backdrop: Backdrop, label: String, value: String, modifier: Modifier) {
    Column(
        modifier.glassChrome(backdrop, RoundedCornerShape(20.dp), blur = 1.5.dp, bevel = 12.dp).padding(vertical = 12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(value, style = MaterialTheme.typography.titleLarge.copy(fontFeatureSettings = "tnum", shadow = Soft), color = Color.White)
        Text(label, style = MaterialTheme.typography.labelMedium.copy(shadow = Soft), color = Color.White.copy(alpha = 0.9f))
    }
}

private fun DrawScope.drawDesktop(t: Float) {
    val w = size.width
    val h = size.height
    val pane = Color.White.copy(alpha = 0.26f)
    val line = Color.White.copy(alpha = 0.72f)
    fun win(x: Float, y: Float, ww: Float, wh: Float) = drawRoundRect(pane, Offset(x * w, y * h), Size(ww * w, wh * h), CornerRadius(8.dp.toPx()))
    win(0f, 0f, 0.58f, 0.72f)
    win(0.5f, 0.3f, 0.5f, 0.6f)
    for (i in 0..3) drawRoundRect(line, Offset(0.05f * w, (0.11f + i * 0.13f) * h), Size((0.44f - 0.07f * (i % 3)) * w, 4.dp.toPx()), CornerRadius(2.dp.toPx()))
    // A chart in the front window, running.
    val chart = Path()
    for (i in 0..24) {
        val x = (0.54f + i / 24f * 0.42f) * w
        val y = (0.66f + 0.13f * sin(i * 0.55f + t * 1.4f) * cos(i * 0.21f)) * h
        if (i == 0) chart.moveTo(x, y) else chart.lineTo(x, y)
    }
    drawPath(chart, Color.White, style = Stroke(2.dp.toPx()))
    // The cursor goes between things and clicks on them.
    val q = t % 5f
    val spots = listOf(Offset(0.2f, 0.2f), Offset(0.74f, 0.5f), Offset(0.4f, 0.82f))
    val i = floor(q / (5f / 3)).toInt().coerceIn(0, 2)
    val since = q - i * (5f / 3)
    val s = settle(since * 1.1f).coerceIn(0f, 1.08f)
    val a = spots[(i + 2) % 3]
    val b = spots[i]
    val c = Offset((a.x + (b.x - a.x) * s) * w, (a.y + (b.y - a.y) * s) * h)
    val click = (since - 1.05f).coerceIn(0f, 0.6f) / 0.6f
    if (click > 0f) drawCircle(Color.White.copy(alpha = (1f - click) * 0.7f), 4.dp.toPx() + click * 20.dp.toPx(), c, style = Stroke(1.5.dp.toPx()))
    val u = 1.dp.toPx()
    val arrow = Path().apply {
        moveTo(c.x, c.y); lineTo(c.x, c.y + 17 * u); lineTo(c.x + 4.4f * u, c.y + 13 * u)
        lineTo(c.x + 7.4f * u, c.y + 19 * u); lineTo(c.x + 10 * u, c.y + 18 * u); lineTo(c.x + 7.2f * u, c.y + 12 * u); lineTo(c.x + 12.6f * u, c.y + 12 * u); close()
    }
    drawPath(arrow, Color.White)
    drawPath(arrow, Color.Black.copy(alpha = 0.55f), style = Stroke(1 * u))
}

/** The phone as three different things, chosen in the same sliding glass the tab bar uses. */
@Composable
private fun ControlsDemo(backdrop: Backdrop, clock: ShowClock, mode: Int, onMode: (Int) -> Unit) {
    var touched by remember { mutableStateOf(false) }
    LaunchedEffect(touched) {
        while (!touched) {
            delay(4200)
            if (!touched) onMode((mode + 1) % CONTROL_MODES.size)
        }
    }
    val current = mode
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
        Box(Modifier.fillMaxWidth().aspectRatio(1.32f).glassChrome(backdrop, RoundedCornerShape(30.dp), blur = 1.dp, bevel = 18.dp)) {
            val shown0 by animateFloatAsState(if (current == 0) 1f else 0f, VyomaMotion.effects(), label = "pad")
            val shown1 by animateFloatAsState(if (current == 1) 1f else 0f, VyomaMotion.effects(), label = "pad")
            val shown2 by animateFloatAsState(if (current == 2) 1f else 0f, VyomaMotion.effects(), label = "pad")
            Canvas(Modifier.fillMaxSize().padding(22.dp)) {
                if (shown0 > 0.01f) drawTouchpad(clock.seconds, shown0)
                if (shown1 > 0.01f) drawController(clock.seconds, shown1)
                if (shown2 > 0.01f) drawViewfinder(clock.seconds, shown2)
            }
        }
        Spacer(Modifier.height(16.dp))
        // Sliding selector: a lit capsule that glides and stretches between the three.
        BoxWithConstraints(Modifier.fillMaxWidth(0.76f).height(60.dp).glassChrome(backdrop, CircleShape, blur = 1.5.dp, bevel = 16.dp)) {
            val share = maxWidth / CONTROL_MODES.size
            val lead by animateDpAsState(share * current, VyomaMotion.leading(), label = "lead")
            val trail by animateDpAsState(share * current, VyomaMotion.trailing(), label = "trail")
            val from = min(lead.value, trail.value).dp
            val to = max(lead.value, trail.value).dp
            Box(Modifier.offset(x = from + 5.dp, y = 5.dp).width(share + (to - from) - 10.dp).height(50.dp).background(Color.White.copy(alpha = 0.24f), CircleShape))
            Row(Modifier.fillMaxSize()) {
                CONTROL_MODES.forEachIndexed { i, m ->
                    Box(
                        Modifier.weight(1f).fillMaxSize().pressable(haptic = Haptic.Select) { touched = true; onMode(i) }
                            .semantics { contentDescription = m.second; role = Role.Tab },
                        contentAlignment = Alignment.Center,
                    ) { Icon(m.first, null, tint = Color.White) }
                }
            }
        }
    }
}

private fun DrawScope.drawTouchpad(t: Float, shown: Float) {
    val w = size.width
    val h = size.height
    val c = Color.White.copy(alpha = shown)
    // The finger's path, fading behind it.
    fun at(s: Float) = Offset(w * (0.5f + 0.34f * sin(s * 1.1f)), h * (0.5f + 0.3f * sin(s * 1.7f + 1f)))
    for (i in 0 until 26) {
        drawCircle(c.copy(alpha = shown * (1f - i / 26f) * 0.55f), (1f - i / 26f) * 7.dp.toPx(), at(t - i * 0.028f))
    }
    val f = at(t)
    drawCircle(c.copy(alpha = shown * 0.28f), 26.dp.toPx(), f)
    drawCircle(c, 26.dp.toPx(), f, style = Stroke(2.dp.toPx()))
    // Left and right buttons, along the bottom.
    val by = h - 8.dp.toPx()
    drawLine(c.copy(alpha = shown * 0.8f), Offset(w * 0.32f, by), Offset(w * 0.47f, by), 4.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)
    drawLine(c.copy(alpha = shown * 0.8f), Offset(w * 0.53f, by), Offset(w * 0.68f, by), 4.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)
}

private fun DrawScope.drawController(t: Float, shown: Float) {
    val w = size.width
    val h = size.height
    val c = Color.White.copy(alpha = shown)
    val cy = h * 0.52f
    val u = min(w, h) / 10f
    // The shape of one: a wide body with its two grips.
    val outline = Path().apply {
        addRoundRect(androidx.compose.ui.geometry.RoundRect(w * 0.03f, h * 0.2f, w * 0.97f, h * 0.84f, CornerRadius(h * 0.3f)))
    }
    drawPath(outline, c.copy(alpha = shown * 0.55f), style = Stroke(2.dp.toPx()))
    // Left stick wandering.
    val ls = Offset(w * 0.23f, cy)
    drawCircle(c.copy(alpha = shown * 0.3f), u * 1.8f, ls, style = Stroke(2.dp.toPx()))
    drawCircle(c, u * 0.95f, ls + Offset(sin(t * 1.4f) * u * 0.75f, cos(t * 1.9f) * u * 0.75f))
    // Right: four buttons, one lit at a time, popping as they're pressed.
    val rs = Offset(w * 0.77f, cy)
    val active = floor(t * 1.6f).toInt() % 4
    val press = (t * 1.6f - floor(t * 1.6f))
    listOf(Offset(0f, -1.45f), Offset(1.45f, 0f), Offset(0f, 1.45f), Offset(-1.45f, 0f)).forEachIndexed { i, d ->
        val on = i == active
        val r = u * (0.62f * if (on) 0.82f + 0.18f * settle(press) else 1f)
        drawCircle(c.copy(alpha = shown * if (on) 1f else 0.3f), r, rs + d * u)
    }
    // Triggers, up top, pulling in turn.
    val pull = (sin(t * 2.3f) + 1f) / 2
    drawLine(c.copy(alpha = shown * (0.4f + 0.6f * pull)), Offset(w * 0.1f, u * 0.5f), Offset(w * 0.1f + w * 0.22f * pull, u * 0.5f), 5.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)
    drawLine(c.copy(alpha = shown * (1f - 0.6f * pull)), Offset(w * 0.9f, u * 0.5f), Offset(w * 0.9f - w * 0.22f * (1f - pull), u * 0.5f), 5.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)
    // Centre buttons.
    drawCircle(c.copy(alpha = shown * 0.5f), u * 0.34f, Offset(w * 0.44f, cy))
    drawCircle(c.copy(alpha = shown * 0.5f), u * 0.34f, Offset(w * 0.56f, cy))
}

private fun DrawScope.drawViewfinder(t: Float, shown: Float) {
    val w = size.width
    val h = size.height
    val c = Color.White.copy(alpha = shown)
    val len = min(w, h) * 0.16f
    val sw = 3.dp.toPx()
    // The corners breathe a little, focusing.
    val inset = 3.dp.toPx() * (1f + sin(t * 1.6f))
    for ((x, y, dx, dy) in listOf(
        listOf(inset, inset, 1f, 1f), listOf(w - inset, inset, -1f, 1f),
        listOf(inset, h - inset, 1f, -1f), listOf(w - inset, h - inset, -1f, -1f),
    )) {
        drawLine(c, Offset(x, y), Offset(x + dx * len, y), sw, cap = androidx.compose.ui.graphics.StrokeCap.Round)
        drawLine(c, Offset(x, y), Offset(x, y + dy * len), sw, cap = androidx.compose.ui.graphics.StrokeCap.Round)
    }
    // Someone on a call.
    val cx = w / 2 + sin(t * 0.6f) * 5.dp.toPx()
    val r = h * 0.15f
    drawCircle(c.copy(alpha = shown * 0.9f), r, Offset(cx, h * 0.38f))
    drawArc(c.copy(alpha = shown * 0.9f), 180f, 180f, true, Offset(cx - r * 2f, h * 0.62f), Size(r * 4f, r * 2.8f))
    // On air.
    val pulse = (sin(t * 3.2f) + 1f) / 2
    drawCircle(Color(0xFFFF453A).copy(alpha = shown * (0.5f + 0.5f * pulse)), 6.dp.toPx(), Offset(w - inset - 18.dp.toPx(), inset + 18.dp.toPx()))
}

private class Thrown(val icon: ImageVector, val label: String)

private val THROWN = listOf(
    Thrown(Icons.Rounded.ContentPaste, "Copied text"),
    Thrown(Icons.Rounded.Photo, "Beach.jpg"),
    Thrown(Icons.Rounded.Description, "Report.pdf"),
    Thrown(Icons.Rounded.Folder, "Holiday"),
)

/** Things crossing between a phone and a PC, in a drop of glass, there and back. */
@Composable
private fun FlowDemo(backdrop: Backdrop, clock: ShowClock) {
    val turn = 2.5f
    val index by remember { derivedStateOf { floor(clock.seconds / turn).toInt() } }
    BoxWithConstraints(Modifier.fillMaxWidth().aspectRatio(0.98f)) {
        val density = LocalDensity.current
        val wPx = constraints.maxWidth.toFloat()
        val hPx = constraints.maxHeight.toFloat()
        val phone = 0.29f
        // A phone at left, a screen at right; the chip travels from one's middle to the other's.
        Box(Modifier.align(Alignment.CenterStart).fillMaxWidth(phone).aspectRatio(0.52f).glassChrome(backdrop, RoundedCornerShape(24.dp), blur = 1.dp, bevel = 13.dp), contentAlignment = Alignment.Center) {
            Icon(Icons.Rounded.Smartphone, null, tint = Color.White, modifier = Modifier.size(28.dp))
        }
        Box(Modifier.align(Alignment.CenterEnd).fillMaxWidth(0.56f).aspectRatio(1.28f).glassChrome(backdrop, RoundedCornerShape(18.dp), blur = 1.dp, bevel = 13.dp)) {
            Canvas(Modifier.fillMaxSize().padding(16.dp)) {
                drawRoundRect(Color.White.copy(alpha = 0.2f), Offset.Zero, size, CornerRadius(8.dp.toPx()))
            }
            Icon(Icons.Rounded.Computer, null, tint = Color.White, modifier = Modifier.align(Alignment.Center).size(32.dp))
        }
        // Where it lands ripples, on the glass over there.
        Canvas(Modifier.fillMaxSize()) {
            val t = clock.seconds % turn
            val toPc = floor(clock.seconds / turn).toInt() % 2 == 0
            val land = (t - 1.25f).coerceIn(0f, 1f)
            if (land > 0f) {
                val c = if (toPc) Offset(size.width * 0.72f, size.height / 2) else Offset(size.width * 0.145f, size.height / 2)
                drawCircle(Color.White.copy(alpha = (1f - land) * 0.8f), 16.dp.toPx() + land * 48.dp.toPx(), c, style = Stroke(2.dp.toPx()))
            }
        }
        val item = THROWN[index % THROWN.size]
        val chipW = with(density) { 136.dp.toPx() }
        val chipH = with(density) { 48.dp.toPx() }
        Row(
            Modifier
                .offset {
                    val t = clock.seconds % turn
                    val toPc = floor(clock.seconds / turn).toInt() % 2 == 0
                    val s = settle((t - 0.2f) * 0.85f).coerceAtLeast(0f)
                    val (a, b) = if (toPc) 0.145f to 0.72f else 0.72f to 0.145f
                    val x = (a + (b - a) * s) * wPx - chipW / 2
                    val y = hPx / 2 - chipH / 2 - sin(s.coerceIn(0f, 1f) * PI.toFloat()) * hPx * 0.3f
                    IntOffset(x.roundToInt(), y.roundToInt())
                }
                .glassChrome(backdrop, CircleShape, blur = 2.dp, bevel = 14.dp)
                .height(48.dp).width(136.dp).padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(item.icon, null, tint = Color.White, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(8.dp))
            Text(item.label, style = MaterialTheme.typography.labelMedium.copy(shadow = Soft), color = Color.White, maxLines = 1)
        }
    }
}

private class RuleDemo(val icon: ImageVector, val trigger: String, val action: String)

// Real rules: every one of these triggers and actions exists in Automation.
private val RULE_DEMOS = listOf(
    RuleDemo(Icons.Rounded.Phone, "A call comes in", "PC's music pauses"),
    RuleDemo(Icons.Rounded.WifiOff, "You leave home", "PC locks itself"),
    RuleDemo(Icons.Rounded.NightsStay, "It's 11 at night", "PC mutes and dims"),
)

@Composable
private fun RulesDemo(backdrop: Backdrop, clock: ShowClock) {
    val active by remember { derivedStateOf { floor(clock.seconds / 2.4f).toInt() % RULE_DEMOS.size } }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        RULE_DEMOS.forEachIndexed { i, r ->
            val on = i == active
            val pop by animateFloatAsState(if (on) 1f else 0f, VyomaMotion.release(), label = "rule")
            val tick by animateFloatAsState(if (on) 1f else 0f, spring(stiffness = Spring.StiffnessLow), label = "tick")
            Row(
                Modifier.fillMaxWidth()
                    .graphicsLayer { val s = 0.95f + 0.05f * pop; scaleX = s; scaleY = s }
                    .glassChrome(backdrop, RoundedCornerShape(24.dp), blur = 3.dp, bevel = 15.dp)
                    .padding(horizontal = 18.dp, vertical = 15.dp)
                    // The words fade, not the pane: a layer with alpha clips the glass's shadow to a rectangle.
                    .graphicsLayer { alpha = 0.78f + 0.22f * tick },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(r.icon, null, tint = Color.White, modifier = Modifier.size(24.dp))
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    Text(r.trigger, style = MaterialTheme.typography.titleSmall.copy(shadow = Soft), color = Color.White)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.AutoMirrored.Rounded.TrendingUp, null, tint = Color.White.copy(alpha = 0.7f), modifier = Modifier.size(14.dp))
                        Spacer(Modifier.width(4.dp))
                        Text(r.action, style = MaterialTheme.typography.bodySmall.copy(shadow = Soft), color = Color.White)
                    }
                }
                Icon(Icons.Rounded.Check, null, tint = Color.White, modifier = Modifier.size(22.dp).graphicsLayer { alpha = tick; scaleX = tick; scaleY = tick })
            }
        }
    }
}

/** A lock under a clear dome, and the same code on both sides, digit by digit. */
@Composable
private fun PrivateDemo(backdrop: Backdrop, clock: ShowClock) {
    val code = "482916"
    val shownDigits by remember { derivedStateOf { min(code.length, (((clock.seconds % 7f) - 0.3f) * 3.2f).toInt().coerceAtLeast(0)) } }
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
        Box(contentAlignment = Alignment.Center) {
            Canvas(Modifier.size(208.dp)) {
                val c = (clock.seconds * 0.45f) % 1f
                drawCircle(Color.White.copy(alpha = (1f - c) * 0.42f), size.minDimension * (0.34f + 0.16f * c), style = Stroke(1.5.dp.toPx()))
            }
            Box(Modifier.size(136.dp).glassChrome(backdrop, CircleShape, blur = 0.dp, bevel = 68.dp, clear = true), contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.Lock, null, tint = Color.White, modifier = Modifier.size(44.dp))
            }
        }
        Spacer(Modifier.height(10.dp))
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            CodeChip(backdrop, "Phone", code, shownDigits)
            val matched = shownDigits == code.length
            val tick by animateFloatAsState(if (matched) 1f else 0f, VyomaMotion.release(), label = "match")
            Icon(Icons.Rounded.Check, if (matched) "Matches" else null, tint = Color.White, modifier = Modifier.size(24.dp).graphicsLayer { alpha = tick.coerceIn(0f, 1f); scaleX = tick; scaleY = tick })
            CodeChip(backdrop, "PC", code, shownDigits)
        }
    }
}

@Composable
private fun CodeChip(backdrop: Backdrop, label: String, code: String, shown: Int) {
    Column(
        Modifier.width(126.dp).glassChrome(backdrop, RoundedCornerShape(20.dp), blur = 2.dp, bevel = 13.dp).padding(vertical = 12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(label, style = MaterialTheme.typography.labelMedium.copy(shadow = Soft), color = Color.White.copy(alpha = 0.9f))
        Text(
            code.take(shown).padEnd(code.length, '·'),
            style = MaterialTheme.typography.titleMedium.copy(fontFeatureSettings = "tnum", letterSpacing = 1.5.sp, shadow = Soft),
            color = Color.White,
            maxLines = 1,
        )
    }
}

/**
 * The moment a PC pairs: a ring runs through the whole picture, a drop of glass lands with a tick in
 * it, and the sun's already coming up behind it (the caller moves the day). Taps through; goes by itself.
 */
@Composable
internal fun PairedMoment(backdrop: Backdrop, ripple: Ripple, name: String, onDone: () -> Unit) {
    val haptics = LocalHapticFeedback.current
    val grow = remember { Animatable(0f) }
    val fade = remember { Animatable(0f) }
    BoxWithConstraints(Modifier.fillMaxSize().pointerInput(Unit) { awaitPointerEventScope { awaitPointerEvent(); onDone() } }, contentAlignment = Alignment.Center) {
        val far = hypot(constraints.maxWidth.toFloat(), constraints.maxHeight.toFloat())
        LaunchedEffect(Unit) {
            haptics.perform(Haptic.Success)
            launch { grow.animateTo(1f, VyomaMotion.jelly()) }
            launch { fade.animateTo(1f, VyomaMotion.effects()) }
            launch {
                ripple.strength.snapTo(46f)
                ripple.strength.animateTo(0f, spring(dampingRatio = 1f, stiffness = 28f))
            }
            ripple.radius.snapTo(0f)
            launch { ripple.radius.animateTo(far, spring(dampingRatio = 1f, stiffness = 18f)) }
            delay(2600)
            onDone()
        }
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Box(
                Modifier.size(150.dp)
                    .graphicsLayer { scaleX = grow.value; scaleY = grow.value }
                    .glassChrome(backdrop, CircleShape, blur = 0.dp, bevel = 75.dp, clear = true),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.Check, null, tint = Color.White, modifier = Modifier.size(64.dp))
            }
            Spacer(Modifier.height(24.dp))
            Column(Modifier.graphicsLayer { alpha = fade.value; translationY = (1f - fade.value) * 18.dp.toPx() }, horizontalAlignment = Alignment.CenterHorizontally) {
                Text("You're connected", style = MaterialTheme.typography.headlineSmall.copy(shadow = Soft), color = Color.White)
                Text(name, style = MaterialTheme.typography.titleMedium.copy(shadow = Soft), color = Color.White.copy(alpha = 0.9f))
            }
        }
    }
}
