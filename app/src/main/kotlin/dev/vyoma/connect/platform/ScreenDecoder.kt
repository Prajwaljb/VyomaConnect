package dev.vyoma.connect.platform

import android.media.MediaCodec
import android.media.MediaFormat
import android.view.Surface
import dev.vyoma.connect.protocol.VLog
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.EOFException
import java.io.IOException
import java.io.InputStream
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/**
 * Plays the PC's live screen: length-prefixed H.264 (the companion's VA-API encoder) into a
 * hardware decoder rendering straight to [surface]. Frames are shown the moment they decode, with
 * no clock or buffering, because for a live screen latency matters more than smoothness.
 *
 * Two threads: one splits the stream into access units and queues them, one releases decoded frames.
 */
class ScreenDecoder(
    private val input: InputStream,
    private val surface: Surface,
    /** Called once the picture size is known (from the stream's SPS), on the reader thread. */
    private val onSize: (width: Int, height: Int) -> Unit,
    /** Called when the stream ends or fails; null error means it simply ended. */
    private val onEnd: (Throwable?) -> Unit,
    /** Called once the first picture is on screen, on the output thread. */
    private val onFirstFrame: () -> Unit = {},
) {
    private val running = AtomicBoolean(true)
    private var codec: MediaCodec? = null

    /** Decoded frames shown and bytes received so far, for the on-screen rate readout. */
    val frames = AtomicLong()
    val bytes = AtomicLong()
    /** Sum of arrival-to-display times (µs) over [frames]: how long the phone holds a frame. */
    val heldUs = AtomicLong()
    /** Decoded pictures not shown because a newer one was already ready. */
    val skipped = AtomicLong()
    /** PC send time per queued frame (keyed by its presentation time), for the end-to-end figure. */
    private val sentAt = java.util.concurrent.ConcurrentHashMap<Long, Long>()
    /** PC-sent to shown, per frame, µs: the sum and count over the current window, and the session's best. */
    private val e2eSum = AtomicLong()
    private val e2eCount = AtomicLong()
    /** The slowest frame of the current window, µs: a burst shows here before it moves an average. */
    private val e2eWorst = AtomicLong()
    @Volatile var e2eBestUs = Long.MAX_VALUE
        private set
    /** Average PC-sent-to-shown delay above the session's best, ms: what's queueing up on the way. */
    @Volatile var queuedMs = 0L
        private set
    /** PC-sent to read off the socket, µs (sum, count, best): the part of [queuedMs] spent on the way. */
    private val inSum = AtomicLong()
    private val inCount = AtomicLong()
    @Volatile private var inBest = Long.MAX_VALUE

    fun start() {
        Thread({ readLoop() }, "screen-in").start()
    }

    /** Stops decoding and closes the stream (which ends the cast on the PC). Safe to call twice. */
    fun stop() {
        if (!running.compareAndSet(true, false)) return
        runCatching { input.close() }
    }

    private fun readLoop() {
        var error: Throwable? = null
        try {
            val units = AccessUnits(input) { bytes.addAndGet(it.toLong()) }
            var sps: ByteArray? = null
            var pps: ByteArray? = null
            while (running.get()) {
                val au = units.next() ?: break
                if (codec == null) {
                    // Wait for the first keyframe: its parameter sets configure the decoder.
                    au.nals.firstOrNull { it.type == 7 }?.let { sps = it.bytes }
                    au.nals.firstOrNull { it.type == 8 }?.let { pps = it.bytes }
                    if (sps == null || pps == null || au.nals.none { it.type == 5 }) continue
                    codec = configure(sps!!, pps!!)
                }
                val pts = System.nanoTime() / 1000
                au.sentUs?.let {
                    sentAt[pts] = it
                    val age = System.currentTimeMillis() * 1000 - it
                    if (age < inBest) inBest = age
                    inSum.addAndGet(age); inCount.incrementAndGet()
                }
                queue(codec!!, au.data, pts)
            }
        } catch (e: Throwable) {
            if (running.get()) error = e
        } finally {
            running.set(false)
            runCatching { input.close() }
            codec?.let { c -> runCatching { c.stop() }; runCatching { c.release() } }
            codec = null
            onEnd(error)
        }
    }

    private fun configure(sps: ByteArray, pps: ByteArray): MediaCodec {
        val (w, h) = SpsSize.parse(sps) ?: (1920 to 1080)
        onSize(w, h)
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h).apply {
            setByteBuffer("csd-0", ByteBuffer.wrap(sps))
            setByteBuffer("csd-1", ByteBuffer.wrap(pps))
            setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            // Realtime priority; decoders that don't know these keys ignore them.
            setInteger(MediaFormat.KEY_PRIORITY, 0)
        }
        val codec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        codec.configure(format, surface, null, 0)
        codec.start()
        Thread({ drainLoop(codec) }, "screen-out").start()
        val lowLatency = runCatching {
            codec.codecInfo.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
                .isFeatureSupported(android.media.MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)
        }.getOrDefault(false)
        VLog.i(TAG, "decoding ${w}x$h with ${codec.name}, low latency ${if (lowLatency) "supported" else "not supported"}")
        return codec
    }

    private fun queue(codec: MediaCodec, data: ByteArray, pts: Long) {
        while (running.get()) {
            val index = codec.dequeueInputBuffer(10_000)
            if (index < 0) continue
            val buffer = codec.getInputBuffer(index) ?: return
            buffer.clear()
            if (data.size > buffer.capacity()) {
                // Can't happen with a sane encoder; drop rather than corrupt the stream.
                codec.queueInputBuffer(index, 0, 0, 0, 0)
                return
            }
            buffer.put(data)
            codec.queueInputBuffer(index, 0, data.size, pts, 0)
            return
        }
    }

    private fun drainLoop(codec: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        var mark = System.nanoTime()
        var markFrames = 0L
        var markBytes = 0L
        var markHeld = 0L
        try {
            while (running.get()) {
                val now = System.nanoTime()
                if (now - mark >= LOG_EVERY_NS) {
                    // For the log (and the demo's research log): what the cast actually delivers.
                    val n = frames.get() - markFrames
                    val count = e2eCount.getAndSet(0)
                    val sum = e2eSum.getAndSet(0)
                    if (count > 0) queuedMs = (sum / count - e2eBestUs) / 1000
                    val ic = inCount.getAndSet(0)
                    val onTheWay = if (ic > 0) (inSum.getAndSet(0) / ic - inBest) / 1000 else -1
                    VLog.i(TAG, "%.0f fps (%d skipped), %.1f Mb/s, decode %d ms, PC-to-screen %d ms raw (best %d), queued %d ms (%d on the way, worst %d)".format(
                        n * 1e9 / (now - mark), skipped.getAndSet(0), (bytes.get() - markBytes) * 8e3 / (now - mark), if (n > 0) (heldUs.get() - markHeld) / n / 1000 else 0,
                        if (count > 0) sum / count / 1000 else -1, if (e2eBestUs == Long.MAX_VALUE) -1 else e2eBestUs / 1000, queuedMs, onTheWay,
                        e2eWorst.getAndSet(0).let { if (it > 0) (it - e2eBestUs) / 1000 else -1 },
                    ))
                    mark = now; markFrames = frames.get(); markBytes = bytes.get(); markHeld = heldUs.get()
                }
                var index = codec.dequeueOutputBuffer(info, 10_000)
                var pts = info.presentationTimeUs
                // A burst decoded at once: show only the newest picture. Rendering every one would
                // queue them up for the screen, and a live view must never fall behind.
                while (index >= 0) {
                    val newer = codec.dequeueOutputBuffer(info, 0)
                    if (newer < 0) break
                    codec.releaseOutputBuffer(index, false)
                    sentAt.remove(pts)
                    skipped.incrementAndGet()
                    index = newer
                    pts = info.presentationTimeUs
                }
                if (index >= 0) {
                    codec.releaseOutputBuffer(index, true)
                    heldUs.addAndGet(System.nanoTime() / 1000 - pts)
                    sentAt.remove(pts)?.let { sent ->
                        // Phone and PC clocks differ by an unknown offset, so the best frame of the session
                        // is the baseline and the excess is the queueing: honest without a clock sync.
                        val e2e = System.currentTimeMillis() * 1000 - sent
                        if (e2e < e2eBestUs) e2eBestUs = e2e
                        e2eSum.addAndGet(e2e); e2eCount.incrementAndGet()
                        e2eWorst.accumulateAndGet(e2e, ::maxOf)
                    }
                    if (frames.incrementAndGet() == 1L) onFirstFrame()
                }
            }
        } catch (_: IllegalStateException) {
            // The codec was released under us while stopping.
        }
    }

    private companion object {
        const val TAG = "ScreenDecoder"
        const val LOG_EVERY_NS = 5_000_000_000L
    }
}

