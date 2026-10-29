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

    /** Manual hosts (Tailscale IPs, hotspot peers); filled from settings later in Phase 1. */
    @Volatile var customHosts: List<String> = emptyList()

    fun lanConfig() = LanBackend.Config(
        broadcastTargets = ::broadcastTargets,
        tuneSocket = ::tuneSocket,
    )

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
        customHosts.forEach { host -> runCatching { targets += InetAddress.getByName(host) } }
        return targets
    }

    /**
     * Keep-alive tuned for a phone: probe after 60 s idle, every 10 s, give up after 3.
     * Dead links surface within ~90 s without waking the radio constantly.
     */
    private fun tuneSocket(socket: Socket) {
        socket.tcpNoDelay = true
        socket.keepAlive = true
        runCatching {
            ParcelFileDescriptor.fromSocket(socket).use { pfd ->
                val fd = pfd.fileDescriptor
                Os.setsockoptInt(fd, OsConstants.IPPROTO_TCP, TCP_KEEPIDLE, 60)
                Os.setsockoptInt(fd, OsConstants.IPPROTO_TCP, TCP_KEEPINTVL, 10)
                Os.setsockoptInt(fd, OsConstants.IPPROTO_TCP, TCP_KEEPCNT, 3)
            }
        }.onFailure { VLog.d(TAG, "keep-alive tuning unavailable: ${it.message}") }
    }

    /** Lets UDP broadcasts reach us while Wi-Fi power-saves. Held only while the service runs. */
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
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        connectivity.registerNetworkCallback(request, cb)
        callback = cb
    }

    fun unwatch() {
        callback?.let { runCatching { connectivity.unregisterNetworkCallback(it) } }
        callback = null
    }

    private companion object {
        const val TAG = "AndroidNetwork"
        // Linux <netinet/tcp.h>; not exposed by OsConstants.
        const val TCP_KEEPIDLE = 4
        const val TCP_KEEPINTVL = 5
        const val TCP_KEEPCNT = 6
    }
}
