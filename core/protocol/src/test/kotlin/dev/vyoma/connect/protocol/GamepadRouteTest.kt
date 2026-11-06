package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.plugin.builtin.CableWatch
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.RoundTrips
import java.io.ByteArrayOutputStream
import java.io.OutputStream
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertSame

class GamepadRouteTest {
    private class FakeCable(private val isPlugged: Boolean, private val stream: OutputStream) : CompanionPlugin.CableChannel {
        override fun plugged() = isPlugged
        override fun open() = stream
        override fun close() {}
    }

    @Test
    fun anAccessoryStillListedAfterThePullIsLeftAlone() {
        assertNull(FakeCable(isPlugged = false, ByteArrayOutputStream()).openIfPlugged())
    }

    @Test
    fun anAccessoryOnAPluggedCableIsUsed() {
        val stream = ByteArrayOutputStream()
        assertSame(stream, FakeCable(isPlugged = true, stream).openIfPlugged())
    }

    @Test
    fun aLateEchoIsNotCountedAgainstTheNewRoute() {
        val trips = RoundTrips()
        trips.sending(32)
        trips.moved()
        trips.echoed(32)
        assertEquals(0, trips.report("USB"))

        trips.sending(64)
        trips.echoed(64)
        assertEquals(1, trips.report("USB"))
    }

    @Test
    fun aPulledCableIsSpottedOnceEchoesStop() {
        val watch = CableWatch(silentMs = 250)
        watch.echoed(33)
        watch.sent(1_000)
        watch.sent(1_100) // Later frames don't restart the wait.
        assertEquals(false, watch.silent(1_250))
        assertEquals(true, watch.silent(1_251))
        watch.echoed(34)
        assertEquals(false, watch.silent(5_000))
    }

    @Test
    fun anOlderCompanionsSparseEchoesDontLookLikeAPull() {
        val watch = CableWatch(silentMs = 250)
        watch.echoed(32)
        watch.sent(1_000)
        assertEquals(false, watch.silent(9_000))
    }

    @Test
    fun aPausedPhoneIsNotMistakenForAPull() {
        val watch = CableWatch(silentMs = 250)
        watch.echoed(33)
        assertEquals(false, watch.silent(9_000)) // Nothing sent, so nothing owed.
    }

    @Test
    fun reportsCarryTheTimingForTheChecksScreen() {
        val seen = mutableListOf<dev.vyoma.connect.protocol.plugin.builtin.TripReport>()
        val trips = RoundTrips(onReport = { seen += it })
        assertEquals(0, trips.report("USB"))
        assertEquals(0, seen.size)
        trips.sending(32)
        trips.echoed(32)
        trips.report("USB")
        assertEquals(1, seen.size)
    }
}
