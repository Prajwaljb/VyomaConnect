package dev.vyoma.connect.demo

import android.content.Context
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.crypto.FileCredentialStore
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.TrustStore
import dev.vyoma.connect.protocol.device.TrustedDevice
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.link.TransportKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.io.File

/**
 * The demo build (`assembleDemo`): a simulated laptop and its companion run inside the app and
 * connect over loopback, already paired, so every screen has something live to show with no PC
 * and no network. The LAN backend is kept off the network entirely: no broadcasts, no UDP
 * listener, no manual hosts; mDNS and the USB tunnel aren't started (see LinkService).
 */
object Demo {
    const val PC_ID = "vyoma_demo_laptop_0000000000000001"
    const val COMPANION_ID = "vyoma_demo_companion_00000000001"

    class Peers internal constructor(internal val pc: LocalCredentials, internal val companion: LocalCredentials)

    /** Loads (or makes, once) the simulated devices' keys and pins them as paired. Blocking: call off the main thread. */
    fun prepare(dir: File, trust: TrustStore): Peers {
        fun creds(name: String, id: String) = FileCredentialStore(File(dir, "$name.p12")).let { store ->
            store.load() ?: LocalCredentials.generate(id).also(store::save)
        }
        val peers = Peers(creds("demo-pc", PC_ID), creds("demo-companion", COMPANION_ID))
        trust.put(TrustedDevice(PC_ID, DemoPc.NAME, DeviceType.LAPTOP, peers.pc.certificate))
        trust.put(TrustedDevice(COMPANION_ID, DemoPc.NAME, DeviceType.LAPTOP, peers.companion.certificate, companionOf = PC_ID))
        return peers
    }

    /** Listens on loopback only in effect: nothing is announced, heard or dialled on the network. */
    fun lanConfig(base: LanBackend.Config) = base.copy(
        listenUdp = false,
        broadcastTargets = { emptyList() },
        manualHosts = { emptyList() },
        // Loopback reads as the USB tunnel otherwise; the pretend laptop is "on Wi-Fi".
        transportOf = { _, _ -> TransportKind.LAN },
    )

    /**
     * The laptop stays away (paired, never connecting), to show the away screens: made by
     * `adb shell touch /sdcard/Android/data/dev.vyoma.connect.demo/files/away`, read at start.
     */
    fun away(context: Context): Boolean = context.getExternalFilesDir(null)?.let { File(it, "away").exists() } == true

    fun start(hub: DeviceHub, peers: Peers, scope: CoroutineScope) {
        val pc = DemoPc(peers.pc)
        pc.run(scope) { hub.lan.tcpPort }
        DemoCompanion(peers.companion, pc).run(scope) { hub.lan.tcpPort }
    }

    /**
     * A starter clipboard history (first run only), so the Clipboard tab isn't empty. Written in
     * AndroidClipboard's own file format before it loads; the phone's real clipboard is untouched.
     */
    fun seedClipboard(context: Context) {
        val file = File(context.filesDir, "clipboard.json")
        if (file.exists()) return
        val now = System.currentTimeMillis()
        val clips = listOf(
            Triple("https://kdeconnect.kde.org/", 4L, PC_ID),
            Triple("Meeting moved to 4:30, room B", 26L, null),
            Triple("git clone https://invent.kde.org/network/kdeconnect-kde.git", 75L, PC_ID),
            Triple("Flat 12B, Lake View Apartments", 190L, null),
            Triple("cargo build --release && ./target/release/orbit", 420L, PC_ID),
            Triple("Remember: water the plants", 1440L, null),
        )
        val json = JsonArray(clips.map { (text, minutesAgo, origin) ->
            buildJsonObject {
                put("text", text)
                put("timestamp", now - minutesAgo * 60_000)
                origin?.let { put("origin", it) }
            }
        })
        runCatching { file.writeText(json.toString()) }
    }
}
