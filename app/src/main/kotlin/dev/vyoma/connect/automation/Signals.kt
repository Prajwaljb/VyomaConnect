package dev.vyoma.connect.automation

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.os.Handler
import android.os.Looper
import dev.vyoma.connect.AppContainer
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.plugin.builtin.CallEvent
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.transformLatest
import kotlinx.coroutines.launch

enum class CallState { NONE, RINGING, TALKING }
enum class WifiState { OFF, OTHER, HOME }

/** Everything rules can react to, at one moment. Unknown PC values (it's away) are null. */
data class Snapshot(
    val pcConnected: Boolean = false,
    /** When the PC went away (wall clock), while it's away. */
    val pcAwaySince: Long? = null,
    val call: CallState = CallState.NONE,
    val phoneBattery: Int? = null,
    val phoneCharging: Boolean = false,
    val pcBattery: Int? = null,
    val pcCharging: Boolean? = null,
    val pcLocked: Boolean? = null,
    val pcPlaying: Boolean = false,
    val headphones: Boolean = false,
    val wifi: WifiState = WifiState.OFF,
    /** This Wi-Fi's fingerprint, null off Wi-Fi. */
    val wifiPrint: String? = null,
    /** How the PC's link runs, null while it's away. */
    val pcVia: TransportKind? = null,
    /** Addresses of the Bluetooth devices connected now. */
    val bluetooth: Set<String> = emptySet(),
    /** False until every input has reported once. */
    val ready: Boolean = false,
)

