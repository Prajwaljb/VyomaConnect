package dev.vyoma.connect.designsystem

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.view.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlin.math.abs
import kotlin.math.atan2
import kotlin.math.hypot
import kotlin.math.tanh

/**
 * Where the light comes from, as seen on the screen: the angle the specular edges are brightest
 * at, in degrees clockwise from 3 o'clock (270 = straight up). At rest it's above the brightest part
 * of the wallpaper behind the app (see [CanvasWallpaper.lightAngle]), straight up without one; tilting the phone sways it up to 45° either way (55° at a 2.2x gain was too twitchy), so a small turn carries
 * it to straight up (a 35° soft limit made the top centre take a deliberate tilt), without circling. Read it only
 * while drawing: a change then redraws the highlights, never recomposes or relayouts.
 */
object SpecularLight {
    const val REST = 270f

    var angle by mutableFloatStateOf(REST)
        private set

    /**
     * A light at a place on screen (the first run's sun). While set, each glass element is lit from
     * wherever it is, in the light's colour, instead of all from [angle]. Its lambdas are read while
     * drawing, so what they read redraws the glass and nothing else.
     */
    var source: Source? by mutableStateOf(null)

    class Source(
        val at: () -> androidx.compose.ui.geometry.Offset,
        val colour: () -> androidx.compose.ui.graphics.Color,
        /** How far it is across the whole picture, in px: what it fades over. */
        val across: () -> Float,
    )

    /** The way to the light from [center], in root coordinates. */
    fun from(center: androidx.compose.ui.geometry.Offset): Float {
        val s = source ?: return angle
        val to = s.at() - center
        return Math.toDegrees(kotlin.math.atan2(to.y, to.x).toDouble()).toFloat()
    }

    /** How much of it gets to [center]: all beside it, down to a little under half across the picture. */
    fun reach(center: androidx.compose.ui.geometry.Offset): Float {
        val s = source ?: return 1f
        return (1f - (s.at() - center).getDistance() / s.across().coerceAtLeast(1f) * 0.8f).coerceIn(0.45f, 1f)
    }

    /** What colour the highlights are. */
    val colour: androidx.compose.ui.graphics.Color get() = source?.colour?.invoke() ?: androidx.compose.ui.graphics.Color.White

    /**
     * How bright the edge across from the light catches it. A real source lights one side more
     * clearly, so that you can tell where it is.
     */
    val opposite: Float get() = if (source != null) 0.45f else 0.8f

    /** Where the light rests before any tilt. */
    internal var rest = REST
        private set

    /** Moves the resting light, and the light with it. */
    internal fun aim(at: Float) {
        angle += at - rest
        rest = at
    }

    /**
     * The light as content (cards) sees it: the same as the floating chrome. At 40% of it the
     * cards felt dead and the buttons too lively, as if two lights (user, 2026-11-05).
     */
    val content: Float get() = angle

    /**
     * Where a shadow [by] px long falls: straight away from the light, so straight down at rest and
     * sliding sideways as the phone tilts (up to 45 deg, where it's as far across as down). Pass
     * [content] for things on the page, so their shadows sway as little as their light.
     */
    fun cast(by: Float, light: Float = angle): androidx.compose.ui.geometry.Offset {
        val a = Math.toRadians(light.toDouble())
        return androidx.compose.ui.geometry.Offset(-kotlin.math.cos(a).toFloat() * by, -kotlin.math.sin(a).toFloat() * by)
    }

    /** Back to where it rests, untilted. */
    internal fun settle() {
        angle = rest
    }

    internal fun publish(a: Float) {
        // Hand tremor is a degree or two: below that nothing is redrawn.
        if (abs(a - angle) >= 1.5f) angle = a
    }
}

/**
 * Follows the phone's tilt into [SpecularLight] while the UI is started (visible); the sensor is off
 * otherwise. Gravity is fused and cheap; the game rate keeps the highlight's travel smooth, and a
 * low-pass filter plus [SpecularLight]'s dead band keeps a still phone from redrawing.
 *
 * Not with Lighter glass: a hand is never still enough, and each sway redraws every lit thing on
 * screen. A moto g52 drew Settings non-stop, each frame ~86 ms (2026-11-07). The light rests instead.
 */
@Composable
internal fun TrackSpecularLight() {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    // Nor under a light in the picture (the first run's sun): a sway there felt jarring (user, 2026-11-08).
    val lite = GlassQuality.lite || SpecularLight.source != null
    DisposableEffect(context, owner, lite) {
        if (lite) {
            SpecularLight.settle()
            return@DisposableEffect onDispose { }
        }
        val sensors = context.getSystemService(Context.SENSOR_SERVICE) as SensorManager
        val sensor = sensors.getDefaultSensor(Sensor.TYPE_GRAVITY) ?: sensors.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)
        val listener = TiltListener(context)
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_START -> sensor?.let {
                    listener.reset()
                    sensors.registerListener(listener, it, SensorManager.SENSOR_DELAY_GAME)
                }
                Lifecycle.Event.ON_STOP -> sensors.unregisterListener(listener)
                else -> Unit
            }
        }
        owner.lifecycle.addObserver(observer)
        onDispose {
            owner.lifecycle.removeObserver(observer)
            sensors.unregisterListener(listener)
        }
    }
}

private class TiltListener(private val context: Context) : SensorEventListener {
    private var x = 0f
    private var y = SensorManager.GRAVITY_EARTH
    /** Samples since the sensor started; the first few set the filter instead of easing in. */
    private var seen = 0

    fun reset() {
        seen = 0
    }

    override fun onSensorChanged(e: SensorEvent) {
        // A fresh registration can hand over a zero or stale sample first: only real gravity counts.
        if (hypot(hypot(e.values[0], e.values[1]), e.values[2]) < SensorManager.GRAVITY_EARTH * 0.5f) return
        val warming = seen < WARMUP
        seen++
        if (warming) {
            x = e.values[0]; y = e.values[1]
        } else {
            x += (e.values[0] - x) * SMOOTH
            y += (e.values[1] - y) * SMOOTH
        }
        // Into screen axes (x right, y down) for the current rotation.
        val (sx, sy) = when (context.display?.rotation) {
            Surface.ROTATION_90 -> -y to -x
            Surface.ROTATION_180 -> -x to y
            Surface.ROTATION_270 -> y to x
            else -> x to -y
        }
        // Gravity's sensor reading points up: that's where up in the room lies on the screen.
        val up = Math.toDegrees(atan2(sy, sx).toDouble()).toFloat()
        var turn = up + 90f // 0 when held upright
        if (turn > 180f) turn -= 360f
        if (turn < -180f) turn += 360f
        // Lying flat there's no "up" on the screen: settle back to the resting light.
        // Phones are held tilted back: full sway from about 15° off flat.
        val upright = ((hypot(sx, sy) / SensorManager.GRAVITY_EARTH - 0.08f) / 0.18f).coerceIn(0f, 1f)
        // The light stays put, straight up: tilting only sways it, and a
        // bigger turn eases into the limit instead of carrying it round the element.
        SpecularLight.publish(SpecularLight.rest + MAX_SWAY * tanh(turn * GAIN / MAX_SWAY) * upright)
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit

    private companion object {
        const val SMOOTH = 0.2f
        const val WARMUP = 8
        /** Calmer than the room's real up: 1.7 swung buttons too much (user, 2026-11-05); 0.7 for
         *  everything, cards included (they were at 0.4 of the buttons, 1.0). */
        const val GAIN = 0.7f
        /** How far the light can sway from rest, either way. */
        const val MAX_SWAY = 45f
    }
}
