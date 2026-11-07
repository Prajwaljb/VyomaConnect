package dev.vyoma.connect.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import android.os.ParcelFileDescriptor
import android.system.Os
import android.system.OsConstants
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.link.LanBackend
import java.net.Inet4Address
import java.net.InetAddress
import java.net.NetworkInterface
import java.net.Socket

/** Android specifics for the LAN backend: broadcast targets, multicast lock, keep-alive, network events. */
class AndroidNetwork(private val context: Context) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val wifi = context.getSystemService(WifiManager::class.java)
    private var multicastLock: WifiManager.MulticastLock? = null
    private var callback: ConnectivityManager.NetworkCallback? = null

    /** Manual hosts (Tailscale IPs or names, other subnets), mirrored from settings by the service. */
    @Volatile var customHosts: List<String> = emptyList()

    fun lanConfig() = LanBackend.Config(
        broadcastTargets = ::broadcastTargets,
        tuneSocket = ::tuneSocket,
        route = ::route,
        manualHosts = ::resolveCustomHosts,
    )

    /** Blocking DNS for names like MagicDNS; called from the hub's IO scope. */
    private fun resolveCustomHosts(): List<InetAddress> =
        customHosts.mapNotNull { host -> runCatching { InetAddress.getByName(host) }.getOrNull() }

    /**
     * Sends LAN traffic over the physical network that owns the target's subnet, bypassing any
     * VPN. Tailscale and other VPN addresses fall through to the default (VPN) route.
     */
    private fun route(socket: Any, target: InetAddress) {
        val network = lanNetworkFor(target) ?: return
        runCatching {
            when (socket) {
                is Socket -> network.bindSocket(socket)
                is java.net.DatagramSocket -> network.bindSocket(socket)
            }
        }.onFailure { VLog.d(TAG, "can't bind to LAN for ${target.hostAddress}: ${it.message}") }
    }

    private fun lanNetworkFor(target: InetAddress): Network? {
        @Suppress("DEPRECATION")
        val candidates = connectivity.allNetworks.filter { n ->
            val caps = connectivity.getNetworkCapabilities(n) ?: return@filter false
            !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) &&
                (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) ||
                    caps.hasTransport(NetworkCapabilities.TRANSPORT_USB))
        }
        if (target.hostAddress == "255.255.255.255") {
            return candidates.firstOrNull { connectivity.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true }
                ?: candidates.firstOrNull()
        }
        return candidates.firstOrNull { n ->
            connectivity.getLinkProperties(n)?.linkAddresses?.any { la -> sameSubnet(la.address, target, la.prefixLength) } == true
        }
    }

    private fun sameSubnet(a: InetAddress, b: InetAddress, prefix: Int): Boolean {
        val x = a.address
        val y = b.address
        if (x.size != y.size) return false
        var bits = prefix
        for (i in x.indices) {
            if (bits <= 0) return true
            val mask = if (bits >= 8) 0xFF else (0xFF shl (8 - bits)) and 0xFF
            if ((x[i].toInt() and mask) != (y[i].toInt() and mask)) return false
            bits -= 8
        }
        return true
    }

    /**
     * 255.255.255.255 doesn't leave every interface on Android (notably the hotspot AP), so
     * also send to each IPv4 interface's directed broadcast address, plus manual hosts.
     */
    private fun broadcastTargets(): Collection<InetAddress> {
        val targets = linkedSetOf<InetAddress>(InetAddress.getByName("255.255.255.255"))
        runCatching {
            for (iface in NetworkInterface.getNetworkInterfaces()) {
                if (!iface.isUp || iface.isLoopback) continue
                iface.interfaceAddresses.mapNotNullTo(targets) { addr ->
                    addr.broadcast?.takeIf { addr.address is Inet4Address }
                }
            }
        }
        targets += resolveCustomHosts()
        return targets
    }

    /**
     * Keep-alive tuned for a phone: probe after 2 min idle, every 10 s, give up after 3.
     * Dead links surface within ~150 s without waking the radio constantly (network changes
     * and our own sends catch most drops sooner).
     */
    private fun tuneSocket(socket: Socket) {
        socket.tcpNoDelay = true
        socket.keepAlive = true
        runCatching {
            ParcelFileDescriptor.fromSocket(socket).use { pfd ->
                val fd = pfd.fileDescriptor
                Os.setsockoptInt(fd, OsConstants.IPPROTO_TCP, TCP_KEEPIDLE, 120)
                Os.setsockoptInt(fd, OsConstants.IPPROTO_TCP, TCP_KEEPINTVL, 10)
                Os.setsockoptInt(fd, OsConstants.IPPROTO_TCP, TCP_KEEPCNT, 3)
                // Keep-alive stops while a write waits for its ack: a PC that lost power would look alive for ~15 min.
                Os.setsockoptInt(fd, OsConstants.IPPROTO_TCP, TCP_USER_TIMEOUT, 150_000)
            }
        }.onFailure { VLog.d(TAG, "keep-alive tuning unavailable: ${it.message}") }
    }

    /** Lets UDP broadcasts reach us while Wi-Fi power-saves. Held only while the app is on screen. */
    fun acquireMulticast() {
        if (multicastLock?.isHeld == true) return
        multicastLock = wifi.createMulticastLock("vyoma").apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    fun releaseMulticast() {
        multicastLock?.takeIf { it.isHeld }?.release()
        multicastLock = null
    }

    /** Calls [onAvailable] when a usable network appears or changes (our cue to re-announce), [onLost] when one drops. */
    fun watch(onAvailable: () -> Unit, onLost: () -> Unit) {
        unwatch()
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = onAvailable()
            override fun onLost(network: Network) = onLost()
            override fun onLinkPropertiesChanged(network: Network, linkProperties: android.net.LinkProperties) = onAvailable()
        }
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
            .addTransportType(NetworkCapabilities.TRANSPORT_VPN)
            .addTransportType(NetworkCapabilities.TRANSPORT_USB)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN) // on by default, and no VPN would match
            .build()
        connectivity.registerNetworkCallback(request, cb)
        callback = cb
    }

    fun unwatch() {
        callback?.let { runCatching { connectivity.unregisterNetworkCallback(it) } }
        callback = null
    }

    private var lanCallback: ConnectivityManager.NetworkCallback? = null

    /** True while the phone is on Wi-Fi or Ethernet (VPNs and mobile data don't count). */
    fun onLan(): Boolean {
        @Suppress("DEPRECATION")
        return connectivity.allNetworks.any { n -> connectivity.getNetworkCapabilities(n)?.let(::isLan) == true }
    }

    private fun isLan(c: NetworkCapabilities) = !c.hasTransport(NetworkCapabilities.TRANSPORT_VPN) &&
        (c.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || c.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET))

    /**
     * Calls [onChange] whenever being on Wi-Fi/Ethernet changes. Kept from the callbacks' own
     * networks: in onLost the system can still list the network going away, so asking [onLan]
     * there said "still on Wi-Fi" after Wi-Fi was turned off.
     */
    fun watchLan(onChange: (Boolean) -> Unit) {
        unwatchLan()
        val up = java.util.Collections.synchronizedSet(mutableSetOf<Network>())
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) { up += network; onChange(true) }
            override fun onLost(network: Network) { up -= network; onChange(up.isNotEmpty()) }
        }
        runCatching { connectivity.registerNetworkCallback(lanRequest(), cb) }.onSuccess { lanCallback = cb }
    }

    fun unwatchLan() {
        lanCallback?.let { runCatching { connectivity.unregisterNetworkCallback(it) } }
        lanCallback = null
    }

    /** Has the system start [OutdoorReceiver] when Wi-Fi or Ethernet comes up, with no process of ours running. */
    fun wakeOnLan(context: Context) {
        cancelWakeOnLan(context)
        runCatching { connectivity.registerNetworkCallback(lanRequest(), outdoorIntent(context)) }
            .onFailure { VLog.w(TAG, "can't watch for Wi-Fi: ${it.message}") }
    }

    fun cancelWakeOnLan(context: Context) {
        runCatching { connectivity.unregisterNetworkCallback(outdoorIntent(context)) }
    }

    private fun outdoorIntent(context: Context) = android.app.PendingIntent.getBroadcast(
        context, 7, android.content.Intent(context, OutdoorReceiver::class.java),
        android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_MUTABLE,
    )

    private fun lanRequest() = android.net.NetworkRequest.Builder()
        .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
        .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
        .build()

    private companion object {
        const val TAG = "AndroidNetwork"
        // Linux <netinet/tcp.h>; not exposed by OsConstants.
        const val TCP_KEEPIDLE = 4
        const val TCP_KEEPINTVL = 5
        const val TCP_KEEPCNT = 6
        const val TCP_USER_TIMEOUT = 18
    }
}
