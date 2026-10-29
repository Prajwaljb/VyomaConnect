package dev.vyoma.connect.service

import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.device.HubEvent
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.launch

/**
 * The one long-lived component. Runs the hub while the app is in use or devices are
 * paired, and keeps a quiet foreground notification so One UI doesn't kill it.
 */
class LinkService : LifecycleService() {
    private var hub: DeviceHub? = null
    private val network get() = application.container.network

    override fun onCreate() {
        super.onCreate()
        Notifications.ensureChannels(this)
        startForeground(
            Notifications.SERVICE_ID,
            Notifications.service(this, connected = emptyList()),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
        )
        lifecycleScope.launch { startHub() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == ACTION_REFRESH) hub?.refresh()
        return START_STICKY
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    private suspend fun startHub() {
        val hub = application.container.obtainHub()
        this.hub = hub
        network.acquireMulticast()
        hub.start()
        network.watch(
            onAvailable = { hub.refresh() },
            onLost = { hub.refresh() },
        )

        // Keep the ongoing notification in sync with who's connected.
        lifecycleScope.launch {
            hub.devices
                .flatMapLatest { devices ->
                    if (devices.isEmpty()) flowOf(emptyList())
                    else combine(devices.values.map { it.state }) { states -> states.filter { it.isReachable && it.isPaired } }
                }
                .distinctUntilChanged { a, b -> a.map(::summary) == b.map(::summary) }
                .collect { connected -> Notifications.updateService(this@LinkService, connected) }
        }
        lifecycleScope.launch {
            hub.events.collect { event ->
                when (event) {
                    is HubEvent.PairRequested -> Notifications.pairRequest(this@LinkService, event.device)
                    is HubEvent.CertificateMismatch -> Notifications.certificateMismatch(this@LinkService, event.name)
                }
            }
        }
        lifecycleScope.launch {
            PingPlugin.received.collect { ping ->
                val name = hub.device(ping.deviceId)?.name ?: return@collect
                Notifications.ping(this@LinkService, name, ping.message)
            }
        }
    }

    private fun summary(s: DeviceState) = Triple(s.identity.deviceId, s.identity.deviceName, s.transports.contains(TransportKind.USB))

    override fun onDestroy() {
        network.unwatch()
        network.releaseMulticast()
        hub?.stop()
        super.onDestroy()
    }

    companion object {
        const val ACTION_REFRESH = "dev.vyoma.connect.REFRESH"

        fun start(context: Context, refresh: Boolean = false) {
            val intent = Intent(context, LinkService::class.java)
            if (refresh) intent.action = ACTION_REFRESH
            ContextCompat.startForegroundService(context, intent)
        }
    }
}
