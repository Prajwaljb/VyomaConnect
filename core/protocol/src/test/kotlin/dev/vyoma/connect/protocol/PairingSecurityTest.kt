package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.device.TrustStore
import dev.vyoma.connect.protocol.device.TrustedDevice
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.put
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.InetAddress
import java.net.Socket
import java.util.concurrent.atomic.AtomicInteger

/**
 * What someone else on the network can try while a phone pairs: hubs talking over loopback, with an
 * attacker who took the PC's id from its broadcast and made a certificate of their own for it.
 */
class PairingSecurityTest {
    private class Memory : TrustStore {
        val map = java.util.concurrent.ConcurrentHashMap<String, TrustedDevice>()
        override fun all() = map.values.toList()
        override fun get(deviceId: String) = map[deviceId]
        override fun put(device: TrustedDevice) { map[device.deviceId] = device }
        override fun remove(deviceId: String) { map.remove(deviceId) }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val lo = InetAddress.getLoopbackAddress()
    private val hubs = mutableListOf<DeviceHub>()
    private val trust = Memory()
    private val pcId = DeviceIdentity.newDeviceId()
    private val phoneCreds = LocalCredentials.generate(DeviceIdentity.newDeviceId())
    private val pcCreds = LocalCredentials.generate(pcId)
    private val attackerCreds = LocalCredentials.generate(pcId)
    private val phoneId get() = phoneCreds.deviceId

    private fun hub(creds: LocalCredentials, store: TrustStore = Memory()): DeviceHub {
        val first = nextPort.getAndAdd(10)
        return DeviceHub(
            credentials = creds,
            trustStore = store,
            pluginSpecs = listOf(PingPlugin.spec),
            scope = scope,
            deviceName = "test",
            deviceType = DeviceType.DESKTOP,
            lanConfig = LanBackend.Config(listenUdp = false, tcpPorts = first until first + 10, broadcastTargets = { emptyList() }),
        ).also { it.lan.start(); hubs += it }
    }

    @After fun tearDown() {
        hubs.forEach { it.stop() }
        scope.cancel()
    }

    private suspend fun waitFor(check: () -> Boolean) = withTimeout(8_000) { while (!check()) delay(20) }

    private suspend fun DeviceHub.link(to: DeviceHub, id: String) {
        lan.connectTo(lo, to.lan.tcpPort)
        waitFor { device(id) != null }
    }

    /** The PC linked and asking; the phone's prompt is on screen. */
    private suspend fun promptFromPc(phone: DeviceHub, pc: DeviceHub) {
        pc.link(phone, phoneId)
        waitFor { phone.device(pcId)?.state?.value?.isReachable == true }
        pc.device(phoneId)!!.requestPairing()
        waitFor { phone.device(pcId)?.state?.value?.pairState == PairState.REQUESTED_BY_PEER }
    }

    private suspend fun pingGetsThrough(from: DeviceHub): Boolean {
        val tag = DeviceIdentity.newDeviceId()
        val got = scope.async { withTimeoutOrNull(1_500) { PingPlugin.received.filter { it.message == tag }.first() } }
        delay(100)
        from.device(phoneId)?.send(packet(PacketTypes.PING) { put("message", tag) })
        return got.await() != null
    }

    @Test fun aLinkArrivingDuringThePromptCantBecomeWhoGetsTrusted() = runBlocking {
        val phone = hub(phoneCreds, trust)
        val pc = hub(pcCreds)
        val attacker = hub(attackerCreds)
        promptFromPc(phone, pc)
        val shown = phone.device(pcId)!!.state.value.verificationKey
        assertEquals(pc.device(phoneId)!!.state.value.verificationKey, shown)

        attacker.link(phone, phoneId)
        delay(2_500) // past the time a replaced link is allowed to linger
        assertEquals(shown, phone.device(pcId)!!.state.value.verificationKey)
        phone.device(pcId)!!.acceptPairing()
        waitFor { phone.device(pcId)!!.state.value.isPaired }

        assertEquals(pcCreds.certificate, trust.get(pcId)?.certificate)
        assertFalse(pingGetsThrough(attacker))
        assertTrue(pingGetsThrough(pc))
    }

