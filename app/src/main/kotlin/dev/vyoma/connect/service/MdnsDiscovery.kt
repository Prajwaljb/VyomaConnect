package dev.vyoma.connect.service

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.DeviceHub
import java.net.Inet4Address
import java.net.InetAddress
import java.util.concurrent.Executor

/**
 * mDNS (`_kdeconnect._udp`), as KDE Connect 24.x+ does it: we publish ourselves with the device id
 * as the service name and our TCP port, and dial any peer we resolve. This finds the PC on
 * networks that drop UDP broadcasts (many routers, guest/mesh Wi-Fi, some hotspots).
 */
class MdnsDiscovery(context: Context, private val hub: DeviceHub) {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val executor = Executor { main.post(it) }
    private val ownId = hub.localIdentity().deviceId

    private var discovery: NsdManager.DiscoveryListener? = null
    private var registration: NsdManager.RegistrationListener? = null
    private val infoCallbacks = HashMap<String, NsdManager.ServiceInfoCallback>()

    // Before API 34 NsdManager resolves one service at a time; queue the rest.
    private val resolveQueue = ArrayDeque<NsdServiceInfo>()
    private var resolving = false

    fun start() {
        if (discovery != null) return
        announce()
        val listener = object : NsdManager.DiscoveryListener {
            override fun onServiceFound(info: NsdServiceInfo) = main.post { found(info) }.let { }
            override fun onServiceLost(info: NsdServiceInfo) = main.post { lost(info) }.let { }
            override fun onDiscoveryStarted(serviceType: String) = VLog.d(TAG, "browsing $serviceType")
            override fun onDiscoveryStopped(serviceType: String) = Unit
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                VLog.w(TAG, "discovery failed: $errorCode")
                discovery = null
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
        }
        discovery = listener
        runCatching { nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener) }
            .onFailure { VLog.w(TAG, "can't browse: ${it.message}"); discovery = null }
    }

    fun stop() {
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        discovery = null
        registration?.let { runCatching { nsd.unregisterService(it) } }
        registration = null
        if (Build.VERSION.SDK_INT >= 34) infoCallbacks.values.forEach { runCatching { nsd.unregisterServiceInfoCallback(it) } }
        infoCallbacks.clear()
        resolveQueue.clear()
    }

    private fun announce() {
        val port = hub.lan.tcpPort.takeIf { it > 0 } ?: return
        val me = hub.localIdentity()
        val info = NsdServiceInfo().apply {
            serviceName = me.deviceId
            serviceType = SERVICE_TYPE
            this.port = port
            setAttribute("id", me.deviceId)
            setAttribute("name", me.deviceName)
            setAttribute("type", me.deviceType.wire)
            setAttribute("protocol", me.protocolVersion.toString())
        }
        val listener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) = VLog.d(TAG, "announced on port $port")
            override fun onRegistrationFailed(info: NsdServiceInfo, errorCode: Int) {
                VLog.w(TAG, "announce failed: $errorCode")
                registration = null
            }
            override fun onServiceUnregistered(info: NsdServiceInfo) = Unit
            override fun onUnregistrationFailed(info: NsdServiceInfo, errorCode: Int) = Unit
        }
        registration = listener
        runCatching { nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener) }
            .onFailure { VLog.w(TAG, "can't announce: ${it.message}"); registration = null }
    }

    private fun found(info: NsdServiceInfo) {
        val id = info.serviceName
        if (id == ownId) return
        if (hub.device(id)?.state?.value?.isReachable == true) return
        if (Build.VERSION.SDK_INT >= 34) {
            if (id in infoCallbacks) return
            val cb = object : NsdManager.ServiceInfoCallback {
                override fun onServiceUpdated(resolved: NsdServiceInfo) = connect(resolved)
                override fun onServiceLost() = Unit
                override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {
                    infoCallbacks.remove(id)
                }
                override fun onServiceInfoCallbackUnregistered() = Unit
            }
            infoCallbacks[id] = cb
            runCatching { nsd.registerServiceInfoCallback(info, executor, cb) }.onFailure { infoCallbacks.remove(id) }
        } else {
            resolveQueue.addLast(info)
            resolveNext()
        }
    }

    private fun lost(info: NsdServiceInfo) {
        if (Build.VERSION.SDK_INT >= 34) {
            infoCallbacks.remove(info.serviceName)?.let { runCatching { nsd.unregisterServiceInfoCallback(it) } }
        }
    }

    @Suppress("DEPRECATION")
    private fun resolveNext() {
        if (resolving) return
        val next = resolveQueue.removeFirstOrNull() ?: return
        resolving = true
        nsd.resolveService(next, object : NsdManager.ResolveListener {
            override fun onServiceResolved(resolved: NsdServiceInfo) = main.post {
                resolving = false
                connect(resolved)
                resolveNext()
            }.let { }

            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) = main.post {
                resolving = false
                resolveNext()
            }.let { }
        })
    }

    private fun connect(info: NsdServiceInfo) {
        val addresses: List<InetAddress> = if (Build.VERSION.SDK_INT >= 34) info.hostAddresses else @Suppress("DEPRECATION") listOfNotNull(info.host)
        // Prefer IPv4: it's what the LAN routing (and KDE Connect) handle best.
        val address = addresses.firstOrNull { it is Inet4Address } ?: addresses.firstOrNull() ?: return
        VLog.i(TAG, "found ${info.serviceName.take(8)}… at ${address.hostAddress}:${info.port}")
        hub.dial(address, info.port, info.serviceName)
    }

    private companion object {
        const val TAG = "Mdns"
        const val SERVICE_TYPE = "_kdeconnect._udp"
    }
}
