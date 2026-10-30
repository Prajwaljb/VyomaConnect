package dev.vyoma.connect.ui

import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import androidx.core.content.edit
import android.app.Application
import android.content.Context
import android.content.Intent
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.vyoma.connect.container
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.Palette
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.platform.NowPlaying
import dev.vyoma.connect.protocol.crypto.Certificates
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.plugin.builtin.BatteryState
import dev.vyoma.connect.protocol.plugin.builtin.ClipEntry
import dev.vyoma.connect.protocol.plugin.builtin.ClipboardPlugin
import dev.vyoma.connect.protocol.plugin.builtin.FindMyPhonePlugin
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import dev.vyoma.connect.protocol.plugin.builtin.MprisPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.protocol.plugin.builtin.RunCommandPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SystemVolumePlugin
import dev.vyoma.connect.service.LinkService
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

@OptIn(ExperimentalCoroutinesApi::class)
class AppViewModel(app: Application) : AndroidViewModel(app) {
    private val container = app.container
    private val settings = container.settings

    val theme: StateFlow<ThemeSettings> = settings.theme
        .stateIn(viewModelScope, SharingStarted.Eagerly, ThemeSettings())

    /** Every known device, ordered: connected, pairing, available, offline. */
    val devices: StateFlow<List<DeviceState>> = container.hub.filterNotNull()
        .flatMapLatest { hub -> hub.devices }
        .flatMapLatest { map ->
            if (map.isEmpty()) flowOf(emptyList())
            else combine(map.values.map { it.state }) { states -> states.sortedWith(deviceOrder) }
        }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    val deviceName: StateFlow<String> = container.hub.filterNotNull()
        .flatMapLatest { hub -> settings.deviceName.map { it ?: hub.deviceName } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), container.defaultDeviceName())

    val nowPlaying: StateFlow<NowPlaying?> get() = container.media.nowPlaying
    val phoneBattery: StateFlow<BatteryState?> get() = container.battery.state

    private val batteryCache by lazy { getApplication<Application>().getSharedPreferences("pc_battery", Context.MODE_PRIVATE) }

    /** Last battery a PC reported. Peers send it once per connection, so a quick reconnect can miss it. */
    fun cachedBattery(id: String): BatteryState? =
        batteryCache.getString(id, null)?.split(',')?.let { (c, ch) -> BatteryState(c.toInt(), ch.toBoolean()) }

    fun cacheBattery(id: String, b: BatteryState) = batteryCache.edit { putString(id, "${b.charge},${b.charging}") }

    /** Pushes this phone's clipboard to every connected PC; returns who got it, or null if nothing was sent. */
    fun sendClipboard(): String? {
        container.clipboard.readNow()
        val text = container.clipboard.current.value?.text?.takeIf { it.isNotBlank() } ?: return null
        val targets = container.hub.value?.devices?.value?.values?.filter { it.state.value.isPaired && it.state.value.isReachable }.orEmpty()
        if (targets.isEmpty()) return null
        viewModelScope.launch { targets.forEach { it.plugin<ClipboardPlugin>(ClipboardPlugin.ID)?.push(text) } }
        return targets.joinToString { it.name }
    }
    val clipHistory: StateFlow<List<ClipEntry>> get() = container.clipboard.history

    /**
     * Whether to show first-run setup; null until known. People who already paired a PC (before
     * setup existed, or after reinstalling with data kept) skip it.
     */
    val showOnboarding: StateFlow<Boolean?> = settings.onboarded.map { !it }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    init {
        viewModelScope.launch {
            if (!settings.onboarded.first() && container.obtainHub().trustStore.all().isNotEmpty()) settings.setOnboarded()
        }
    }

    fun finishOnboarding() = viewModelScope.launch { settings.setOnboarded() }

    fun startService() = LinkService.start(getApplication(), visible = true)

    fun refresh() = LinkService.start(getApplication(), refresh = true, visible = true)

    fun device(id: String): Device? = container.hub.value?.device(id)
    fun deviceName(id: String): String = device(id)?.name ?: "PC"

    fun requestPair(id: String) = act(id) { it.requestPairing() }
    fun acceptPair(id: String) = act(id) { it.acceptPairing() }
    fun rejectPair(id: String) = act(id) { it.rejectPairing() }
    /** Unpairing a PC also drops its companion, which was only trusted on the PC's behalf. */
    fun unpair(id: String) {
        act(id) { it.unpair() }
        devices.value.filter { it.identity.companionOf == id }.forEach { c -> act(c.identity.deviceId) { it.unpair() } }
    }
    fun ping(id: String) = act(id) { it.plugin<PingPlugin>(PingPlugin.ID)?.ping("Ping from Vyoma") }
    fun ringPc(id: String) = act(id) { it.plugin<FindMyPhonePlugin>(FindMyPhonePlugin.ID)?.ringPeer() }
    fun lockPc(id: String) = act(id) { it.plugin<LockPlugin>(LockPlugin.ID)?.setLocked(true) }
    fun setPcLocked(id: String, locked: Boolean) = act(id) { it.plugin<LockPlugin>(LockPlugin.ID)?.setLocked(locked) }
    fun runCommand(id: String, key: String) = act(id) { it.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.run(key) }
    fun openCommandSetup(id: String) = act(id) { it.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.openSetup() }
    fun forget(id: String) { container.hub.value?.forget(id) }

    fun pickFiles(context: Context) =
        context.startActivity(Intent(context, ShareActivity::class.java).setAction(ShareActivity.ACTION_PICK))

    // ---- media --------------------------------------------------------------------------------

    fun playPause() = container.media.control { p, n -> p.action(n, MprisPlugin.MediaAction.PLAY_PAUSE) }
    fun next() = container.media.control { p, n -> p.action(n, MprisPlugin.MediaAction.NEXT) }
    fun previous() = container.media.control { p, n -> p.action(n, MprisPlugin.MediaAction.PREVIOUS) }
    fun seekTo(ms: Long) = container.media.control { p, n -> p.seekTo(n, ms) }
    fun pickPlayer(name: String) { container.media.preferredPlayer.value = name }

    /**
     * Latest-value sender for slider drags: the first value goes out at once, then at most one
     * every [gapMs] while the finger moves, and the final value is never dropped.
     */
    private fun <T : Any> throttled(gapMs: Long = 50, send: suspend (T) -> Unit): (T) -> Unit {
        val latest = MutableStateFlow<T?>(null)
        viewModelScope.launch { latest.filterNotNull().collect { send(it); delay(gapMs) } }
        return { latest.value = it }
    }

    private val playerVolume = throttled<Int> { v -> container.media.control { p, n -> p.setVolume(n, v) } }
    private val sinkVolume = throttled<Triple<String, String, Int>> { (id, sink, v) ->
        device(id)?.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.setVolume(sink, v)
    }

    fun setPlayerVolume(v: Int) = playerVolume(v)
    fun setSinkVolume(id: String, sink: String, v: Int) = sinkVolume(Triple(id, sink, v))

    fun setSinkMuted(id: String, sink: String, muted: Boolean) = act(id) { it.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.setMuted(sink, muted) }
    fun makeSinkDefault(id: String, sink: String) = act(id) { it.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.makeDefault(sink) }

    // ---- clipboard ----------------------------------------------------------------------------

    fun copyClip(text: String) = container.clipboard.copy(text)
    fun clearClipHistory() = container.clipboard.clearHistory()
    fun backgroundClipboardSync() = container.clipboard.backgroundSyncAvailable()

    /** Called when the app comes to the front: read the clipboard and (re)start background sync. */
    fun onForeground() {
        container.clipboard.readNow()
        container.clipboard.startBackgroundSync()
    }

    fun fingerprint(id: String): String? = device(id)?.peerCertificate?.let(Certificates::sha256Fingerprint)

    fun takeScreenshot(plugin: CompanionPlugin) = viewModelScope.launch { plugin.requestScreenshot() }
    fun listPcFiles(plugin: CompanionPlugin, path: String) = viewModelScope.launch { plugin.listFiles(path) }
    fun downloadPcFile(plugin: CompanionPlugin, path: String) = viewModelScope.launch { plugin.download(path) }

    fun setPalette(p: Palette) = viewModelScope.launch { settings.setPalette(p) }
    fun setDarkMode(m: DarkMode) = viewModelScope.launch { settings.setDarkMode(m) }
    fun setAmoled(enabled: Boolean) = viewModelScope.launch { settings.setAmoled(enabled) }

    fun setDeviceName(name: String) = viewModelScope.launch {
        settings.setDeviceName(name)
        container.hub.value?.let { hub ->
            hub.deviceName = name
            hub.refresh()
        }
    }

    val manualHosts: StateFlow<Set<String>> = settings.manualHosts
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    /** Accepts an IP or host name, optionally with a :port we ignore (KDE Connect is always 1716). Returns false if invalid. */
    fun addManualHost(input: String): Boolean {
        val host = input.trim().removePrefix("http://").removePrefix("https://").substringBefore('/').let {
            if (it.count { c -> c == ':' } == 1) it.substringBefore(':') else it
        }
        if (host.isEmpty() || host.length > 253 || !host.all { it.isLetterOrDigit() || it in ".-:_" }) return false
        viewModelScope.launch { settings.addManualHost(host) }
        return true
    }

    fun removeManualHost(host: String) = viewModelScope.launch { settings.removeManualHost(host) }

    private fun act(id: String, block: suspend (Device) -> Unit) {
        viewModelScope.launch { device(id)?.let { block(it) } }
    }

    private companion object {
        fun rank(s: DeviceState) = when {
            s.isPaired && s.isReachable -> 0
            s.pairState == PairState.REQUESTED_BY_PEER || s.pairState == PairState.REQUESTED_BY_US -> 1
            s.isReachable -> 2
            else -> 3
        }

        val deviceOrder = compareBy<DeviceState>(::rank).thenBy { it.identity.deviceName.lowercase() }
    }
}
