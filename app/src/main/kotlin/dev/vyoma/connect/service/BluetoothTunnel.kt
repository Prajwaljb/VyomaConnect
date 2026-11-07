package dev.vyoma.connect.service

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.content.SharedPreferences
import android.content.pm.PackageManager
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runInterruptible
import java.io.Closeable
import java.io.InputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/** The Bluetooth switches (the backup link; steadier input), and each PC's address as its companion told us over Wi-Fi. */
object BluetoothFallback {
    private const val PREFS = "bluetooth_fallback"
    private const val ON = "on"
    private const val ADDRESS = "address."
    private const val STEADIER = "steadier"
    /** Before they shared [steadier]: the touchpad's switch, and [ON] also sent the controller over Bluetooth. */
    private const val OLD_POINTER = "pointer"

    @Volatile private var prefs: SharedPreferences? = null
    private val _enabled = MutableStateFlow(false)
    val enabled: StateFlow<Boolean> = _enabled.asStateFlow()
    private val _steadier = MutableStateFlow(false)
    /** The controller and touchpad over Bluetooth, off until asked for (Wi-Fi is fine for most). */
    val steadier: StateFlow<Boolean> = _steadier.asStateFlow()

    private fun load(context: Context): SharedPreferences = prefs ?: synchronized(this) {
        prefs ?: context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE).also {
            _enabled.value = it.getBoolean(ON, false)
            if (!it.contains(STEADIER)) it.edit().putBoolean(STEADIER, it.getBoolean(ON, false) || it.getBoolean(OLD_POINTER, false)).apply()
            _steadier.value = it.getBoolean(STEADIER, false)
            prefs = it
        }
    }

    fun init(context: Context): BluetoothFallback {
        load(context)
        return this
    }

    fun setEnabled(context: Context, on: Boolean) {
        load(context).edit().putBoolean(ON, on).apply()
        _enabled.value = on
    }

    fun setSteadier(context: Context, on: Boolean) {
        load(context).edit().putBoolean(STEADIER, on).apply()
        _steadier.value = on
    }

    fun remember(context: Context, pcId: String, address: String) {
        val p = load(context)
        if (p.getString(ADDRESS + pcId, null) != address) p.edit().putString(ADDRESS + pcId, address).apply()
    }

    /** PC id → Bluetooth address. */
    fun addresses(context: Context): Map<String, String> = load(context).all.mapNotNull { (k, v) ->
        if (k.startsWith(ADDRESS) && v is String) k.removePrefix(ADDRESS) to v else null
    }.toMap()

    fun permitted(context: Context) =
        context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

    private fun adapter(context: Context): BluetoothAdapter? = context.getSystemService(BluetoothManager::class.java)?.adapter

    /** What stands between the switch and working, for Settings. */
    enum class Readiness { NO_ADAPTER, NEEDS_PERMISSION, RADIO_OFF, PC_UNKNOWN, NOT_PAIRED, READY }

    @SuppressLint("MissingPermission") // Checked first.
    fun readiness(context: Context): Readiness {
        val adapter = adapter(context) ?: return Readiness.NO_ADAPTER
        if (!permitted(context)) return Readiness.NEEDS_PERMISSION
        if (!adapter.isEnabled) return Readiness.RADIO_OFF
        val known = addresses(context).values
        if (known.isEmpty()) return Readiness.PC_UNKNOWN
        return if (known.any { BluetoothAdapter.checkBluetoothAddress(it) && adapter.getRemoteDevice(it).bondState == BluetoothDevice.BOND_BONDED }) Readiness.READY
        else Readiness.NOT_PAIRED
    }

    internal fun adapterOrNull(context: Context) = adapter(context)

    /** A connected socket to [service] on this PC, if everything's in place for one. */
    @SuppressLint("MissingPermission") // permitted() is checked first.
    internal fun connect(context: Context, pcId: String?, service: UUID): BluetoothSocket? {
        if (!permitted(context)) return null
        val adapter = adapter(context)?.takeIf { it.isEnabled } ?: return null
        val address = addresses(context)[pcId]?.takeIf { BluetoothAdapter.checkBluetoothAddress(it) } ?: return null
        val peer = adapter.getRemoteDevice(address)
        if (peer.bondState != BluetoothDevice.BOND_BONDED) return null
        val radio = peer.createRfcommSocketToServiceRecord(service)
        return try {
            radio.connect()
            radio
        } catch (e: Exception) {
            runCatching { radio.close() }
            VLog.d("BluetoothTunnel", "no channel to the PC: ${e.message}")
            null
        }
    }
}

/** The controller's frames over Bluetooth, beside whatever the link is on (companion bluetooth.rs). */
class BluetoothPadChannel(private val context: Context, private val pcId: String?) : CompanionPlugin.RadioChannel {
    @Volatile private var socket: BluetoothSocket? = null