/**
 * The live inputs, merged into one [Snapshot] flow. Everything is a callback or an existing flow:
 * nothing polls. Started once, with the link service; the time is the engine's business.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class Signals(private val context: Context, private val c: AppContainer, private val scope: CoroutineScope) {
    private val prefs = context.getSharedPreferences("automation", Context.MODE_PRIVATE)

    private val call = MutableStateFlow(CallState.NONE)
    private val headphones = MutableStateFlow(false)
    private val bluetooth = MutableStateFlow<Set<String>>(emptySet())
    /** This Wi-Fi network's fingerprint (its subnet and router), null off Wi-Fi. */
    private val wifiPrint = MutableStateFlow<String?>(null)
    private val homes = MutableStateFlow(prefs.getStringSet(HOMES, emptySet()).orEmpty())

    /** The PC rules act on: the first paired one that's connected (as the widgets pick it). */
    private val pc: Flow<Device?> = c.hub.flatMapLatest { hub ->
        hub?.devices?.flatMapLatest { devices ->
            if (devices.isEmpty()) flowOf(null)
            else combine(devices.values.map { d -> d.state.map { s -> d.takeIf { s.isPaired && s.isReachable && !s.isCompanion } } }) { list ->
                list.filterNotNull().minByOrNull { d -> d.state.value.transports.minOfOrNull { it.ordinal } ?: Int.MAX_VALUE }
            }
        } ?: flowOf(null)
    }.distinctUntilChanged()

    val snapshot: StateFlow<Snapshot> by lazy { build() }

    private fun build(): StateFlow<Snapshot> {
        watchCalls()
        watchHeadphones()
        watchBluetooth()
        watchWifi()
        learnHome()

        // A Wi-Fi/USB swap drops the link for a moment: "away" only after it's stayed gone 20 s.
        val link = pc.map { it != null }.distinctUntilChanged().transformLatest { up ->
            if (up) emit(true to null) else {
                val since = System.currentTimeMillis()
                delay(AWAY_GRACE_MS)
                emit(false to since)
            }
        }
        val pcState = pc.flatMapLatest { d ->
            // Held through a link swap, like [link], or a swap reads as the PC's battery and lock changing.
            if (d == null) flow { delay(AWAY_GRACE_MS); emit(Triple(null, null, null)) }
            else d.state.map { it.plugins }.distinctUntilChanged().flatMapLatest {
                val battery = d.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote ?: flowOf(null)
                val locked = d.plugin<LockPlugin>(LockPlugin.ID)?.locked ?: flowOf(null)
                combine(battery, locked) { b, l -> Triple(b?.charge, b?.charging, l) }
            }
        }
        val playing = combine(c.media.nowPlaying.map { it?.player?.isPlaying == true }, pc.map { it != null }) { playing, up -> playing to up }
            .transformLatest { (playing, up) ->
                if (!playing && !up) delay(AWAY_GRACE_MS)
                emit(playing)
            }
            .distinctUntilChanged()
        val wifi = combine(wifiPrint, homes) { print, homes -> if (print == null) WifiState.OFF else if (print in homes) WifiState.HOME else WifiState.OTHER }
        // Cable in or out swaps the link over a few seconds: a rule shouldn't run for the gap.
        val via = pc.flatMapLatest { d -> d?.state?.map { it.transports.minOrNull() } ?: flowOf(null) }.distinctUntilChanged()
            .transformLatest { delay(VIA_SETTLE_MS); emit(it) }.onStart { emit(null) }
        val connections = combine(wifiPrint, via, bluetooth) { print, route, nearby -> Connections(print, route, nearby) }
        val phone = c.battery.state
        val local = combine(call, headphones, wifi, phone) { call, phones, wifi, battery -> Local(call, phones, wifi, battery?.charge, battery?.charging == true) }
        return combine(link, pcState, playing, local, connections) { (up, since), (charge, charging, locked), play, l, conn ->
            Snapshot(
                pcConnected = up, pcAwaySince = since, call = l.call,
                phoneBattery = l.battery, phoneCharging = l.charging,
                pcBattery = charge, pcCharging = charging, pcLocked = locked, pcPlaying = play && up,
                headphones = l.headphones, wifi = l.wifi,
                wifiPrint = conn.wifiPrint, pcVia = conn.pcVia, bluetooth = conn.bluetooth, ready = true,
            )
        }.distinctUntilChanged().stateIn(scope, SharingStarted.Eagerly, Snapshot())
    }

    private data class Local(val call: CallState, val headphones: Boolean, val wifi: WifiState, val battery: Int?, val charging: Boolean)

    private data class Connections(val wifiPrint: String?, val pcVia: TransportKind?, val bluetooth: Set<String>)

    private fun watchCalls() = scope.launch {
        c.calls.events.collect { e ->
            call.value = when (e) {
                is CallEvent.Ringing -> CallState.RINGING
                is CallEvent.Talking -> CallState.TALKING
                is CallEvent.Ended -> CallState.NONE
                else -> call.value
            }
        }
    }

    private fun watchHeadphones() {
        val audio = context.getSystemService(AudioManager::class.java)
        fun check() {
            headphones.value = audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS).any { it.type in HEADPHONES }
        }
        audio.registerAudioDeviceCallback(object : AudioDeviceCallback() {
            override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = check()
            override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = check()
        }, Handler(Looper.getMainLooper()))
        check()
    }

    private fun bluetoothAllowed() = context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

    /**
     * Every device's connect and disconnect, audio or not; what's already connected comes from
     * [lookForConnected]. Exported: these come from the Bluetooth process, which a private receiver
     * never hears. Only the system can send them (protected broadcasts).
     */
    private fun watchBluetooth() {
        val filter = IntentFilter().apply {
            addAction(BluetoothDevice.ACTION_ACL_CONNECTED)
            addAction(BluetoothDevice.ACTION_ACL_DISCONNECTED)
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
        }
        context.registerReceiver(object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                @Suppress("DEPRECATION")
                val address = intent.getParcelableExtra<BluetoothDevice>(BluetoothDevice.EXTRA_DEVICE)?.address
                when (intent.action) {
                    BluetoothDevice.ACTION_ACL_CONNECTED -> address?.let { a -> bluetooth.update { it + a } }
                    BluetoothDevice.ACTION_ACL_DISCONNECTED -> address?.let { a -> bluetooth.update { it - a } }
                    BluetoothAdapter.ACTION_STATE_CHANGED ->
                        if (intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.STATE_OFF) != BluetoothAdapter.STATE_ON) bluetooth.value = emptySet()
                }
            }
        }, filter, Context.RECEIVER_EXPORTED)
        lookForConnected()
    }

    /** Android only tells a running app about changes: ask its audio profiles what's connected already. */
    @SuppressLint("MissingPermission")
    private fun lookForConnected() {
        if (!bluetoothAllowed()) return
        val adapter = context.getSystemService(BluetoothManager::class.java).adapter ?: return
        val profiles = buildList {
            add(BluetoothProfile.A2DP)
            add(BluetoothProfile.HEADSET)
            if (Build.VERSION.SDK_INT >= 33) add(BluetoothProfile.LE_AUDIO)
        }
        for (profile in profiles) runCatching {
            adapter.getProfileProxy(context, object : BluetoothProfile.ServiceListener {
                override fun onServiceConnected(which: Int, proxy: BluetoothProfile) {
                    val found = runCatching { proxy.connectedDevices.map { it.address } }.getOrDefault(emptyList())
                    bluetooth.update { it + found }
                    adapter.closeProfileProxy(which, proxy)
                }
                override fun onServiceDisconnected(which: Int) = Unit
            }, profile)
        }
    }

    /** (address, name) of each paired device; empty without the permission. */
    @SuppressLint("MissingPermission")
    fun pairedBluetooth(): List<Pair<String, String>> {
        if (!bluetoothAllowed()) return emptyList()
        // The permission may only just have been given.
        lookForConnected()
        val adapter = context.getSystemService(BluetoothManager::class.java).adapter ?: return emptyList()
        return runCatching { adapter.bondedDevices.map { it.address to (it.name ?: it.address) }.sortedBy { it.second.lowercase() } }.getOrDefault(emptyList())
    }

    /**
     * Which Wi-Fi this is, without the location permission an SSID needs: its subnet and router
     * address. Home is learned, not set: whichever network the PC is reached on over Wi-Fi.
     */
    private fun watchWifi() {
        val cm = context.getSystemService(ConnectivityManager::class.java)
        val prints = java.util.concurrent.ConcurrentHashMap<Network, String>()
        val request = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI).build()
        runCatching {
            cm.registerNetworkCallback(request, object : ConnectivityManager.NetworkCallback() {
                override fun onLinkPropertiesChanged(network: Network, lp: LinkProperties) {
                    fingerprint(lp)?.let { prints[network] = it } ?: prints.remove(network)
                    wifiPrint.value = prints.values.firstOrNull()
                }
                override fun onLost(network: Network) {
                    prints.remove(network)
                    wifiPrint.value = prints.values.firstOrNull()
                }
            })
        }
    }

    private fun fingerprint(lp: LinkProperties): String? {
        val subnet = lp.linkAddresses.firstOrNull { it.address is java.net.Inet4Address }?.let { a ->
            val bytes = a.address.address
            val bits = a.prefixLength
            val masked = bytes.mapIndexed { i, b -> val keep = (bits - i * 8).coerceIn(0, 8); (b.toInt() and (0xFF shl (8 - keep)) and 0xFF) }
            masked.joinToString(".") + "/" + bits
        } ?: return null
        val router = lp.routes.firstOrNull { it.isDefaultRoute && it.gateway is java.net.Inet4Address }?.gateway?.hostAddress
        val dhcp = if (Build.VERSION.SDK_INT >= 30) lp.dhcpServerAddress?.hostAddress else null
        return listOfNotNull(subnet, router, dhcp).joinToString("|")
    }

    /** While the PC is connected over Wi-Fi, this network is home (a few remembered, newest kept). */
    private fun learnHome() = scope.launch {
        combine(pc.flatMapLatest { d -> d?.state?.map { TransportKind.LAN in it.transports } ?: flowOf(false) }, wifiPrint) { lan, print -> print.takeIf { lan } }
            .distinctUntilChanged()
            .collect { print ->
                if (print == null || print in homes.value) return@collect
                val next = (homes.value + print).toList().takeLast(MAX_HOMES).toSet()
                prefs.edit().putStringSet(HOMES, next).apply()
                homes.value = next
            }
    }

    private companion object {
        const val HOMES = "home_wifi"
        const val MAX_HOMES = 3
        const val AWAY_GRACE_MS = 20_000L
        const val VIA_SETTLE_MS = 3_000L
        val HEADPHONES = setOf(
            AudioDeviceInfo.TYPE_WIRED_HEADSET, AudioDeviceInfo.TYPE_WIRED_HEADPHONES, AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
            AudioDeviceInfo.TYPE_USB_HEADSET, AudioDeviceInfo.TYPE_BLE_HEADSET,
        )
    }
}
