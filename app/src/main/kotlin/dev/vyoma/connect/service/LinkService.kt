package dev.vyoma.connect.service

import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import dev.vyoma.connect.platform.Handoff
import dev.vyoma.connect.platform.outgoingFile
import dev.vyoma.connect.platform.folderItems
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.CompanionUploads
import dev.vyoma.connect.protocol.plugin.builtin.SendItem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
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
import dev.vyoma.connect.webcam.PhoneWebcam
import dev.vyoma.connect.mirror.PhoneMirror
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
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.MutableStateFlow

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
    private var bluetooth: BluetoothTunnel? = null
    private var lastConnected: List<dev.vyoma.connect.protocol.device.DeviceState> = emptyList()

    override fun onCreate() {
        super.onCreate()
        // Running again: whatever woke us, the outdoor wake-up isn't needed any more.
        network.cancelWakeOnLan(this)
        Notifications.ensureChannels(this)
        // Refused right after the user stops the app from One UI's Active apps panel. Started from the
        // UI we still run (as a plain service while the app is open), so carry on rather than crash.
        startForegroundNow()
        lifecycleScope.launch { startHub() }
    }

    private var foreground = false

    private fun startForegroundNow() {
        runCatching {
            startForeground(
                Notifications.SERVICE_ID,
                Notifications.service(this, connected = emptyList()),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
            foreground = true
        }.onFailure { VLog.w("LinkService", "not allowed in the foreground yet: ${it.message}") }
    }

    // What the phone is streaming to the PC: each adds its foreground type (see ACTION_MEDIA).
    private var mediaCamera = false
    private var mediaMic = false
    private var mediaProjection = false

    private fun foregroundForMedia(): Boolean {
        var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
        if (mediaCamera) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        if (mediaMic) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        if (mediaProjection) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
        return runCatching { startForeground(Notifications.SERVICE_ID, Notifications.service(this, lastConnected), types) }
            .onFailure { VLog.w("LinkService", "media in the foreground refused: ${it.message}") }.isSuccess
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        // Refused in onCreate and started in the foreground now: Android ends the app unless we say it again.
        if (!foreground) startForegroundNow()
        when (intent?.action) {
            ACTION_REFRESH -> {
                hub?.refresh()
                startPhoneMonitors() // permissions may have just been granted
            }
            ACTION_SEND -> handleSend(intent)
            // The phone's camera/mic streaming to the PC (PhoneWebcam): Android lets a service use them
            // in the background only while it's in the foreground as such. Added from the screen that
            // starts it (in front), dropped again when it stops.
            ACTION_WEBCAM_STOP -> dev.vyoma.connect.webcam.PhoneWebcam.stop(this)
            ACTION_MEDIA -> {
                if (intent.hasExtra(EXTRA_CAMERA)) mediaCamera = intent.getBooleanExtra(EXTRA_CAMERA, false)
                if (intent.hasExtra(EXTRA_MIC)) mediaMic = intent.getBooleanExtra(EXTRA_MIC, false)
                if (intent.hasExtra(EXTRA_PROJECTION)) mediaProjection = intent.getBooleanExtra(EXTRA_PROJECTION, false)
                foregroundForMedia()
            }
            // The user allowed the screen to be shared: Android hands the grant over only to a service
            // already in the foreground as mediaProjection.
            ACTION_MIRROR -> {
                val code = intent.getIntExtra(EXTRA_CODE, 0)
                @Suppress("DEPRECATION")
                val data = intent.getParcelableExtra<Intent>(EXTRA_DATA)
                mediaProjection = true
                if (data != null && foregroundForMedia()) dev.vyoma.connect.mirror.PhoneMirror.begin(this, code, data)
                else mediaProjection = false
            }
            ACTION_MIRROR_STOP -> dev.vyoma.connect.mirror.PhoneMirror.stop(this)
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
        // Every port taken: no links, but no crash at every start (at boot, too) over something that passes.
        try {
            hub.start()
        } catch (e: java.io.IOException) {
            VLog.e("LinkService", "can't listen for links", e)
        }
        if (!BuildConfig.DEMO) mdns = MdnsDiscovery(this, hub).also { it.start() }
        usb?.start()
        if (!BuildConfig.DEMO) bluetooth = BluetoothTunnel(this, hub, lifecycleScope).also { it.start() }
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

        if (!BuildConfig.DEMO) watchOutdoors(connectedFlow.map { it.isNotEmpty() }.distinctUntilChanged())

        // Remember when each PC drops, so Home can say how long it's been away.
        lifecycleScope.launch {
            var before = emptySet<String>()
            connectedFlow.map { list -> list.map { it.identity.deviceId }.toSet() }.distinctUntilChanged().collect { now ->
                (before - now).takeIf { it.isNotEmpty() }?.let { LastSeen.mark(this@LinkService, it) }
                before = now
            }
        }

        // Keep the ongoing notification in sync with who's connected.
        lifecycleScope.launch {
            connectedFlow
                .distinctUntilChanged { a, b -> a.map(::summary) == b.map(::summary) }
                .collect { connected ->
                    lastConnected = connected
                    Notifications.updateService(this@LinkService, connected)
                }
        }
        // The webcam's state is in the notification too (title, Stop).
        lifecycleScope.launch {
            dev.vyoma.connect.webcam.PhoneWebcam.status.map { Triple(it.on, it.pc, it.options.mic) }.distinctUntilChanged()
                .collect { Notifications.updateService(this@LinkService, lastConnected) }
        }
        lifecycleScope.launch {
            dev.vyoma.connect.mirror.PhoneMirror.status.map { it.on to it.pc }.distinctUntilChanged()
                .collect { Notifications.updateService(this@LinkService, lastConnected) }
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
                        TransferLog.add(this@LinkService, t)
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
        container.automation.start()

        // Widgets and tiles read a snapshot; refresh it whenever something they show changes.
        // The PC's battery arrives after the link is up (once its plugin loads), then on every change.
        val pcBattery = connectedFlow
            .map { states -> states.map { it.identity.deviceId to it.plugins } }
            .distinctUntilChanged()
            .flatMapLatest { Commands.primary(container)?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote ?: flowOf(null) }
        lifecycleScope.launch {
            // Transfers in 5% steps, for the drop widget's ring; the newest clips, for the clipboard widget.
            val transfers = Transfers.all.map { list ->
                val running = list.filter { it.state == Transfer.State.RUNNING }
                val total = running.sumOf { it.totalBytes.coerceAtLeast(0) }
                running.size to if (total > 0) running.sumOf { it.doneBytes } * 20 / total else 0L
            }
            val shown = combine(transfers, container.clipboard.history.map { it.take(4) }) { running, clips -> running to clips }
            combine(connectedFlow, container.media.nowPlaying, shown, pcBattery, container.battery.state) { c, np, others, pc, phone ->
                listOf(c.map(::summary), np?.let { Triple(it.player.title, it.player.isPlaying, it.art) }, others, pc, phone)
            }
                .distinctUntilChanged()
                .throttleLatest(500)
                .collect { Widgets.refresh(this@LinkService) }
        }
    }

    /**
     * Outdoor mode: away from Wi-Fi, with no cable, no PC connected for a minute (a Tailscale link
     * keeps it running), the app not on screen, no Bluetooth fallback to wait for and no camera or
     * mirror on, the service stops: no notification, no sockets, nothing waking the phone. ConnectivityManager holds a PendingIntent that starts it again
     * ([OutdoorReceiver]) when Wi-Fi or Ethernet comes up; opening the app starts it too.
     */
    private fun watchOutdoors(anyConnected: Flow<Boolean>) {
        val onLan = MutableStateFlow(network.onLan())
        network.watchLan { onLan.value = it }
        lifecycleScope.launch {
            val sharing = combine(PhoneWebcam.status, PhoneMirror.status) { camera, mirror -> camera.on || mirror.on }
            val staying = combine(anyConnected, container.uiVisible, usb?.cable ?: flowOf(false), BluetoothFallback.enabled, sharing) { connected, visible, cable, radio, shared ->
                connected || visible || cable || radio || shared
            }
            combine(container.settings.outdoorMode, onLan, staying) { enabled, lan, stay -> enabled && !lan && !stay }
                .distinctUntilChanged()
                .collectLatest { outdoors ->
                    if (!outdoors) return@collectLatest
                    delay(OUTDOOR_GRACE_MS)
                    VLog.i("LinkService", "outdoor mode: off Wi-Fi with nothing connected; pausing until Wi-Fi is back")
                    network.wakeOnLan(this@LinkService)
                    stopSelf()
                }
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
        val isFolder = intent.getBooleanExtra(EXTRA_FOLDER, false)
        lifecycleScope.launch {
            val hub = container.obtainHub()
            val device = hub.device(deviceId) ?: return@launch
            val share = device.plugin<SharePlugin>(SharePlugin.ID)
            // Through the companion when it's there: resumes after drops, takes the cable, carries folders.
            val uploads = CompanionUploads(device) { companionOf(hub, deviceId) }.takeIf { companionOf(hub, deviceId) != null }
            if (isFolder) {
                val tree = uris.firstOrNull() ?: return@launch
                val (name, items) = withContext(Dispatchers.IO) { contentResolver.folderItems(tree) }
                when {
                    uploads != null -> uploads.send(items, folderName = name)
                    // Without the companion its files still go, side by side in Downloads.
                    else -> share?.sendFiles(items.map { it.file })
                }
                runCatching { contentResolver.releasePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION) }
            } else if (uris.isNotEmpty()) {
                val files = withContext(Dispatchers.IO) { uris.map(contentResolver::outgoingFile) }
                // The PC needs a file's size to resume it; a provider that won't say goes the stock way.
                if (uploads != null && files.all { it.size >= 0 }) uploads.send(files.map { SendItem(it) }) else share?.sendFiles(files)
            } else if (share != null && text != null) {
                if (looksLikeUrl(text)) share.sendUrl(Handoff.continueOnPc(this@LinkService, text.trim())) else share.sendText(text)
            }
        }
    }

    /** The companion beside PC [pcId], while it's linked. */
    private fun companionOf(hub: DeviceHub, pcId: String): CompanionPlugin? =
        hub.devices.value.values.firstOrNull { it.state.value.identity.companionOf == pcId && it.state.value.isPaired && it.state.value.isReachable }
            ?.plugin(CompanionPlugin.ID)

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
        network.unwatchLan()
        mediaSession?.stop()
        mdns?.stop()
        usb?.stop()
        bluetooth?.stop()
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
        const val ACTION_MEDIA = "dev.vyoma.connect.MEDIA"
        const val ACTION_WEBCAM_STOP = "dev.vyoma.connect.WEBCAM_STOP"
        const val ACTION_MIRROR = "dev.vyoma.connect.MIRROR"
        const val ACTION_MIRROR_STOP = "dev.vyoma.connect.MIRROR_STOP"
        private const val EXTRA_CAMERA = "camera"
        private const val EXTRA_MIC = "mic"
        private const val EXTRA_PROJECTION = "projection"
        private const val EXTRA_CODE = "code"
        private const val EXTRA_DATA = "data"

        /** The user allowed screen sharing ([code], [data] from the prompt): start mirroring to the PC. */
        fun mirror(context: Context, code: Int, data: Intent) {
            val intent = Intent(context, LinkService::class.java).setAction(ACTION_MIRROR)
                .putExtra(EXTRA_CODE, code).putExtra(EXTRA_DATA, data)
            runCatching { context.startService(intent) }.onFailure { VLog.w("LinkService", "mirror: ${it.message}") }
        }

        /** Mirroring ended: drop the mediaProjection foreground type. */
        fun projecting(context: Context, on: Boolean) {
            val intent = Intent(context, LinkService::class.java).setAction(ACTION_MEDIA).putExtra(EXTRA_PROJECTION, on)
            runCatching { context.startService(intent) }.onFailure { VLog.w("LinkService", "projection: ${it.message}") }
        }

        /** The phone's camera / mic are (no longer) streaming to the PC; see ACTION_MEDIA. */
        fun mediaInUse(context: Context, camera: Boolean, mic: Boolean) {
            val intent = Intent(context, LinkService::class.java).setAction(ACTION_MEDIA)
                .putExtra(EXTRA_CAMERA, camera).putExtra(EXTRA_MIC, mic)
            runCatching { context.startService(intent) }.onFailure { VLog.w("LinkService", "media: ${it.message}") }
        }
        /** Off Wi-Fi this long, with nothing connected, before outdoor mode pauses (a lift, a tunnel). */
        private const val OUTDOOR_GRACE_MS = 60_000L
        /** With ACTION_SEND: the one URI is a folder tree to send whole. */
        const val EXTRA_FOLDER = "folder"
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
            try {
                start(context, intent, visible)
            } catch (_: SecurityException) {
                // A read grant we may not pass on (some senders, `am start` from a shell): this app
                // still holds it for now, so hand the URIs over without granting them again.
                start(context, Intent(intent).setFlags(intent.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION.inv()), visible)
            }
        }

        private fun start(context: Context, intent: Intent, visible: Boolean) {
            if (visible) {
                try {
                    context.startService(intent)
                    return
                } catch (_: IllegalStateException) {
                    // Already went to the background; fall through.
                }
            }
            try {
                ContextCompat.startForegroundService(context, intent)
            } catch (e: IllegalStateException) {
                VLog.w("LinkService", "not allowed to start now: ${e.message}") // in the background, with no exemption
            }
        }

        fun looksLikeUrl(text: String): Boolean {
            val t = text.trim()
            return !t.contains(' ') && !t.contains('\n') && (t.startsWith("http://", ignoreCase = true) || t.startsWith("https://", ignoreCase = true))
        }
    }
}
