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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlin.math.abs
import kotlin.math.atan2
import kotlin.math.hypot
import kotlin.math.sqrt

/**
 * Where the light comes from, as seen on the screen: the angle the specular edges are brightest
 * at, in degrees clockwise from 3 o'clock (225 = top left). It stays up in the room as the phone
 * tilts, like iOS's glass, so turning the phone slides the highlight around the edges. [drift] is
 * the recent tilt (each axis -1..1) for things that sit deeper than the page (the canvas glow, like
 * iOS's wallpaper parallax); it eases back to centre over a few seconds once the phone is held
 * still, so any holding angle is home. Read both only while drawing: a change then redraws, never
 * recomposes or relayouts.
 */
object SpecularLight {
    const val REST = 225f

    var angle by mutableFloatStateOf(REST)
        private set

    var drift by mutableStateOf(Offset.Zero)
        private set

    internal fun publish(a: Float, d: Offset) {
        // Hand tremor is a degree or two: below that nothing is redrawn.
        if (abs(a - angle) >= 1.5f) angle = a
        if ((d - drift).getDistance() >= 0.04f) drift = d
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
    private var z = 0f
    /** Samples since the sensor started; the first few settle the filters instead of drifting. */
    private var seen = 0
    /** Where the phone has been held lately (roll, pitch in degrees); drift is measured from it. */
    private var restRoll = 0f
    private var restPitch = 0f

    fun reset() {
        seen = 0
    }

    override fun onSensorChanged(e: SensorEvent) {
        // A fresh registration can hand over a zero or stale sample first: only real gravity counts.
        if (hypot(hypot(e.values[0], e.values[1]), e.values[2]) < SensorManager.GRAVITY_EARTH * 0.5f) return
        val warming = seen < WARMUP
        seen++
        if (warming) {
            x = e.values[0]; y = e.values[1]; z = e.values[2]
        } else {
            x += (e.values[0] - x) * SMOOTH
            y += (e.values[1] - y) * SMOOTH
            z += (e.values[2] - z) * SMOOTH
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
        val upright = ((hypot(sx, sy) / SensorManager.GRAVITY_EARTH - 0.15f) / 0.35f).coerceIn(0f, 1f)
        // Leaning back or forward: 0 upright, 90 flat on its back.
        val pitch = Math.toDegrees(atan2(z, sqrt(sx * sx + sy * sy)).toDouble()).toFloat()
        val roll = turn * upright
        if (warming) {
            // The way it's held when the app opens is home, so nothing moves on its own afterwards.
            restRoll = roll; restPitch = pitch
        }
        restRoll += (roll - restRoll) * RECENTER
        restPitch += (pitch - restPitch) * RECENTER
        val drift = Offset(
            ((roll - restRoll) / DRIFT_DEG).coerceIn(-1f, 1f),
            ((pitch - restPitch) / DRIFT_DEG).coerceIn(-1f, 1f),
        )
        SpecularLight.publish(SpecularLight.REST + (turn * GAIN).coerceIn(-RANGE, RANGE) * upright, drift)
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit

    private companion object {
        const val SMOOTH = 0.15f
        const val WARMUP = 8
        /** A little livelier than the room's real up, so ordinary handling visibly moves it. */
        const val GAIN = 1.4f
        const val RANGE = 110f
        /** Tilt (degrees from the recent rest) that drifts the canvas all the way. */
        const val DRIFT_DEG = 18f
        /** Per 20 ms sample: back to rest with a time constant of about three seconds. */
        const val RECENTER = 0.0067f
    }
}
