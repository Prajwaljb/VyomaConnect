package dev.vyoma.connect.service

import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.device.HubEvent
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.plugin.builtin.OutgoingFile
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
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.sample
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
        network.acquireMulticast()
        hub.start()
        network.watch(
            onAvailable = { hub.refresh() },
            onLost = { hub.refresh() },
        )

        val connectedFlow = hub.devices
            .flatMapLatest { devices ->
                if (devices.isEmpty()) flowOf(emptyList())
                else combine(devices.values.map { it.state }) { states -> states.filter { it.isReachable && it.isPaired } }
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
            Transfers.all.sample(400).collect { all ->
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
        lifecycleScope.launch {
            combine(connectedFlow, container.media.nowPlaying, Transfers.all.map { list -> list.count { it.state == Transfer.State.RUNNING } }) { c, np, running ->
                Triple(c.map(::summary), np?.let { Triple(it.player.title, it.player.isPlaying, it.art) }, running)
            }
                .distinctUntilChanged()
                .sample(500)
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
                share.sendFiles(uris.mapNotNull(::describe))
            } else if (text != null) {
                if (looksLikeUrl(text)) share.sendUrl(text.trim()) else share.sendText(text)
            }
        }
    }

    private fun describe(uri: Uri): OutgoingFile? {
        var name: String? = null
        var size = -1L
        runCatching {
            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    name = c.getString(0)
                    if (!c.isNull(1)) size = c.getLong(1)
                }
            }
        }.onFailure { VLog.w("Share", "can't read $uri: ${it.message}") }
        if (size < 0) size = runCatching { contentResolver.openAssetFileDescriptor(uri, "r")?.use { it.length } }.getOrNull() ?: -1
        return OutgoingFile(name ?: uri.lastPathSegment ?: "file", size, null) {
            contentResolver.openInputStream(uri) ?: error("can't open $uri")
        }
    }

    private fun summary(s: DeviceState) = Triple(s.identity.deviceId, s.identity.deviceName, s.transports.contains(TransportKind.USB))

    override fun onDestroy() {
        mediaSession?.stop()
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

        fun start(context: Context, refresh: Boolean = false) {
            val intent = Intent(context, LinkService::class.java)
            if (refresh) intent.action = ACTION_REFRESH
            ContextCompat.startForegroundService(context, intent)
        }

        fun looksLikeUrl(text: String): Boolean {
            val t = text.trim()
            return !t.contains(' ') && !t.contains('\n') && (t.startsWith("http://") || t.startsWith("https://"))
        }
    }
}