    override fun open(): Pair<OutputStream, InputStream>? {
        BluetoothFallback.init(context)
        if (!BluetoothFallback.steadier.value) return null
        val radio = BluetoothFallback.connect(context, pcId, SERVICE) ?: return null
        socket = radio
        return dev.vyoma.connect.gamepad.LatestFrame(radio.outputStream) to radio.inputStream
    }

    override fun close() {
        runCatching { socket?.close() }
        socket = null
    }

    private companion object {
        /** Must match PAD_SERVICE in the companion's bluetooth.rs. */
        val SERVICE: UUID = UUID.fromString("6f1d3e52-8a47-4c1b-9d06-2be7a5c4f803")
    }
}

/** The touchpad over Bluetooth (companion pointer.rs). Every frame counts here, so no [LatestFrame]. */
class BluetoothPointerChannel(private val context: Context, private val pcId: String?) : CompanionPlugin.RadioChannel {
    @Volatile private var socket: BluetoothSocket? = null

    override fun open(): Pair<OutputStream, InputStream>? {
        val radio = BluetoothFallback.connect(context, pcId, SERVICE) ?: return null
        socket = radio
        return radio.outputStream to radio.inputStream
    }

    override fun close() {
        runCatching { socket?.close() }
        socket = null
    }

    private companion object {
        /** Must match POINTER_SERVICE in the companion's bluetooth.rs. */
        val SERVICE: UUID = UUID.fromString("6f1d3e52-8a47-4c1b-9d06-2be7a5c4f804")
    }
}

/**
 * Keeps a paired PC reachable when Wi-Fi and the cable aren't there: opens the companion's Bluetooth
 * service and links to kdeconnectd through it (companion bluetooth.rs), the usual TLS link inside.
 * It only ever dials a PC that has been offline a while, and backs off, so it costs nothing while
 * Wi-Fi works and little while the PC is out of range. The better link replaces it (DeviceHub).
 */
class BluetoothTunnel(private val context: Context, private val hub: DeviceHub, private val scope: CoroutineScope) {
    private var job: Job? = null
    private val tunnels = ConcurrentHashMap<String, Job>()
    private val offlineSince = HashMap<String, Long>()
    private val nextTry = HashMap<String, Long>()
    private val wait = HashMap<String, Long>()

    fun start() {
        if (job != null) return
        BluetoothFallback.init(context)
        job = scope.launch(Dispatchers.IO) {
            while (isActive) {
                runCatching { tick() }.onFailure { VLog.w(TAG, "tick failed", it) }
                delay(TICK_MS)
            }
        }
    }

    fun stop() {
        job?.cancel()
        job = null
        tunnels.values.forEach { it.cancel() }
        tunnels.clear()
    }

    private fun now() = System.nanoTime() / 1_000_000

    @SuppressLint("MissingPermission") // permitted() is checked first.
    private suspend fun tick() {
        // Learn addresses whether or not the switch is on, so turning it on works with Wi-Fi gone.
        for (device in hub.devices.value.values) {
            val pc = device.identity.featuresOf ?: continue
            device.plugin<CompanionPlugin>(CompanionPlugin.ID)?.bluetoothAddress?.value?.let { BluetoothFallback.remember(context, pc, it) }
        }
        val on = BluetoothFallback.enabled.value
        if (!on) {
            hub.devices.value.values.forEach { it.disconnect(TransportKind.BLUETOOTH) }
            return
        }
        if (!BluetoothFallback.permitted(context)) return
        val adapter = BluetoothFallback.adapterOrNull(context) ?: return
        if (!adapter.isEnabled) return
        for ((pcId, address) in BluetoothFallback.addresses(context)) {
            val device = hub.device(pcId) ?: continue
            val state = device.state.value
            if (!state.isPaired) continue
            if (state.isReachable) {
                offlineSince.remove(pcId)
                nextTry.remove(pcId)
                wait.remove(pcId)
                if (state.transports == setOf(TransportKind.BLUETOOTH)) linkCompanion(pcId, address, adapter)
                continue
            }
            val since = offlineSince.getOrPut(pcId) { now() }
            // Let Wi-Fi and the cable have their go first.
            if (now() - since < GRACE_MS || now() < (nextTry[pcId] ?: 0L)) continue
            if (tunnels[pcId]?.isActive == true) continue
            if (!BluetoothAdapter.checkBluetoothAddress(address)) continue
            val peer = adapter.getRemoteDevice(address)
            if (peer.bondState != BluetoothDevice.BOND_BONDED) continue
            val backoff = (wait[pcId] ?: FIRST_RETRY_MS)
            nextTry[pcId] = now() + backoff
            wait[pcId] = (backoff * 2).coerceAtMost(MAX_RETRY_MS)
            tunnels[pcId] = scope.launch(Dispatchers.IO) { open(pcId, peer) }
        }
    }

