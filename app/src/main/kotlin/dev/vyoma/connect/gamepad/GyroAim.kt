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
 * Drift (the user saw the pointer creep while holding still, 2026-11-03): the S24 FE's gyro keeps a
 * leftover offset of about 0.009 rad/s after Android's own calibration, and hands tremble. So the
 * offset is relearned whenever the phone has been steady for [STEADY_NS], and slow turning is
 * tightened (scaled down towards zero below [TIGHT_RAD_S]) rather than cut at a hard edge, which kept
 * creep just past the edge and made fine aim jump.
 *
 * Readings come on a thread of their own ([onMove] runs there, after every reading while [active]):
 * on the main looper they queued behind each frame the controller drew, and the aim stuttered.
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
    /** The gyro's leftover offset, relearned while the phone is steady. */
    private val bias = FloatArray(3)
    private val steadySum = FloatArray(3)
    private val steadyFirst = FloatArray(3)
    private var steadyCount = 0
    private var steadySince = 0L

    private var thread: android.os.HandlerThread? = null

    fun start() {
        if (running || gyro == null) return
        running = true
        lastNs = 0L
        val t = android.os.HandlerThread("gyro", android.os.Process.THREAD_PRIORITY_URGENT_DISPLAY).also { it.start() }
        thread = t
        val handler = android.os.Handler(t.looper)
        // 200 Hz: as fast as apps may ask without the high-rate permission; no batching.
        sensors.registerListener(this, gyro, 5_000, 0, handler)
        gravity?.let { sensors.registerListener(this, it, SensorManager.SENSOR_DELAY_GAME, 0, handler) }
    }

    fun stop() {
        if (!running) return
        running = false
        sensors.unregisterListener(this)
        thread?.quitSafely()
        thread = null
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
        learnBias(e.values, e.timestamp)
        val (wx, wy, wz) = Triple(e.values[0] - bias[0], e.values[1] - bias[1], e.values[2] - bias[2])
        // Turning right is clockwise seen from above: minus the spin around "up".
        var yaw = -(wx * up[0] + wy * up[1] + wz * up[2])
        // The screen's horizontal axis is the device's y one way round, minus it the other.
        var pitch = if (rotation() == Surface.ROTATION_270) -wy else wy
        if (invertY) pitch = -pitch
        yaw = tighten(yaw)
        pitch = tighten(pitch)
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

    /**
     * Steady means every reading stays within [STEADY_SPREAD] of where this run of them began; after
     * [STEADY_NS] of that, their average is the offset (eased in, so one still moment can't jolt it).
     */
    private fun learnBias(v: FloatArray, ns: Long) {
        val calm = steadyCount > 0 && (0..2).all { abs(v[it] - steadyFirst[it]) < STEADY_SPREAD } &&
            (0..2).all { abs(v[it]) < STEADY_MAX }
        if (!calm) {
            for (i in 0..2) { steadyFirst[i] = v[i]; steadySum[i] = 0f }
            steadyCount = 0
            steadySince = ns
            if ((0..2).any { abs(v[it]) >= STEADY_MAX }) return
        }
        for (i in 0..2) steadySum[i] += v[i]
        steadyCount++
        if (ns - steadySince >= STEADY_NS) {
            for (i in 0..2) bias[i] += (steadySum[i] / steadyCount - bias[i]) * 0.5f
            for (i in 0..2) steadySum[i] = 0f
            steadyCount = 0
            steadySince = ns
        }
    }

    /** Slow turning scaled down smoothly (squared below [TIGHT_RAD_S]); faster is untouched. */
    private fun tighten(v: Float): Float {
        val a = abs(v)
        return if (a >= TIGHT_RAD_S) v else v * (a / TIGHT_RAD_S)
    }

    private fun push(v: Float): Float {
        if (v == 0f) return 0f
        return sign(v) * (STICK_FLOOR + (1 - STICK_FLOOR) * abs(v).coerceAtMost(1f))
    }

    private companion object {
        const val PX_PER_RAD = 573f
        const val FULL_STICK_RAD_S = 2.1f
        const val TIGHT_RAD_S = 0.04f
        const val STEADY_SPREAD = 0.015f
        const val STEADY_MAX = 0.03f
        const val STEADY_NS = 800_000_000L
        const val STICK_FLOOR = 0.15f
    }
}
