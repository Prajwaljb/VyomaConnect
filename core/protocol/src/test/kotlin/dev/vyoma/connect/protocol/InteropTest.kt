package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.FileCredentialStore
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.FileTrustStore
import dev.vyoma.connect.protocol.device.HubEvent
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.plugin.builtin.BatteryState
import dev.vyoma.connect.protocol.plugin.builtin.IncomingFile
import dev.vyoma.connect.protocol.plugin.builtin.MprisPlugin
import dev.vyoma.connect.protocol.plugin.builtin.OutgoingFile
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.protocol.plugin.builtin.RunCommandPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SharePlugin
import dev.vyoma.connect.protocol.plugin.builtin.ShareSink
import dev.vyoma.connect.protocol.plugin.builtin.SystemVolumePlugin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
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

    /** Collects files the desktop shares with us. */
    private val received = java.util.concurrent.ConcurrentHashMap<String, ByteArray>()
    private val sink = object : ShareSink {
        override fun onText(from: Device, text: String) = Unit
        override fun onUrl(from: Device, url: String) = Unit
        override fun createFile(from: Device, name: String, size: Long, lastModified: Long?, folder: String?) = object : IncomingFile {
            val buffer = java.io.ByteArrayOutputStream()
            override val output = buffer
            override fun complete(): String { received[name] = buffer.toByteArray(); return name }
            override fun abort() = Unit
        }
    }

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
            pluginSpecs = listOf(
                PingPlugin.spec,
                BatteryPlugin.spec(MutableStateFlow(BatteryState(77, charging = true))),
                SharePlugin.spec(sink),
                MprisPlugin.spec,
                SystemVolumePlugin.spec,
                RunCommandPlugin.spec,
            ),
            scope = scope,
            deviceName = "Vyoma Interop Test",
            deviceType = DeviceType.PHONE,
            lanConfig = LanBackend.Config(listenUdp = false, tcpPorts = 1717..1764, broadcastTargets = { listOf(lanIp) }),
        )
    }

    /**
     * Keeps announcing: kdeconnectd ignores a broadcast that comes right after the last test's link
     * closed, and a phone would announce again anyway.
     */
    private suspend fun awaitDesktop(): Device = coroutineScope {
        val announcing = launch {
            while (true) {
                delay(2_000)
                hub.lan.broadcastIdentity()
            }
        }
        try {
            withTimeout(15_000) {
                hub.devices.first { desktopId in it }.getValue(desktopId).also { d -> d.state.first { it.isReachable } }
            }
        } finally {
            announcing.cancel()
        }
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

        val desktop = awaitDesktop()
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
        waitUntil { dbus(ourId, "isPaired") == "true" }

        desktop.unpair()
        waitUntil { dbus(ourId, "isPaired") == "false" }
        println("all interop checks passed against kdeconnectd")
    }

    /** The desktop reports its own battery once plugins load; widgets and the device page show it. */
    @Test
    fun desktopBatteryArrives() = runBlocking {
        val ourId = hub.localIdentity().deviceId
        hub.start()
        val desktop = awaitDesktop()
        if (!desktop.state.value.isPaired) {
            val request = async(start = CoroutineStart.UNDISPATCHED) { hub.events.filterIsInstance<HubEvent.PairRequested>().first() }
            cli("-d", ourId, "--pair")
            withTimeout(10_000) { request.await() }
            desktop.acceptPairing()
            withTimeout(5_000) { desktop.state.first { it.pairState == PairState.PAIRED } }
        }
        try {
            val battery = withTimeout(10_000) {
                desktop.state.first { BatteryPlugin.ID in it.plugins }
                desktop.plugin<BatteryPlugin>(BatteryPlugin.ID)!!.remote.first { it != null }
            }
            println("desktop battery: $battery")
        } finally {
            desktop.unpair()
            waitUntil { dbus(ourId, "isPaired") == "false" }
        }
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

    /** Payload transfers both ways plus read-only plugin state. Pairs a throwaway identity, unpairs at the end. */
    @Test
    fun payloadsAndPlugins() = runBlocking {
        val ourId = hub.localIdentity().deviceId
        hub.start()
        val desktop = awaitDesktop()
        if (!desktop.state.value.isPaired) {
            desktop.requestPairing()
            waitUntil { dbus(ourId, "isPairRequestedByPeer") == "true" }
            dbus(ourId, "acceptPairing")
            withTimeout(5_000) { desktop.state.first { it.pairState == PairState.PAIRED } }
        }
        withTimeout(5_000) { desktop.state.first { SharePlugin.ID in it.plugins } }
        try {
            // Phone -> PC: 3 MiB random file lands in the desktop's download folder intact.
            val data = ByteArray(3 * 1024 * 1024).also { java.util.Random(7).nextBytes(it) }
            val name = "vyoma-interop-${System.currentTimeMillis()}.bin"
            val share = desktop.plugin<SharePlugin>(SharePlugin.ID)!!
            val started = System.nanoTime()
            val sent = share.sendFiles(listOf(OutgoingFile(name, data.size.toLong(), null) { data.inputStream() }))
            assertEquals(1, sent)
            println("upload: ${data.size / 1024} KiB in ${(System.nanoTime() - started) / 1_000_000} ms")
            val downloads = File(ProcessBuilder("xdg-user-dir", "DOWNLOAD").start().inputStream.bufferedReader().readText().trim())
            val landed = File(downloads, name)
            waitUntil { landed.exists() && landed.length() == data.size.toLong() }
            assertTrue(landed.readBytes().contentEquals(data), "desktop received identical bytes")
            landed.delete()

            // PC -> phone.
            val outgoing = File(stateDir, "from-desktop-${System.currentTimeMillis()}.bin")
            val payload = ByteArray(2 * 1024 * 1024).also { java.util.Random(11).nextBytes(it) }
            outgoing.writeBytes(payload)
            cli("-d", ourId, "--share", outgoing.absolutePath)
            waitUntil(20_000) { received.containsKey(outgoing.name) }
            assertTrue(received.getValue(outgoing.name).contentEquals(payload), "we received identical bytes")
            outgoing.delete()

            // Read-only state from desktop plugins.
            delay(1500)
            desktop.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.sinks?.value?.let { println("sinks: ${it.map { s -> s.description }}") }
            desktop.plugin<MprisPlugin>(MprisPlugin.ID)?.players?.value?.let { println("players: ${it.values.map { p -> p.name + " / " + p.title }}") }
            desktop.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.commands?.value?.let { println("commands: ${it.map { c -> c.name }}") }
            println("desktop battery: ${desktop.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote?.value}")

            // Media control round trip, when a desktop player is around (play then restore).
            val mpris = desktop.plugin<MprisPlugin>(MprisPlugin.ID)!!
            val player = mpris.players.value.values.firstOrNull { it.hasMedia && !it.name.contains("kdeconnect", true) }
            if (player != null) {
                val wasPlaying = player.isPlaying
                mpris.action(player.name, MprisPlugin.MediaAction.PLAY_PAUSE)
                withTimeout(5_000) { mpris.players.first { it[player.name]?.isPlaying == !wasPlaying && System.currentTimeMillis() - it[player.name]!!.positionAt < 4_000 } }
                delay(800)
                val desktopState = ProcessBuilder("playerctl", "-a", "metadata", "--format", "{{status}}|{{title}}").start()
                    .inputStream.bufferedReader().readLines().firstOrNull { it.substringAfter('|') == player.title }?.substringBefore('|').orEmpty()
                mpris.action(player.name, MprisPlugin.MediaAction.PLAY_PAUSE)
                println("mpris: '${player.title}' toggled; desktop reported '$desktopState' (was playing=$wasPlaying)")
                assertEquals(if (wasPlaying) "Paused" else "Playing", desktopState)
            }
        } finally {
            desktop.unpair()
            waitUntil { dbus(ourId, "isPaired") == "false" }
        }
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
