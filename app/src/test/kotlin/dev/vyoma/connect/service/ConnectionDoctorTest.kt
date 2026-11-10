package dev.vyoma.connect.service

import dev.vyoma.connect.service.ConnectionDoctor.Facts
import dev.vyoma.connect.service.ConnectionDoctor.Fix
import dev.vyoma.connect.service.ConnectionDoctor.Probe
import org.junit.Test
import java.net.InetAddress
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ConnectionDoctorTest {
    private val fine = Facts(pcName = "Desk", onLan = true, vpn = false, localNetworkAllowed = true, sameNetwork = true, probe = Probe.SILENT)
    private fun lead(f: Facts) = ConnectionDoctor.findings(f).first()

    @Test
    fun noWifiComesFirstWithWifiSettings() {
        val f = lead(fine.copy(onLan = false, sameNetwork = null, probe = Probe.NOT_TRIED))
        assertEquals("This phone isn't on Wi-Fi", f.title)
        assertEquals(Fix.WIFI_SETTINGS, f.fix)
        assertEquals(1, ConnectionDoctor.findings(fine.copy(onLan = false, sameNetwork = null, probe = Probe.NOT_TRIED)).size)
    }

    @Test
    fun permissionComesBeforeEverything() {
        assertEquals(Fix.ALLOW_LOCAL_NETWORK, lead(fine.copy(localNetworkAllowed = false, onLan = false)).fix)
    }

    @Test
    fun refusedMeansVyomaIsntRunningAndNothingElse() {
        val all = ConnectionDoctor.findings(fine.copy(probe = Probe.REFUSED, vpn = true))
        assertEquals(listOf("Vyoma isn't running on Desk"), all.map { it.title })
    }

    @Test
    fun answersSaysConnecting() {
        assertTrue(lead(fine.copy(probe = Probe.ANSWERS)).title.endsWith("connecting"))
    }

    @Test
    fun blockedIsTheFirewall() {
        assertTrue("firewall" in lead(fine.copy(probe = Probe.BLOCKED)).title)
    }

    @Test
    fun differentNetworkLeadsAndSilenceFollows() {
        val all = ConnectionDoctor.findings(fine.copy(sameNetwork = false))
        assertEquals(Fix.WIFI_SETTINGS, all.first().fix)
        assertEquals(Fix.USE_CABLE, all.last().fix)
    }

    @Test
    fun vpnIsNamedWhenThePcIsSilent() {
        val all = ConnectionDoctor.findings(fine.copy(vpn = true))
        assertEquals(Fix.VPN_SETTINGS, all.first().fix)
    }

    @Test
    fun alwaysSaysSomething() {
        assertTrue(ConnectionDoctor.findings(fine.copy(probe = Probe.NOT_TRIED, sameNetwork = null)).isNotEmpty())
    }

    @Test
    fun prefixes() {
        fun b(s: String) = InetAddress.getByName(s).address
        assertTrue(ConnectionDoctor.samePrefix(b("192.168.1.20"), b("192.168.1.200"), 24))
        assertFalse(ConnectionDoctor.samePrefix(b("192.168.1.20"), b("192.168.2.20"), 24))
        assertTrue(ConnectionDoctor.samePrefix(b("10.0.3.1"), b("10.0.200.9"), 16))
        assertFalse(ConnectionDoctor.samePrefix(b("10.0.0.1"), b("fd00::1"), 8))
    }
}