    @Test fun aLinkArrivingJustBeforeTheAnswerIsntObeyedAfterIt() = runBlocking {
        val phone = hub(phoneCreds, trust)
        val pc = hub(pcCreds)
        val attacker = hub(attackerCreds)
        promptFromPc(phone, pc)
        attacker.link(phone, phoneId)
        delay(150)
        phone.device(pcId)!!.acceptPairing()
        waitFor { phone.device(pcId)!!.state.value.isPaired }

        assertEquals(pcCreds.certificate, trust.get(pcId)?.certificate)
        assertFalse(pingGetsThrough(attacker))
    }

    @Test fun someoneHoldingAnIdDoesntKeepTheRealDeviceOut() = runBlocking {
        val phone = hub(phoneCreds, trust)
        val pc = hub(pcCreds)
        val attacker = hub(attackerCreds)
        attacker.link(phone, phoneId)
        waitFor { phone.device(pcId)?.state?.value?.isReachable == true }
        promptFromPc(phone, pc)
        phone.device(pcId)!!.acceptPairing()
        waitFor { phone.device(pcId)!!.state.value.isPaired }

        assertEquals(pcCreds.certificate, trust.get(pcId)?.certificate)
        assertFalse(pingGetsThrough(attacker))
        assertTrue(pingGetsThrough(pc))
    }

    @Test fun aPairedDeviceRefusesAnyOtherCertificate() = runBlocking {
        val phone = hub(phoneCreds, trust)
        val pc = hub(pcCreds)
        val attacker = hub(attackerCreds)
        promptFromPc(phone, pc)
        phone.device(pcId)!!.acceptPairing()
        waitFor { phone.device(pcId)!!.state.value.isPaired }

        attacker.link(phone, phoneId)
        assertFalse(pingGetsThrough(attacker))
        assertTrue(pingGetsThrough(pc))
    }

    @Test fun aPairingRequestMustCarryATimeNearOurs() = runBlocking {
        val phone = hub(phoneCreds)
        val pc = hub(pcCreds)
        pc.link(phone, phoneId)
        waitFor { phone.device(pcId) != null }
        val now = System.currentTimeMillis() / 1000

        suspend fun ask(timestamp: Long?): PairState {
            pc.device(phoneId)!!.send(packet(PacketTypes.PAIR) {
                put("pair", true)
                timestamp?.let { put("timestamp", it) }
            })
            delay(250)
            return phone.device(pcId)!!.state.value.pairState
        }
        assertEquals(PairState.UNPAIRED, ask(1))
        assertEquals(PairState.UNPAIRED, ask(now + 7200))
        assertEquals(PairState.UNPAIRED, ask(null))
        assertEquals(PairState.REQUESTED_BY_PEER, ask(now - 60))
        assertNotNull(phone.device(pcId)!!.state.value.verificationKey)
    }

    @Test fun idleConnectionsDontStallTheApp() = runBlocking {
        val phone = hub(phoneCreds)
        val sockets = List(70) { Socket(lo, phone.lan.tcpPort) }
        delay(400)
        val start = System.nanoTime()
        withContext(Dispatchers.IO) {}
        val waited = (System.nanoTime() - start) / 1_000_000
        sockets.forEach { it.close() }
        assertTrue("waited $waited ms for a thread", waited < 1_000)
    }

    @Test fun anUnpairedPeerCantSendHugePackets() = runBlocking {
        val phone = hub(phoneCreds)
        val pc = hub(pcCreds)
        pc.link(phone, phoneId)
        waitFor { phone.device(pcId)?.state?.value?.isReachable == true }
        pc.device(phoneId)!!.send(packet(PacketTypes.PING) { put("message", "x".repeat(128 * 1024)) })
        waitFor { phone.device(pcId)?.state?.value?.isReachable == false }
    }

    @Test fun strangersCantGrowTheDeviceListForever() = runBlocking {
        val phone = hub(phoneCreds)
        val strangers = List(12) { hub(LocalCredentials.generate(DeviceIdentity.newDeviceId())) }
        for (s in strangers) {
            s.link(phone, phoneId)
            delay(100)
        }
        assertTrue("${phone.devices.value.size} devices", phone.devices.value.size <= 8)
        // Whoever came last is still in the list.
        assertNotNull(phone.device(strangers.last().localDeviceId))
        assertNull(phone.device(strangers.first().localDeviceId))
    }

    private companion object {
        val nextPort = AtomicInteger(18100)
    }
}
