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
 * at, in degrees clockwise from 3 o'clock (270 = straight up). At rest it's straight up, centred
 * (the user asked for it there, not up-left); tilting the phone sways it up to 45° either way (55° at a 2.2x gain was too twitchy), so a small turn carries
 * it to straight up (a 35° soft limit made the top centre take a deliberate tilt), without circling. Read it only
 * while drawing: a change then redraws the highlights, never recomposes or relayouts.
 */
object SpecularLight {
    const val REST = 270f

    var angle by mutableFloatStateOf(REST)
        private set

    /**
     * The light as content (cards) sees it: it sways 40% as far as on the floating chrome, so a
     * tilt moves the layer above more than the page and two dozen card edges don't all shimmer.
     */
    val content: Float get() = REST + (angle - REST) * 0.4f

    /**
     * Where a shadow falls. Apple's glass is lit from the front: a floating button's shadow is an even
     * halo (iOS Music's search button: the same depth to the side as below), leaning only [drop] down.
     * Tilting swings the light, and the shadow slides the other way, up to [sway] (the offset turns
     * with the light, so at the 45 deg limit it's 0.71 sway across and 0.29 up). Pass [content] as
     * [light] for things on the page, so their shadows sway as little as their light.
     */
    fun cast(drop: Float, sway: Float, light: Float = angle): androidx.compose.ui.geometry.Offset {
        val a = Math.toRadians(light.toDouble())
        return androidx.compose.ui.geometry.Offset(
            -kotlin.math.cos(a).toFloat() * sway,
            drop + (-kotlin.math.sin(a).toFloat() - 1f) * sway,
        )
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
 */
@Composable
internal fun TrackSpecularLight() {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    DisposableEffect(context, owner) {
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
        SpecularLight.publish(SpecularLight.REST + MAX_SWAY * tanh(turn * GAIN / MAX_SWAY) * upright)
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit

    private companion object {
        const val SMOOTH = 0.2f
        const val WARMUP = 8
        /** Livelier than the room's real up, so even small, ordinary handling visibly moves it. */
        const val GAIN = 1.7f
        /** How far the light can sway from rest, either way. */
        const val MAX_SWAY = 45f
    }
}
