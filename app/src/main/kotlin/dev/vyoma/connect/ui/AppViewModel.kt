package dev.vyoma.connect.ui

import dev.vyoma.connect.data.LiteGlass
import dev.vyoma.connect.automation.Action
import dev.vyoma.connect.automation.Rule
import dev.vyoma.connect.automation.RunEntry
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.OutgoingFile
import androidx.core.content.edit
import android.app.Application
import android.content.Context
import android.content.Intent
import dev.vyoma.connect.platform.Handoff
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
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import dev.vyoma.connect.protocol.plugin.builtin.MprisPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.platform.AppLock
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
    val liteGlass: StateFlow<LiteGlass> = settings.liteGlass
        .stateIn(viewModelScope, SharingStarted.Eagerly, LiteGlass(on = false, chosen = false, detected = false))
    val canvasGlow: StateFlow<Boolean> = settings.canvasGlow.stateIn(viewModelScope, SharingStarted.Eagerly, true)

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

    private val uiPrefs by lazy { getApplication<Application>().getSharedPreferences("ui", Context.MODE_PRIVATE) }
    private val promo = androidx.compose.runtime.mutableStateOf<Boolean?>(null)

    /** Whether Home still offers the companion (once, until dismissed). */
    fun companionPromo(): Boolean = promo.value ?: (!uiPrefs.getBoolean("companion_promo_seen", false)).also { promo.value = it }

    fun dismissCompanionPromo() {
        promo.value = false
        uiPrefs.edit { putBoolean("companion_promo_seen", true) }
    }

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
    fun replayOnboarding() = viewModelScope.launch { settings.setOnboarded(false) }

    fun startService() = LinkService.start(getApplication(), visible = true)

    fun refresh() = LinkService.start(getApplication(), refresh = true, visible = true)

    fun device(id: String): Device? = container.hub.value?.device(id)
    fun deviceName(id: String): String = device(id)?.name ?: "PC"

    fun requestPair(id: String) = act(id) { it.requestPairing() }
    fun acceptPair(id: String) = act(id) { it.acceptPairing() }
    fun rejectPair(id: String) = act(id) { it.rejectPairing() }
    fun unpair(id: String) = act(id) {
        it.unpair()
        dev.vyoma.connect.data.PcSpecsStore.forget(getApplication(), id)
    }
    fun ping(id: String) = act(id) { it.plugin<PingPlugin>(PingPlugin.ID)?.ping("Ping from Vyoma") }
    fun lockPc(id: String) = act(id) { it.plugin<LockPlugin>(LockPlugin.ID)?.setLocked(true) }
    fun setPcLocked(id: String, locked: Boolean) = act(id) {
        // Unlocking wakes the screen too: kdeconnectd unlocks but a screen that idled off stays dark.
        if (!locked) companionOf(id)?.wake()
        it.plugin<LockPlugin>(LockPlugin.ID)?.setLocked(locked)
    }
    fun runCommand(id: String, key: String) {
        uiPrefs.edit { putString("last_command_$id", key) }
        act(id) { it.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.run(key) }
    }

    /** The command last run on [id] (Commands' long-press runs it again), if it still exists. */
    fun lastCommand(id: String) = uiPrefs.getString("last_command_$id", null)

    /** Which desktop's shortcuts the touchpad's gestures send (see DesktopKind). */
    fun desktopKind(): DesktopKind = uiPrefs.getString("desktop_kind", null)?.let { k -> DesktopKind.entries.firstOrNull { it.name == k } } ?: DesktopKind.PLASMA
    fun setDesktopKind(kind: DesktopKind) = uiPrefs.edit { putString("desktop_kind", kind.name) }
    fun openCommandSetup(id: String) = act(id) { it.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.openSetup() }
    fun forget(id: String) { container.hub.value?.forget(id) }

    fun pickFiles(context: Context) =
        context.startActivity(Intent(context, ShareActivity::class.java).setAction(ShareActivity.ACTION_PICK))

    fun pickFolder(context: Context) =
        context.startActivity(Intent(context, ShareActivity::class.java).setAction(ShareActivity.ACTION_PICK_FOLDER))

    fun pickPhotos(context: Context) =
        context.startActivity(Intent(context, ShareActivity::class.java).setAction(ShareActivity.ACTION_PICK_PHOTOS))

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
        // A channel, not a StateFlow: the same value twice is two sends (the PC may have moved in between).
        val latest = kotlinx.coroutines.channels.Channel<T>(kotlinx.coroutines.channels.Channel.CONFLATED)
        viewModelScope.launch { for (value in latest) { send(value); delay(gapMs) } }
        return { latest.trySend(it) }
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
    /** Whether copies here are reaching the PC by themselves right now. */
    val clipboardSync get() = container.clipboard.syncState
    /** Starts the clipboard watcher again from here, in front, so Android asks about log access again. */
    fun retryClipboardSync() = container.clipboard.startBackgroundSync(again = true)

    /** Opens [url] in the PC's browser, as a shared link. */
    suspend fun openOnPc(id: String, url: String): Boolean = device(id)?.plugin<dev.vyoma.connect.protocol.plugin.builtin.SharePlugin>(dev.vyoma.connect.protocol.plugin.builtin.SharePlugin.ID)?.sendUrl(url) == true

    /** Called when the app comes to the front: read the clipboard and (re)start background sync. */
    fun onForeground() {
        container.clipboard.readNow()
        container.clipboard.startBackgroundSync()
    }

    fun fingerprint(id: String): String? = device(id)?.peerCertificate?.let(Certificates::sha256Fingerprint)

    fun capture(capture: CompanionPlugin.Capture) = viewModelScope.launch { capture.request() }

    /** The companion beside PC [pcId], when it's connected. */
    fun companionOf(pcId: String): CompanionPlugin? =
        devices.value.firstOrNull { it.identity.featuresOf == pcId && it.isPaired && it.isReachable }
            ?.let { device(it.identity.deviceId)?.plugin<CompanionPlugin>(CompanionPlugin.ID) }

    /**
     * Carries on here with what PC [pcId] is playing: the PC pauses, and the link opens on the
     * phone at the same moment (YouTube) or just opens (any other page).
     */
    fun continueOnPhone(pcId: String) = viewModelScope.launch {
        val app = getApplication<android.app.Application>()
        val plugin = companionOf(pcId) ?: return@launch toast(app, "Vyoma isn't running on the PC")
        plugin.handoff().fold(
            onSuccess = { h ->
                if (!LinkService.looksLikeUrl(h.url)) return@launch toast(app, "No app here can open that link")
                val open = Intent(Intent.ACTION_VIEW, android.net.Uri.parse(h.positionMs?.let { Handoff.at(h.url, it) } ?: h.url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                runCatching { app.startActivity(open) }.onFailure { toast(app, "No app here can open that link") }
            },
            onFailure = { toast(app, it.message ?: "Couldn't hand off") },
        )
    }
    fun listPcFiles(plugin: CompanionPlugin, path: String) = viewModelScope.launch { plugin.listFiles(path) }
    fun downloadPcFile(plugin: CompanionPlugin, path: String, keep: Boolean = true) = viewModelScope.launch { plugin.download(path, keep) }
    // App scope: an upload keeps going if you leave the browser.
    fun uploadPcFiles(plugin: CompanionPlugin, folder: String, files: List<OutgoingFile>) =
        container.appScope.launch { plugin.upload(folder, files) }

    // Automation rules
    val rules: StateFlow<List<Rule>> get() = container.automation.store.rules
    val ruleLog: StateFlow<List<RunEntry>> get() = container.automation.store.log
    fun rule(id: String): Rule? = container.automation.store.rule(id)
    fun saveRule(rule: Rule) = viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) { container.automation.store.save(rule) }
    /** Set by the rule editor on its way out: a new rule's id, or SUGGESTION + key for a suggestion given back. */
    var ruleArrival: String? = null
    fun deleteRule(id: String) = viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) { container.automation.store.delete(id) }
    fun setRuleEnabled(id: String, enabled: Boolean) = viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) { container.automation.store.setEnabled(id, enabled) }
    fun ruleNeedsFingerprint(rule: Rule) = rule.actions.any {
        ((it == Action.PcScreenshot || it == Action.SleepPc) && AppLock.enabled(getApplication())) ||
            (it is Action.RunCommand && pcCommandLocked(it.key))
    }
    fun testRule(rule: Rule) = viewModelScope.launch { container.automation.test(rule) }

    /** (address, name) of the phone's paired Bluetooth devices. The demo's are made up. */
    fun pairedBluetooth(): List<Pair<String, String>> =
        if (dev.vyoma.connect.BuildConfig.DEMO) listOf("demo:car" to "Car audio", "demo:buds" to "Earbuds")
        else container.automation.pairedBluetooth()

    fun needsBluetoothPermission(context: android.content.Context): Boolean = !dev.vyoma.connect.BuildConfig.DEMO &&
        context.checkSelfPermission(android.Manifest.permission.BLUETOOTH_CONNECT) != android.content.pm.PackageManager.PERMISSION_GRANTED

    /** The fingerprint of the Wi-Fi the phone is on; the demo has a pretend one. */
    fun currentWifi(): String? = if (dev.vyoma.connect.BuildConfig.DEMO) "demo-wifi" else container.automation.currentWifi()
    /** The connected PC's commands, for a "Run command" action. */
    fun pcCommands(): List<dev.vyoma.connect.protocol.plugin.builtin.RemoteCommand> =
        dev.vyoma.connect.service.Commands.primary(container)?.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.commands?.value.orEmpty()
    /** Whether the connected PC marked its command [key] to ask for the fingerprint when run by hand. */
    fun pcCommandLocked(key: String): Boolean = pcCommands().any { c -> c.key == key && c.fingerprint }
    /** Whether a companion has ever paired (walk-away rules and some actions need it). */
    fun hasCompanion(): Boolean = container.hub.value?.devices?.value?.values?.any { it.state.value.identity.featuresOf != null && it.state.value.isPaired } == true

    fun setPalette(p: Palette) = viewModelScope.launch { settings.setPalette(p) }
    fun setDarkMode(m: DarkMode) = viewModelScope.launch { settings.setDarkMode(m) }
    fun setLiteGlass(enabled: Boolean) = viewModelScope.launch { settings.setLiteGlass(enabled) }
    fun setCanvasGlow(enabled: Boolean) = viewModelScope.launch { settings.setCanvasGlow(enabled) }
    val outdoorMode: StateFlow<Boolean> = settings.outdoorMode.stateIn(viewModelScope, SharingStarted.Eagerly, true)
    fun setOutdoorMode(enabled: Boolean) = viewModelScope.launch { settings.setOutdoorMode(enabled) }

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

private fun toast(context: android.content.Context, text: String) =
    android.widget.Toast.makeText(context, text, android.widget.Toast.LENGTH_SHORT).show()