    /** The PC is up over the radio: bring its companion over too (stats, controls, the controller). */
    @SuppressLint("MissingPermission")
    private fun linkCompanion(pcId: String, address: String, adapter: BluetoothAdapter) {
        val companion = hub.devices.value.values.firstOrNull { it.identity.companionOf == pcId && it.state.value.isPaired } ?: return
        val id = companion.identity.deviceId
        if (companion.state.value.isReachable || now() < (nextTry[id] ?: 0L) || tunnels[id]?.isActive == true) return
        nextTry[id] = now() + COMPANION_RETRY_MS
        tunnels[id] = scope.launch(Dispatchers.IO) { openCompanion(adapter.getRemoteDevice(address)) }
    }

    @SuppressLint("MissingPermission")
    private suspend fun open(pcId: String, peer: BluetoothDevice) {
        val radio = peer.createRfcommSocketToServiceRecord(SERVICE)
        var local: java.net.Socket? = null
        try {
            closingOnCancel(radio) { radio.connect() }
            VLog.i(TAG, "connected; linking through it")
            ServerSocket(0, 1, InetAddress.getLoopbackAddress()).use { server ->
                server.soTimeout = ACCEPT_TIMEOUT_MS
                hub.dialBluetooth(server.localPort, pcId)
                local = runInterruptible { server.accept() }
            }
            relay(local!!, radio)
        } catch (e: Exception) {
            VLog.d(TAG, "couldn't reach the PC: ${e.message}")
        } finally {
            runCatching { local?.close() }
            runCatching { radio.close() }
        }
    }

    /** As [open], but the companion dials: it calls every phone, so the phone's listener answers. */
    @SuppressLint("MissingPermission")
    private suspend fun openCompanion(peer: BluetoothDevice) {
        val radio = peer.createRfcommSocketToServiceRecord(COMPANION_SERVICE)
        var local: java.net.Socket? = null
        try {
            closingOnCancel(radio) { radio.connect() }
            VLog.i(TAG, "companion connected; linking through it")
            ServerSocket(0, 1, InetAddress.getLoopbackAddress()).use { server ->
                local = java.net.Socket(InetAddress.getLoopbackAddress(), server.localPort)
                val far = server.accept()
                far.tcpNoDelay = true
                hub.acceptBluetooth(far)
            }
            relay(local!!, radio)
        } catch (e: Exception) {
            VLog.d(TAG, "couldn't reach the companion: ${e.message}")
        } finally {
            runCatching { local?.close() }
            runCatching { radio.close() }
        }
    }

    private suspend fun relay(socket: java.net.Socket, radio: BluetoothSocket) {
        socket.tcpNoDelay = true
        // Whichever side ends first ends both, so a dropped radio link shows at once.
        val up = scope.launch(Dispatchers.IO) { pipe(socket.getInputStream(), radio.outputStream) }
        val down = scope.launch(Dispatchers.IO) { pipe(radio.inputStream, socket.getOutputStream()) }
        val both = listOf(up, down)
        try {
            while (both.all { it.isActive }) delay(PIPE_CHECK_MS)
        } finally {
            both.forEach { it.cancel() }
        }
    }

    private suspend fun closingOnCancel(socket: BluetoothSocket, block: () -> Unit) {
        val closer = kotlin.coroutines.coroutineContext[Job]?.invokeOnCompletion { runCatching { socket.close() } }
        try {
            runInterruptible { block() }
        } finally {
            closer?.dispose()
        }
    }

    private fun pipe(from: InputStream, to: OutputStream) {
        val buffer = ByteArray(16 * 1024)
        runCatching {
            while (true) {
                val n = from.read(buffer)
                if (n < 0) break
                to.write(buffer, 0, n)
                to.flush()
            }
        }
    }

    private companion object {
        const val TAG = "BluetoothTunnel"
        /** Must match SERVICE in the companion's bluetooth.rs. */
        val SERVICE: UUID = UUID.fromString("6f1d3e52-8a47-4c1b-9d06-2be7a5c4f801")
        val COMPANION_SERVICE: UUID = UUID.fromString("6f1d3e52-8a47-4c1b-9d06-2be7a5c4f802")
        const val TICK_MS = 5_000L
        /** The PC is known to be there, and its side may need a moment to notice Wi-Fi went. */
        const val COMPANION_RETRY_MS = 10_000L
        const val GRACE_MS = 15_000L
        const val FIRST_RETRY_MS = 20_000L
        const val MAX_RETRY_MS = 300_000L
        const val ACCEPT_TIMEOUT_MS = 10_000
        const val PIPE_CHECK_MS = 500L
    }
}
