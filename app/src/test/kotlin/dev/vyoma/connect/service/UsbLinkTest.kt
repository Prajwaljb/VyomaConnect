package dev.vyoma.connect.service

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.link.TransportKind
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class UsbLinkTest {
    private fun device(companionOf: String?, vararg via: TransportKind) = DeviceState(
        DeviceIdentity("id", "name", DeviceType.DESKTOP, 8, emptySet(), emptySet(), companionOf = companionOf),
        PairState.PAIRED,
        via.toSet(),
    )

    @Test fun aCompanionOnTheCableLeavesThePcsLinkStillToDial() {
        assertFalse(UsbLink.pcOnCable(listOf(device(null), device("pc", TransportKind.USB))))
        assertFalse(UsbLink.pcOnCable(listOf(device(null, TransportKind.LAN), device("pc", TransportKind.USB))))
    }

    @Test fun thePcOnTheCableIsEnough() {
        assertTrue(UsbLink.pcOnCable(listOf(device(null, TransportKind.USB), device("pc"))))
    }
}
