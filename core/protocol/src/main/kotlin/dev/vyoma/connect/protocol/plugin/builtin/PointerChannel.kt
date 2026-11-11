package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.VLog
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * The touchpad beside the link, for Wi-Fi that stalls (the companion's pointer.rs). One writer keeps
 * everything in order, so a click can't overtake the move before it.
 *
 * Only [WINDOW] frames are on their way at once. Android's Bluetooth takes every write at once and
 * queues it inside the stack, so when Wi-Fi crowds the shared radio (a busy 2.4 GHz band) the moves
 * would line up there and the pointer would trail the finger by however long that line got. Held
 * here instead, they merge into one step that goes as soon as the radio frees up. Every frame is
 * numbered in steps of [ECHO_EVERY], which the PC echoes, so each echo says what has arrived.
 *
 * It idles with a small frame, which also keeps Bluetooth from dozing between strokes, unless
 * [resting] (the touchpad is off screen and the channel is only kept for coming back).
 */
class PointerChannel(private val out: OutputStream, private val input: InputStream, key: ByteArray, onTrip: (TripReport) -> Unit, private val onClose: () -> Unit) {
    private class Step(val kind: Int, var dx: Float, var dy: Float)

    private val mac = pointerMac(key)
    private val queue = ArrayDeque<Step>()
    /** Frames sent and not yet echoed: their numbers and when they went. Under [lock]. */
    private val inFlight = ArrayDeque<Pair<Int, Long>>()
    private val lock = Object()
    private val trips = RoundTrips("touchpad", onTrip)
    @Volatile private var open = true
    @Volatile private var writingSince = 0L
    /** When the oldest frame still waiting for its echo went (0: none waiting). */
    @Volatile private var unansweredSince = 0L

    /** No idle frames: the touchpad is off screen and the channel is only kept for coming back. */
    @Volatile var resting = false
        set(value) {
            field = value
            synchronized(lock) { lock.notifyAll() }
        }

    /** False once it has failed or stalled: send the old way, over the link. */
    val alive: Boolean
        get() {
            val now = now()
            val writing = writingSince
            val waiting = unansweredSince
            if (open && ((writing != 0L && now - writing > STALL_MS) || (waiting != 0L && now - waiting > SILENT_MS))) {
                VLog.i(TAG, "touchpad's Bluetooth stalled; back to the link")
                close()
            }
            return open
        }

    init {
        Thread(::write, "vyoma-pointer-out").apply { isDaemon = true }.start()
        Thread(::read, "vyoma-pointer-in").apply { isDaemon = true }.start()
    }

    fun move(dx: Float, dy: Float) = post(POINTER_MOVE, dx, dy)
    fun scroll(dx: Float, dy: Float) = post(POINTER_SCROLL, dx, dy)
    /** [button]: 0 left, 1 right, 2 middle. */
    fun button(button: Int, down: Boolean) = post(if (down) POINTER_PRESS else POINTER_RELEASE, button.toFloat(), 0f)
    fun click(button: Int) = button(button, true) && button(button, false)

    /** False if it couldn't take it. */
    private fun post(kind: Int, dx: Float, dy: Float): Boolean {
        if (!alive) return false
        synchronized(lock) {
            val last = queue.lastOrNull()
            if (last != null && last.kind == kind && (kind == POINTER_MOVE || kind == POINTER_SCROLL)) {
                last.dx += dx
                last.dy += dy
            } else queue.addLast(Step(kind, dx, dy))
            lock.notifyAll()
        }
        return true
    }

    /** The next frame to send once there's room for it, or null once closed. */
    private fun next(lastSent: Long): Step? = synchronized(lock) {
        while (open) {
            val now = now()
            // An echo that never comes (the PC gave up writing it) mustn't hold the window shut.
            while (inFlight.isNotEmpty() && now - inFlight.first().second > ASSUME_ARRIVED_MS) inFlight.removeFirst()
            val room = inFlight.size < WINDOW
            if (room && queue.isNotEmpty()) return@synchronized queue.removeFirst()
            val idle = now - lastSent
            if (room && !resting && idle >= KEEP_AWAKE_MS) return@synchronized Step(POINTER_KEEP_AWAKE, 0f, 0f)
            val untilIdle = if (resting) Long.MAX_VALUE else KEEP_AWAKE_MS - idle
            val untilRoom = if (room) Long.MAX_VALUE else ASSUME_ARRIVED_MS - (now - inFlight.first().second)
            lock.wait(minOf(untilIdle, untilRoom, KEEP_AWAKE_MS).coerceAtLeast(1L))
        }
        null
    }

    private fun write() {
        var seq = 0
        var lastSent = 0L
        var reported = now()
        runCatching {
            while (open) {
                val step = next(lastSent) ?: break
                seq += ECHO_EVERY
                val now = now()
                synchronized(lock) {
                    inFlight.addLast(seq to now)
                    if (unansweredSince == 0L) unansweredSince = now
                }
                trips.sending(seq)
                writingSince = now.coerceAtLeast(1L)
                out.write(pointerFrame(seq, step.kind, step.dx, step.dy, mac))
                out.flush()
                writingSince = 0L
                lastSent = now()
                if (lastSent - reported > REPORT_MS) {
                    trips.report("Bluetooth")
                    reported = lastSent
                }
            }
        }.onFailure { VLog.d(TAG, "touchpad channel closed: ${it.message}") }
        trips.report("Bluetooth")
        close()
    }

    /** The PC's echoes: everything up to the echoed frame has arrived (the radio keeps order). */
    private fun read() {
        val echo = ByteArray(4)
        runCatching {
            while (open) {
                var got = 0
                while (got < 4) {
                    val n = input.read(echo, got, 4 - got)
                    if (n < 0) throw java.io.EOFException()
                    got += n
                }
                val seq = ByteBuffer.wrap(echo).int
                trips.echoed(seq)
                synchronized(lock) {
                    while (inFlight.isNotEmpty() && inFlight.first().first - seq <= 0) inFlight.removeFirst()
                    unansweredSince = inFlight.firstOrNull()?.second ?: 0L
                    lock.notifyAll()
                }
            }
        }
        close()
    }

    fun close() {
        synchronized(lock) {
            if (!open) return
            open = false
            lock.notifyAll()
        }
        runCatching { onClose() }
    }

    private fun now() = System.nanoTime() / 1_000_000

    private companion object {
        const val TAG = "CompanionPlugin"
        /** Frames on their way at once: a move every 8 ms keeps flowing while echoes take under ~30 ms. */
        const val WINDOW = 4
        /** Past this a frame counts as arrived even without its echo. */
        const val ASSUME_ARRIVED_MS = 300L
        const val KEEP_AWAKE_MS = 100L
        /** A write blocked this long, or no echo at all for [SILENT_MS]: the radio is gone. */
        const val STALL_MS = 500L
        const val SILENT_MS = 1_500L
        const val REPORT_MS = 30_000L
    }
}

internal const val POINTER_KEEP_AWAKE = 0
internal const val POINTER_MOVE = 1
internal const val POINTER_SCROLL = 2
internal const val POINTER_PRESS = 3
internal const val POINTER_RELEASE = 4

internal fun pointerMac(key: ByteArray): Mac = Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(key, "HmacSHA256")) }

/** "VYP1", seq, kind, dx, dy, big-endian, then the HMAC of those cut to 16 bytes: pointer.rs reads it. */
internal fun pointerFrame(seq: Int, kind: Int, dx: Float, dy: Float, mac: Mac): ByteArray {
    val b = ByteBuffer.allocate(33)
    b.put("VYP1".toByteArray()).putInt(seq).put(kind.toByte()).putFloat(dx).putFloat(dy)
    mac.update(b.array(), 0, 17)
    b.put(mac.doFinal(), 0, 16)
    return b.array()
}