/** One NAL unit as MediaCodec wants it: a 4-byte start code, then the unit. */
private class Nal(val bytes: ByteArray) {
    val type: Int get() = bytes[4].toInt() and 0x1f
    val isSlice: Boolean get() = type == 1 || type == 5
}

/** [sentUs]: when the PC sent it (µs since the epoch, the companion's stamp), if stamped. */
private class AccessUnit(val nals: List<Nal>, val sentUs: Long?) {
    val data: ByteArray by lazy {
        ByteArrayOutputStream(nals.sumOf { it.bytes.size }).also { out -> nals.forEach { out.write(it.bytes) } }.toByteArray()
    }
}

private fun startCodeLength(b: ByteArray) = if (b.size > 3 && b[2].toInt() == 0) 4 else 3

/**
 * Reads pictures from the companion's length-prefixed stream (4-byte big-endian size, then the
 * NAL unit). Each picture is one slice, so it's complete the moment its slice arrives: nothing
 * waits for the next frame. The size prefix is overwritten in place with a start code.
 */
private class AccessUnits(input: InputStream, private val counted: (Int) -> Unit) {
    private val data = DataInputStream(BufferedInputStream(input, 256 * 1024))

    fun next(): AccessUnit? {
        val unit = ArrayList<Nal>(3)
        var sent: Long? = null
        while (true) {
            val size = try { data.readInt() } catch (_: EOFException) { return null }
            if (size <= 0 || size > MAX_NAL) throw IOException("Bad NAL size $size")
            val bytes = ByteArray(size + 4)
            data.readFully(bytes, 4, size)
            bytes[3] = 1 // 00 00 00 01
            counted(size + 4)
            // The companion's send-time stamp: read it, keep it away from the decoder.
            if (size == STAMP_SIZE && bytes[4].toInt() == 6 && bytes[5] == 'V'.code.toByte() && bytes[6] == 'Y'.code.toByte() &&
                bytes[7] == 'T'.code.toByte() && bytes[8] == 'S'.code.toByte()
            ) {
                sent = java.nio.ByteBuffer.wrap(bytes, 9, 8).long
                continue
            }
            val nal = Nal(bytes)
            unit += nal
            if (nal.isSlice) return AccessUnit(unit, sent)
        }
    }

