package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.FileCredentialStore
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.FileTrustStore
import dev.vyoma.connect.protocol.device.HubEvent
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import java.io.File
import java.net.Inet4Address
import java.net.NetworkInterface
import java.util.concurrent.TimeUnit
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Talks to the real kdeconnectd on this machine. Run with:
 *   VYOMA_INTEROP=1 ./gradlew :core:protocol:test --tests '*InteropTest*'
 * The desktop's UDP 1716 is taken by kdeconnectd, so we don't listen for UDP; we
 * broadcast to the LAN address and let kdeconnectd connect to us.
 */
class InteropTest {
    private lateinit var scope: CoroutineScope
    private lateinit var hub: DeviceHub
    private val stateDir = File("build/interop").apply { mkdirs() }
    private lateinit var desktopId: String

    @Before
    fun setUp() {
        assumeTrue("interop disabled", System.getenv("VYOMA_INTEROP") == "1")
        desktopId = cli("--my-id").trim()
        val credStore = FileCredentialStore(File(stateDir, "credentials.p12"))
        val creds = credStore.load() ?: LocalCredentials.generate(DeviceIdentity.newDeviceId()).also(credStore::save)
        scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val lanIp = lanAddress()
        hub = DeviceHub(
            credentials = creds,
            trustStore = FileTrustStore(File(stateDir, "trusted.json")),
            pluginSpecs = listOf(PingPlugin.spec),
            scope = scope,
            deviceName = "Vyoma Interop Test",
            deviceType = DeviceType.PHONE,
            lanConfig = LanBackend.Config(listenUdp = false, tcpPorts = 1717..1764, broadcastTargets = { listOf(lanIp) }),
        )
    }

    @After
    fun tearDown() {
        if (!::hub.isInitialized) return
        hub.stop()
        scope.cancel()
    }

    @Test
    fun discoverPairPingUnpair() = runBlocking {
        val ourId = hub.localIdentity().deviceId
        hub.start()

        val desktop = withTimeout(15_000) {
            hub.devices.first { desktopId in it }.getValue(desktopId).also { d -> d.state.first { it.isReachable } }
        }
        println("link established with ${desktop.name}, v${desktop.identity.protocolVersion}")

        // 1) Desktop asks to pair, we accept; codes must match on both screens.
        if (desktop.state.value.isPaired) desktop.unpair()
        val request = async(start = CoroutineStart.UNDISPATCHED) { hub.events.filterIsInstance<HubEvent.PairRequested>().first() }
        cli("-d", ourId, "--pair")
        withTimeout(10_000) { request.await() }
        assertEquals(desktopKey(ourId), desktop.state.value.verificationKey, "verification keys differ (desktop-initiated)")
        desktop.acceptPairing()
        withTimeout(5_000) { desktop.state.first { it.pairState == PairState.PAIRED } }
        waitUntil { dbus(ourId, "isPaired") == "true" }

        // 2) Packets flow both ways once paired.
        val ping = async(start = CoroutineStart.UNDISPATCHED) { PingPlugin.received.first { it.deviceId == desktopId } }
        cli("-d", ourId, "--ping-msg", "hello from kdeconnectd")
        assertEquals("hello from kdeconnectd", withTimeout(5_000) { ping.await() }.message)

        // 3) Unpair from our side propagates.
        desktop.unpair()
        waitUntil { dbus(ourId, "isPaired") == "false" }

        // 4) We ask to pair, desktop accepts over D-Bus.
        desktop.requestPairing()
        waitUntil { dbus(ourId, "isPairRequestedByPeer") == "true" }
        assertEquals(desktopKey(ourId), desktop.state.value.verificationKey, "verification keys differ (phone-initiated)")
        dbus(ourId, "acceptPairing")
        withTimeout(5_000) { desktop.state.first { it.pairState == PairState.PAIRED } }

        desktop.unpair()
        waitUntil { dbus(ourId, "isPaired") == "false" }
        println("all interop checks passed against kdeconnectd")
    }

    /** Manual-IP / Tailscale path: we dial the desktop's TCP port, so we're the TLS server. */
    @Test
    fun directConnectAsTlsServer() = runBlocking {
        hub.lan.start()
        hub.lan.connectTo(lanAddress(), 1716)
        val desktop = withTimeout(10_000) {
            hub.devices.first { desktopId in it }.getValue(desktopId).also { d -> d.state.first { it.isReachable } }
        }
        assertEquals(8, desktop.identity.protocolVersion)
        assertTrue(desktop.identity.incomingCapabilities.isNotEmpty(), "full identity received over TLS")
    }

    private fun desktopKey(ourId: String) = dbus(ourId, "verificationKey").trim().uppercase()

    private fun dbus(ourId: String, member: String): String {
        val proc = ProcessBuilder(
            "qdbus6", "org.kde.kdeconnect", "/modules/kdeconnect/devices/$ourId", "org.kde.kdeconnect.device.$member",
        ).redirectErrorStream(true).start()
        proc.waitFor(10, TimeUnit.SECONDS)
        return proc.inputStream.bufferedReader().readText().trim()
    }

    private fun waitUntil(timeoutMs: Long = 10_000, check: () -> Boolean) {
        val end = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < end) {
            if (check()) return
            Thread.sleep(200)
        }
        error("condition not met in ${timeoutMs}ms")
    }

    private fun cli(vararg args: String): String {
        val proc = ProcessBuilder(listOf("kdeconnect-cli") + args).redirectErrorStream(true).start()
        proc.waitFor(15, TimeUnit.SECONDS)
        return proc.inputStream.bufferedReader().readText()
    }

    private fun lanAddress() = NetworkInterface.networkInterfaces().toList()
        .filter { it.isUp && !it.isLoopback && !it.isVirtual && !it.name.startsWith("docker") }
        .flatMap { it.inetAddresses.toList() }
        .first { it is Inet4Address }
}
