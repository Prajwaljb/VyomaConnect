package dev.vyoma.connect.service

import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import dev.vyoma.connect.platform.Handoff
import dev.vyoma.connect.platform.outgoingFile
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import dev.vyoma.connect.BuildConfig
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.device.HubEvent
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SharePlugin
import dev.vyoma.connect.protocol.plugin.builtin.Transfer
import dev.vyoma.connect.protocol.plugin.builtin.Transfers
import dev.vyoma.connect.widget.Widgets
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.transform
import kotlinx.coroutines.launch

/**
 * The one long-lived component. Runs the hub while the app is in use or devices are
 * paired, and keeps a quiet foreground notification so One UI doesn't kill it.
 */
class LinkService : LifecycleService() {
    private var hub: DeviceHub? = null
    private val container get() = application.container
    private val network get() = container.network
    private var mediaSession: PcMediaSession? = null
    private var mdns: MdnsDiscovery? = null
    private var usb: UsbLink? = null

    override fun onCreate() {
        super.onCreate()
        Notifications.ensureChannels(this)
        // Refused right after the user stops the app from One UI's Active apps panel. Started from the
        // UI we still run (as a plain service while the app is open), so carry on rather than crash.
        runCatching {
            startForeground(
                Notifications.SERVICE_ID,
                Notifications.service(this, connected = emptyList()),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
        }.onFailure { VLog.w("LinkService", "not allowed in the foreground yet: ${it.message}") }
        lifecycleScope.launch { startHub() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        when (intent?.action) {
            ACTION_REFRESH -> {
                hub?.refresh()
                startPhoneMonitors() // permissions may have just been granted
            }
            ACTION_SEND -> handleSend(intent)
        }
        return START_STICKY
    }

    @OptIn(ExperimentalCoroutinesApi::class, FlowPreview::class)
    private suspend fun startHub() {
        val hub = container.obtainHub()
        this.hub = hub
        network.customHosts = container.settings.manualHosts.first().toList()
        // The demo's laptop is simulated in-app: nothing to find on the network or over a cable.
        val usb = if (BuildConfig.DEMO) null else UsbLink(this, hub, lifecycleScope).also { this.usb = it }
        // Cable first: hold Wi-Fi back from the very first refresh, which hub.start() does.
        if (usb?.cableIn() == true) hub.cablePlugged()
        hub.start()
        if (!BuildConfig.DEMO) mdns = MdnsDiscovery(this, hub).also { it.start() }
        usb?.start()
        lifecycleScope.launch {
            container.settings.manualHosts.drop(1).collect { hosts ->
                network.customHosts = hosts.toList()
                hub.refresh()
            }
        }
        network.watch(
            onAvailable = { hub.refresh() },
            onLost = { hub.refresh() },
        )
        // The multicast lock keeps Wi-Fi from filtering broadcasts in power save, which costs
        // battery on every packet on the network. Only discovering new devices needs to hear the
        // PC's broadcasts, and that happens with the app open; in the background, paired PCs are
        // reached by our own broadcasts (no lock needed to send), mDNS, and network changes.
        lifecycleScope.launch {
            container.uiVisible.collect { visible ->
                if (visible) {
                    network.acquireMulticast()
                    hub.refresh()
                } else {
                    network.releaseMulticast()
                }
            }
        }

        val connectedFlow = hub.devices
            .flatMapLatest { devices ->
                if (devices.isEmpty()) flowOf(emptyList())
                else combine(devices.values.map { it.state }) { states -> states.filter { it.isReachable && it.isPaired && !it.isCompanion } }
            }

        // Keep the ongoing notification in sync with who's connected.
        lifecycleScope.launch {
            connectedFlow
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

        // Transfers: one live progress notification, then a result per file.
        lifecycleScope.launch {
            val announced = mutableSetOf<Long>()
            Transfers.all.throttleLatest(400).collect { all ->
                Notifications.transfers(this@LinkService, all.filter { it.state == Transfer.State.RUNNING })
                for (t in all) {
                    if (t.state != Transfer.State.RUNNING && announced.add(t.id)) {
                        Notifications.transferFinished(this@LinkService, t)
                        lifecycleScope.launch { delay(60_000); Transfers.dismiss(t.id) }
                    }
                }
            }
        }

        // Keep trying paired devices that are offline: fast at first, then every minute.
        lifecycleScope.launch {
            var wait = 5_000L
            while (true) {
                delay(wait)
                if (hub.anyPairedOffline()) {
                    hub.refresh()
                    wait = (wait * 2).coerceAtMost(60_000)
                } else {
                    wait = 5_000
                }
            }
        }

        mediaSession = PcMediaSession(this, container.media).also { it.start(lifecycleScope) }
        container.clipboard.startBackgroundSync()
        startPhoneMonitors()

        // Widgets and tiles read a snapshot; refresh it whenever something they show changes.
        // The PC's battery arrives after the link is up (once its plugin loads), then on every change.
        val pcBattery = connectedFlow
            .map { states -> states.map { it.identity.deviceId to it.plugins } }
            .distinctUntilChanged()
            .flatMapLatest { Commands.primary(container)?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote ?: flowOf(null) }
        lifecycleScope.launch {
            combine(
                connectedFlow, container.media.nowPlaying, Transfers.all.map { list -> list.count { it.state == Transfer.State.RUNNING } },
                pcBattery, container.battery.state,
            ) { c, np, running, pc, phone ->
                listOf(c.map(::summary), np?.let { Triple(it.player.title, it.player.isPlaying, it.art) }, running, pc, phone)
            }
                .distinctUntilChanged()
                .throttleLatest(500)
                .collect { Widgets.refresh(this@LinkService) }
        }
    }

    /** Each is a no-op without its permission, and idempotent. */
    private fun startPhoneMonitors() {
        container.calls.start()
        container.signal.start()
    }

    private fun handleSend(intent: Intent) {
        val deviceId = intent.getStringExtra(EXTRA_DEVICE) ?: return
        val uris = buildList {
            intent.clipData?.let { clip -> for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let(::add) }
        }
        val text = intent.getStringExtra(Intent.EXTRA_TEXT)
        lifecycleScope.launch {
            val device = container.obtainHub().device(deviceId) ?: return@launch
            val share = device.plugin<SharePlugin>(SharePlugin.ID) ?: return@launch
            if (uris.isNotEmpty()) {
                share.sendFiles(uris.map(contentResolver::outgoingFile))
            } else if (text != null) {
                if (looksLikeUrl(text)) share.sendUrl(Handoff.continueOnPc(this@LinkService, text.trim())) else share.sendText(text)
            }
        }
    }

    /**
     * At most one value per [ms], always ending on the latest. Unlike `sample`, which runs a
     * timer forever (waking the main thread twice a second for nothing), idle costs nothing.
     */
    private fun <T> Flow<T>.throttleLatest(ms: Long): Flow<T> = conflate().transform {
        emit(it)
        delay(ms)
    }

    private fun summary(s: DeviceState) = Triple(s.identity.deviceId, s.identity.deviceName, s.transports.contains(TransportKind.USB))

    override fun onDestroy() {
        mediaSession?.stop()
        mdns?.stop()
        usb?.stop()
        container.calls.stop()
        container.signal.stop()
        network.unwatch()
        network.releaseMulticast()
        hub?.stop()
        super.onDestroy()
    }

    companion object {
        const val ACTION_REFRESH = "dev.vyoma.connect.REFRESH"
        const val ACTION_SEND = "dev.vyoma.connect.SEND"
        const val EXTRA_DEVICE = "device"

        /**
         * [visible] callers (an activity on screen) use a plain start: it's allowed while the app is in
         * front and, unlike a foreground start, has no "call startForeground within 10 s" deadline
         * that crashes the app when the system holds the service back (e.g. just after Stop in
         * Active apps). Background callers must use a foreground start.
         */
        fun start(context: Context, refresh: Boolean = false, visible: Boolean = false) {
            val intent = Intent(context, LinkService::class.java)
            if (refresh) intent.action = ACTION_REFRESH
            deliver(context, intent, visible)
        }

        fun deliver(context: Context, intent: Intent, visible: Boolean) {
            if (visible) {
                try {
                    context.startService(intent)
                    return
                } catch (_: IllegalStateException) {
                    // Already went to the background; fall through.
                }
            }
            ContextCompat.startForegroundService(context, intent)
        }

        fun looksLikeUrl(text: String): Boolean {
            val t = text.trim()
            return !t.contains(' ') && !t.contains('\n') && (t.startsWith("http://") || t.startsWith("https://"))
        }
    }
}
