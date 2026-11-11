package dev.vyoma.connect.ui

import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.plugin.builtin.GamepadRoute
import dev.vyoma.connect.protocol.plugin.builtin.TripReport
import dev.vyoma.connect.service.BluetoothFallback
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectionChecksTest {
    private val offline = LinkFacts(
        pc = emptySet(), paired = true, companion = null, bluetooth = BluetoothFallback.Readiness.READY,
        controllerRoute = null, controllerTrip = null, touchpadOpen = false, touchpadTrip = null,
    )

    private fun LinkFacts.check(kind: CheckKind) = connectionChecks(this).single { it.kind == kind }

    @Test fun bluetoothOnlyShowsAsWorkingEvenThoughAndroidSaysDisconnected() {
        val facts = offline.copy(
            pc = setOf(TransportKind.BLUETOOTH),
            companion = setOf(TransportKind.BLUETOOTH),
            controllerRoute = GamepadRoute.BLUETOOTH,
            touchpadOpen = true,
        )
        val radio = facts.check(CheckKind.BLUETOOTH)
        assertEquals(CheckLevel.GOOD, radio.level)
        assertTrue(radio.detail, radio.detail.startsWith("In use: the PC, Vyoma on the PC, touchpad, controller."))
        assertEquals(CheckLevel.QUIET, facts.check(CheckKind.WIFI).level)
        assertEquals(CheckLevel.QUIET, facts.check(CheckKind.USB).level)
        assertEquals("Connected over Bluetooth", facts.check(CheckKind.COMPANION).detail)
    }

    @Test fun aSideChannelCountsAsBluetoothInUseBesideWifi() {
        val facts = offline.copy(pc = setOf(TransportKind.LAN), touchpadOpen = true)
        assertEquals(CheckLevel.GOOD, facts.check(CheckKind.BLUETOOTH).level)
        assertEquals("In use: the PC", facts.check(CheckKind.WIFI).detail)
    }

    @Test fun whatStopsBluetoothIsNamed() {
        val needs = offline.copy(bluetooth = BluetoothFallback.Readiness.NEEDS_PERMISSION).check(CheckKind.BLUETOOTH)
        assertEquals(CheckLevel.PROBLEM, needs.level)
        assertEquals(CheckLevel.QUIET, offline.copy(bluetooth = BluetoothFallback.Readiness.RADIO_OFF).check(CheckKind.BLUETOOTH).level)
    }

    @Test fun timingShowsOnceMeasured() {
        val open = offline.copy(controllerRoute = GamepadRoute.USB)
        assertEquals("Over the USB cable, timing it now", open.check(CheckKind.CONTROLLER).detail)
        assertEquals(
            "Over the USB cable, 2 ms typical, 4 ms at worst",
            open.copy(controllerTrip = TripReport(2, 4)).check(CheckKind.CONTROLLER).detail,
        )
    }

    @Test fun awayKeepsOnlyWhatStillHasSomethingToSay() {
        val away = connectionChecks(offline, away = true)
        assertEquals(listOf(CheckKind.BLUETOOTH), away.map { it.kind })
        assertEquals(CheckLevel.PROBLEM, connectionChecks(offline.copy(bluetooth = BluetoothFallback.Readiness.NOT_PAIRED), away = true).single().level)
    }

    @Test fun bluetoothBackupOffIsNoProblem() {
        val off = offline.copy(bluetooth = BluetoothFallback.Readiness.NEEDS_PERMISSION, bluetoothOn = false)
        assertEquals(CheckLevel.QUIET, off.check(CheckKind.BLUETOOTH).level)
        assertTrue(connectionChecks(off, away = true).isEmpty())
    }

    @Test fun idleSideChannelsAreLeftOut() {
        val kinds = connectionChecks(offline.copy(pc = setOf(TransportKind.LAN))).map { it.kind }
        assertTrue(CheckKind.CONTROLLER !in kinds && CheckKind.TOUCHPAD !in kinds)
    }

    @Test fun nothingConnectedClaimsNoEncryption() {
        assertEquals(CheckLevel.QUIET, offline.check(CheckKind.ENCRYPTION).level)
    }
}
