package dev.vyoma.connect.presenter

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import kotlin.math.abs
import kotlin.math.sqrt

/** Pointer travel for one gyro reading, in KDE Connect's units (the screen is -1..1 across). */
internal object PointerMath {
    /** 3 units a radian: a turn of about 38° crosses the whole screen. KDE scales y by its aspect, so one gain suits both. */
    const val GAIN = 3f
    /** Below this (rad/s) turning is scaled down, so a steady hand holds the dot still without a hard edge. */
    const val TIGHT_RAD_S = 0.05f

    /**
     * [wx], [wy], [wz] are the gyro's rad/s and [up] the unit vector to the sky, in device axes. Turning
     * is measured around the real vertical, so it feels the same however the phone is tilted; raising
     * the top edge aims up. Returns dx (right +) and dy (down +).
     */
    fun step(wx: Float, wy: Float, wz: Float, up: FloatArray, dt: Float): Pair<Float, Float> {
        val yaw = tighten(-(wx * up[0] + wy * up[1] + wz * up[2]))
        val pitch = tighten(wx)
        return yaw * dt * GAIN to -pitch * dt * GAIN
    }

    private fun tighten(v: Float): Float {
        val a = abs(v)
        return if (a >= TIGHT_RAD_S) v else v * (a / TIGHT_RAD_S)
    }
}

/** Runs only while the pointer is held: [onMove] gets each reading's travel on the main thread. */
class PointerGyro(context: Context, private val onMove: (dx: Float, dy: Float) -> Unit) : SensorEventListener {
    private val sensors = context.getSystemService(SensorManager::class.java)
    private val gyro: Sensor? = sensors.getDefaultSensor(Sensor.TYPE_GYROSCOPE)
    private val gravity: Sensor? = sensors.getDefaultSensor(Sensor.TYPE_GRAVITY)
    private val up = floatArrayOf(0f, 0f, 1f)
    private var lastNs = 0L

    val available: Boolean get() = gyro != null

    fun start() {
        lastNs = 0L
        gyro?.let { sensors.registerListener(this, it, SensorManager.SENSOR_DELAY_GAME) }
        gravity?.let { sensors.registerListener(this, it, SensorManager.SENSOR_DELAY_GAME) }
    }

    fun stop() = sensors.unregisterListener(this)

    override fun onSensorChanged(e: SensorEvent) {
        val v = e.values
        if (e.sensor.type == Sensor.TYPE_GRAVITY) {
            val n = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
            if (n > 1f) for (i in 0..2) up[i] = v[i] / n
            return
        }
        val dt = if (lastNs == 0L) 0f else ((e.timestamp - lastNs) / 1e9f).coerceIn(0f, 0.05f)
        lastNs = e.timestamp
        val (dx, dy) = PointerMath.step(v[0], v[1], v[2], up, dt)
        if (dx != 0f || dy != 0f) onMove(dx, dy)
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit
}
