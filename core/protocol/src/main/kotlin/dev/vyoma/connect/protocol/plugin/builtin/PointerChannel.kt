package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.VLog
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * The touchpad beside the link, for Wi-Fi that stalls (the companion's pointer.rs). One writer keeps
 * everything in order, so a click can't overtake the move before it, and runs of moves merge if the
 * radio falls behind. It idles with a small frame, which also keeps Bluetooth from dozing between strokes.
 */
class PointerChannel(private val out: OutputStream, private val input: InputStream, key: ByteArray, onTrip: (TripReport) -> Unit, private val onClose: () -> Unit) {
    private class Step(val kind: Int, var dx: Float, var dy: Float)

    private val mac = pointerMac(key)
    private val queue = ArrayDeque<Step>()
    private val lock = Object()
    private val trips = RoundTrips("touchpad", onTrip)
    @Volatile private var open = true
    @Volatile private var writingSince = 0L

    /** False once it has failed or stalled: send the old way, over the link. */
    val alive: Boolean
        get() {
            val since = writingSince
            if (open && since != 0L && now() - since > STALL_MS) {
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

    private fun write() {
        var seq = 0
        var reported = now()
        runCatching {
            while (open) {
                val step = synchronized(lock) {
                    if (queue.isEmpty()) lock.wait(KEEP_AWAKE_MS)
                    queue.removeFirstOrNull()
                } ?: Step(POINTER_KEEP_AWAKE, 0f, 0f)
                if (!open) break
                seq++
                trips.sending(seq)
                writingSince = now().coerceAtLeast(1L)
                out.write(pointerFrame(seq, step.kind, step.dx, step.dy, mac))
                out.flush()
                writingSince = 0L
                if (now() - reported > REPORT_MS) {
                    trips.report("Bluetooth")
                    reported = now()
                }
            }
        }.onFailure { VLog.d(TAG, "touchpad channel closed: ${it.message}") }
        trips.report("Bluetooth")
        close()
    }

    /** Echoes of every 32nd frame, for the log. */
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
                trips.echoed(ByteBuffer.wrap(echo).int)
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
        const val KEEP_AWAKE_MS = 100L
        const val STALL_MS = 500L
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
