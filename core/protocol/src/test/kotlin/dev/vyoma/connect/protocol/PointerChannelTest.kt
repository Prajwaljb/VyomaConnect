package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.plugin.builtin.PointerChannel
import org.junit.Test
import java.io.DataInputStream
import java.io.PipedInputStream
import java.io.PipedOutputStream
import java.nio.ByteBuffer
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class PointerChannelTest {
    private class Frame(val seq: Int, val kind: Int, val dx: Float, val dy: Float)

    /** A PC that reads the frames and echoes only when told to. */
    private class FakePc {
        val toPc = PipedInputStream(1 shl 16)
        val fromPhone = PipedOutputStream(toPc)
        val toPhone = PipedInputStream(1 shl 16)
        private val echoes = PipedOutputStream(toPhone)
        val frames = LinkedBlockingQueue<Frame>()

        init {
            Thread {
                val input = DataInputStream(toPc)
                val buf = ByteArray(33)
                runCatching {
                    while (true) {
                        input.readFully(buf)
                        val b = ByteBuffer.wrap(buf, 4, 13)
                        frames.add(Frame(b.int, b.get().toInt(), b.float, b.float))
                    }
                }
            }.apply { isDaemon = true }.start()
        }

        fun echo(seq: Int) {
            echoes.write(ByteBuffer.allocate(4).putInt(seq).array())
            echoes.flush()
        }

        fun next(): Frame = assertNotNull(frames.poll(2, TimeUnit.SECONDS), "no frame came")
    }

    @Test
    fun movesWaitForEchoesAndMergeMeanwhile() {
        val pc = FakePc()
        val channel = PointerChannel(pc.fromPhone, pc.toPhone, ByteArray(32) { 1 }, {}) {}
        try {
            // The first frame (idle) says hello; its number is one the PC echoes.
            val hello = pc.next()
            assertEquals(0, hello.kind)
            assertEquals(0, hello.seq % 32)
            pc.echo(hello.seq)
            // Four moves may be on their way; the rest wait while no echo comes.
            val sent = mutableListOf<Frame>()
            repeat(20) {
                channel.move(1f, 2f)
                Thread.sleep(5)
            }
            Thread.sleep(50)
            pc.frames.drainTo(sent)
            assertTrue(sent.size <= 4, "sent ${sent.size} frames with no echo")
            assertTrue(sent.zipWithNext().all { (a, b) -> b.seq - a.seq == 32 })
            // The echo of the last one lets the rest go, as one merged move.
            pc.echo(sent.last().seq)
            val merged = pc.next()
            assertEquals(1, merged.kind)
            assertEquals(20f, sent.sumOf { it.dx.toDouble() }.toFloat() + merged.dx)
            assertEquals(40f, sent.sumOf { it.dy.toDouble() }.toFloat() + merged.dy)
            assertTrue(channel.alive)
        } finally {
            channel.close()
        }
    }

    @Test
    fun aSilentPcCountsAsGone() {
        val pc = FakePc()
        val channel = PointerChannel(pc.fromPhone, pc.toPhone, ByteArray(32) { 1 }, {}) {}
        pc.next()
        Thread.sleep(1_700)
        assertTrue(!channel.alive, "no echo for 1.5 s: back to the link")
    }

    @Test
    fun restingSendsNothing() {
        val pc = FakePc()
        val channel = PointerChannel(pc.fromPhone, pc.toPhone, ByteArray(32) { 1 }, {}) {}
        try {
            pc.echo(pc.next().seq)
            channel.resting = true
            Thread.sleep(50)
            pc.frames.clear()
            Thread.sleep(400)
            assertEquals(0, pc.frames.size)
            channel.resting = false
            assertEquals(0, pc.next().kind)
        } finally {
            channel.close()
        }
    }
}
