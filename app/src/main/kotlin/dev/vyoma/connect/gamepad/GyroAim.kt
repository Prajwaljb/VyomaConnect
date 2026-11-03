package dev.vyoma.connect.gamepad

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.view.Surface
import kotlin.math.abs
import kotlin.math.sign
import kotlin.math.sqrt

/**
 * Gyro aiming for the controller. Turning is measured around the real vertical (from gravity), so it
 * turns the same however far back the phone is tilted ("player space"); tilting the top edge towards
 * you aims up, as on a Switch. Out come either mouse pixels (running totals, the companion moves its
 * gyro mouse by the difference) or a right-stick push.
 *
 * Runs on the main thread's looper; [onMove] follows every reading while [active].
 */
class GyroAim(context: Context, private val rotation: () -> Int, private val onMove: () -> Unit) : SensorEventListener {
    private val sensors = context.getSystemService(SensorManager::class.java)
    private val gyro: Sensor? = sensors.getDefaultSensor(Sensor.TYPE_GYROSCOPE)
    private val gravity: Sensor? = sensors.getDefaultSensor(Sensor.TYPE_GRAVITY)
    val available: Boolean get() = gyro != null

    var speed = 1f
    var invertY = false
    /** Whether readings count right now (the profile's mode and, for "Holding LT", the trigger). */
    var active = false
        set(v) {
            if (field == v) return
            field = v
            if (!v) { stickX = 0f; stickY = 0f; restX = 0f; restY = 0f }
        }

    /** Mouse pixels moved so far (wrapping). */
    var mouseX = 0; private set
    var mouseY = 0; private set
    /** The right stick's push, -1..1 (y down). */
    var stickX = 0f; private set
    var stickY = 0f; private set

    private var restX = 0f
    private var restY = 0f
    private val up = floatArrayOf(0f, 0f, 1f)
    private var lastNs = 0L
    private var running = false

    fun start() {
        if (running || gyro == null) return
        running = true
        lastNs = 0L
        // 200 Hz: as fast as apps may ask without the high-rate permission.
        sensors.registerListener(this, gyro, 5_000)
        gravity?.let { sensors.registerListener(this, it, SensorManager.SENSOR_DELAY_GAME) }
    }

    fun stop() {
        if (!running) return
        running = false
        sensors.unregisterListener(this)
        stickX = 0f; stickY = 0f
    }

    override fun onSensorChanged(e: SensorEvent) {
        if (e.sensor.type == Sensor.TYPE_GRAVITY) {
            val n = sqrt(e.values[0] * e.values[0] + e.values[1] * e.values[1] + e.values[2] * e.values[2])
            if (n > 1f) for (i in 0..2) up[i] = e.values[i] / n
            return
        }
        val dt = if (lastNs == 0L) 0f else ((e.timestamp - lastNs) / 1e9f).coerceIn(0f, 0.05f)
        lastNs = e.timestamp
        if (!active) return
        val (wx, wy, wz) = Triple(e.values[0], e.values[1], e.values[2])
        // Turning right is clockwise seen from above: minus the spin around "up".
        var yaw = -(wx * up[0] + wy * up[1] + wz * up[2])
        // The screen's horizontal axis is the device's y one way round, minus it the other.
        var pitch = if (rotation() == Surface.ROTATION_270) -wy else wy
        if (invertY) pitch = -pitch
        // Below this the phone is only resting in the hands.
        yaw = calm(yaw)
        pitch = calm(pitch)
        // About 10 px per degree at speed 1.
        restX += yaw * dt * PX_PER_RAD * speed
        restY += -pitch * dt * PX_PER_RAD * speed
        val dx = restX.toInt()
        val dy = restY.toInt()
        restX -= dx; restY -= dy
        mouseX += dx; mouseY += dy
        // The stick: full at 120°/s (at speed 1), pushed past a game's usual dead zone once moving.
        stickX = push(yaw * speed / FULL_STICK_RAD_S)
        stickY = push(-pitch * speed / FULL_STICK_RAD_S)
        onMove()
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}

    private fun calm(v: Float) = if (abs(v) < CALM_RAD_S) 0f else v - sign(v) * CALM_RAD_S

    private fun push(v: Float): Float {
        if (v == 0f) return 0f
        return sign(v) * (STICK_FLOOR + (1 - STICK_FLOOR) * abs(v).coerceAtMost(1f))
    }

    private companion object {
        const val PX_PER_RAD = 573f
        const val FULL_STICK_RAD_S = 2.1f
        const val CALM_RAD_S = 0.01f
        const val STICK_FLOOR = 0.15f
    }
}