    private companion object {
        const val MAX_NAL = 16 * 1024 * 1024
        const val STAMP_SIZE = 13
    }
}

/** The picture size from an SPS, so the decoder is created at the right size. */
internal object SpsSize {
    fun parse(nal: ByteArray): Pair<Int, Int>? = runCatching {
        val r = BitReader(unescape(nal.copyOfRange(startCodeLength(nal) + 1, nal.size)))
        val profile = r.bits(8)
        r.bits(16) // constraint flags, level
        r.ue() // sps id
        if (profile in setOf(100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135)) {
            val chroma = r.ue()
            if (chroma == 3) r.bits(1)
            r.ue(); r.ue(); r.bits(1)
            if (r.bits(1) == 1) repeat(if (chroma != 3) 8 else 12) { i -> if (r.bits(1) == 1) skipScaling(r, if (i < 6) 16 else 64) }
        }
        r.ue() // log2_max_frame_num
        when (r.ue()) {
            0 -> r.ue()
            1 -> { r.bits(1); r.se(); r.se(); repeat(r.ue()) { r.se() } }
        }
        r.ue(); r.bits(1)
        val wMbs = r.ue() + 1
        val hMapUnits = r.ue() + 1
        val frameMbsOnly = r.bits(1)
        if (frameMbsOnly == 0) r.bits(1)
        r.bits(1)
        var w = wMbs * 16
        var h = (2 - frameMbsOnly) * hMapUnits * 16
        if (r.bits(1) == 1) {
            val (l, rt, t, b) = List(4) { r.ue() }
            w -= (l + rt) * 2
            h -= (t + b) * 2 * (2 - frameMbsOnly)
        }
        w to h
    }.getOrNull()

    private fun skipScaling(r: BitReader, size: Int) {
        var last = 8
        var next = 8
        repeat(size) {
            if (next != 0) next = (last + r.se() + 256) % 256
            if (next != 0) last = next
        }
    }

    private fun unescape(b: ByteArray): ByteArray {
        val out = ByteArrayOutputStream(b.size)
        var zeros = 0
        for (x in b) {
            if (zeros >= 2 && x.toInt() == 3) { zeros = 0; continue }
            out.write(x.toInt())
            zeros = if (x.toInt() == 0) zeros + 1 else 0
        }
        return out.toByteArray()
    }

    private class BitReader(private val b: ByteArray) {
        private var bit = 0
        fun bits(n: Int): Int {
            var v = 0
            repeat(n) {
                val byte = b[bit ushr 3].toInt()
                v = (v shl 1) or ((byte ushr (7 - (bit and 7))) and 1)
                bit++
            }
            return v
        }
        fun ue(): Int {
            var zeros = 0
            while (bits(1) == 0) zeros++
            return (1 shl zeros) - 1 + if (zeros > 0) bits(zeros) else 0
        }
        fun se(): Int = ue().let { if (it and 1 == 1) (it + 1) / 2 else -(it / 2) }
    }
}
