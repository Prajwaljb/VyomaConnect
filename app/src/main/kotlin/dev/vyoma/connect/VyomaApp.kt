package dev.vyoma.connect

import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import android.app.Activity
import android.app.Application
import android.os.Bundle
import android.provider.Settings
import android.util.Log
import dev.vyoma.connect.automation.RuleEngine
import dev.vyoma.connect.data.PcSpecsStore
import dev.vyoma.connect.data.SettingsRepository
import dev.vyoma.connect.demo.Demo
import dev.vyoma.connect.demo.DemoDiagnostics
import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.FileCredentialStore
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.FileTrustStore
import dev.vyoma.connect.designsystem.AppCanvas
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.merge
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.filterNotNull
import dev.vyoma.connect.designsystem.CanvasWallpaper
import dev.vyoma.connect.designsystem.GlassQuality
import dev.vyoma.connect.platform.AndroidClipboard
import dev.vyoma.connect.platform.AppLock
import dev.vyoma.connect.platform.FrameBudget
import dev.vyoma.connect.platform.AndroidShareSink
import dev.vyoma.connect.platform.AndroidContacts
import dev.vyoma.connect.platform.AndroidSms
import dev.vyoma.connect.platform.CallMonitor
import dev.vyoma.connect.platform.SignalMonitor
import dev.vyoma.connect.platform.BatteryMonitor
import dev.vyoma.connect.platform.MediaHub
import dev.vyoma.connect.platform.NotificationBridge
import dev.vyoma.connect.platform.Ringer
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.plugin.builtin.ClipboardPlugin
import dev.vyoma.connect.protocol.plugin.builtin.ConnectivityReportPlugin
import dev.vyoma.connect.protocol.plugin.builtin.ContactsPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SmsPlugin
import dev.vyoma.connect.protocol.plugin.builtin.TelephonyPlugin
import dev.vyoma.connect.protocol.plugin.builtin.FindMyPhonePlugin
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import dev.vyoma.connect.protocol.plugin.builtin.MousePadPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PresenterPlugin
import dev.vyoma.connect.protocol.plugin.builtin.MprisPlugin
import dev.vyoma.connect.protocol.plugin.builtin.NotificationsPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.protocol.plugin.builtin.RunCommandPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SharePlugin
import dev.vyoma.connect.protocol.plugin.builtin.SystemVolumePlugin
import dev.vyoma.connect.service.Notifications
import dev.vyoma.connect.service.AndroidNetwork
import dev.vyoma.connect.widget.Widgets
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File

class VyomaApp : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        dev.vyoma.connect.mirror.PhoneMirror.restoreVolume(this)
        VLog.sink = VLog.Sink { level, tag, message, error ->
            when (level) {
                VLog.Level.DEBUG -> Log.d("Vyoma/$tag", message, error)
                VLog.Level.INFO -> Log.i("Vyoma/$tag", message, error)
                VLog.Level.WARN -> Log.w("Vyoma/$tag", message, error)
                VLog.Level.ERROR -> Log.e("Vyoma/$tag", message, error)
            }
        }
        if (BuildConfig.DEMO) {
            Demo.seedClipboard(this)
            DemoDiagnostics.install(this)
        }
        AppLock.install(this)
        dev.vyoma.connect.platform.TextsLock.install(this)
        container = AppContainer(this)
        if (BuildConfig.DEMO) container.appScope.launch { DemoDiagnostics.reportCrashes(this@VyomaApp) }
        // Lighter glass: the user's setting, or found here when this phone can't keep up with the full kind.
        val budget = FrameBudget { container.appScope.launch { container.settings.recordSlowLaunch() } }
        budget.install(this)
        container.appScope.launch(Dispatchers.Main) {
            container.settings.liteGlass.collect { glass ->
                GlassQuality.lite = glass.on
                if (glass.chosen || glass.detected) budget.stop()
            }
        }
        container.appScope.launch(Dispatchers.Main) {
            container.settings.canvasGlow.collect { AppCanvas.glow = it }
        }
        // The PC's wallpaper colours for the canvas: the last ones kept, so the look holds while the
        // PC is away, and replaced whenever a companion sends new ones.
        val canvasPrefs = getSharedPreferences("ui", MODE_PRIVATE)
        canvasPrefs.getString("pc_wallpaper", null)?.let(::decodeWallpaper)?.let { AppCanvas.wallpaper = it }
        container.appScope.launch(Dispatchers.Main) {
            @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
            container.hub.filterNotNull().flatMapLatest { hub ->
                hub.devices.flatMapLatest { all ->
                    val each = all.values.map { d ->
                        d.state.map { d.plugin<CompanionPlugin>(CompanionPlugin.ID) }.distinctUntilChanged()
                            .flatMapLatest { it?.wallpaper ?: flowOf(null) }
                    }
                    if (each.isEmpty()) flowOf(null) else merge(*each.toTypedArray())
                }
            }.filterNotNull().distinctUntilChanged().collect { w ->
                val grid = CanvasWallpaper(w.cols, w.rows, w.colors.toIntArray())
                AppCanvas.wallpaper = grid
                canvasPrefs.edit().putString("pc_wallpaper", "${w.cols},${w.rows}," + w.colors.joinToString(",")).apply()
            }
        }
        // Each PC's specs, kept as its companion sends them (on every link) for its details page.
        PcSpecsStore.load(this)
        container.appScope.launch(Dispatchers.Main) {
            @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
            container.hub.filterNotNull().flatMapLatest { hub ->
                hub.devices.flatMapLatest { all ->
                    val each = all.values.map { d ->
                        d.state.map { d.plugin<CompanionPlugin>(CompanionPlugin.ID) }.distinctUntilChanged()
                            .flatMapLatest { p -> p?.specs?.map { s -> d.identity.featuresOf?.let { pc -> s?.let { Triple(pc, it.first, it.second) } } } ?: flowOf(null) }
                    }
                    if (each.isEmpty()) flowOf(null) else merge(*each.toTypedArray())
                }
            }.filterNotNull().collect { (pc, specs, body) -> PcSpecsStore.put(this@VyomaApp, pc, specs, body) }
        }
        // The PC turned something off for this phone and nothing on screen says so (a wake, a stopped cast).
        container.appScope.launch(Dispatchers.Main) {
            @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
            container.hub.filterNotNull().flatMapLatest { hub ->
                hub.devices.flatMapLatest { all ->
                    val each = all.values.map { d ->
                        d.state.map { d.plugin<CompanionPlugin>(CompanionPlugin.ID) }.distinctUntilChanged().flatMapLatest { it?.refused ?: emptyFlow() }
                    }
                    if (each.isEmpty()) emptyFlow() else merge(*each.toTypedArray())
                }
            }.collect { why ->
                // In the app's own status bubble while Home is on screen; a system toast otherwise.
                if (container.refusals.subscriptionCount.value == 0 || !container.refusals.tryEmit(why)) {
                    android.widget.Toast.makeText(this@VyomaApp, why, android.widget.Toast.LENGTH_LONG).show()
                }
            }
        }
        // Whether any of our activities is started: background work scales down when it isn't.
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            private var started = 0
            override fun onActivityStarted(activity: Activity) {
                started++
                container.uiVisible.value = true
            }
            override fun onActivityStopped(activity: Activity) {
                started = (started - 1).coerceAtLeast(0)
                if (started == 0) container.uiVisible.value = false
            }
            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
            override fun onActivityResumed(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = Unit
        })
        container.appScope.launch { Widgets.publishPreviews(this@VyomaApp) }
    }
}

/**
 * Hand-rolled DI: everything is lazy so cold start does no work beyond what the first
 * screen needs. The hub is created once (key generation happens off the main thread).
 */
class AppContainer(private val app: Application) {
    val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    /** True while one of our activities is on screen. */
    val uiVisible = MutableStateFlow(false)
    /** What the PC refused or couldn't do, for the main screen's status bubble while it's showing. */
    val refusals = MutableSharedFlow<String>(extraBufferCapacity = 4)
    val settings by lazy { SettingsRepository(app) }
    val network by lazy { AndroidNetwork(app) }
    val battery by lazy { BatteryMonitor(app) }
    val clipboard by lazy { AndroidClipboard(app, appScope) }
    val shareSink by lazy { AndroidShareSink(app, clipboard) }
    val media by lazy { MediaHub(hub, appScope) }
    val sms by lazy { AndroidSms(app) }
    val contacts by lazy { AndroidContacts(app) }
    val calls by lazy { CallMonitor(app, contacts) }
    val signal by lazy { SignalMonitor(app) }
    /** "When / Do" rules; started with the link service (and by its alarms). */
    val automation by lazy { RuleEngine(app, this) }

    private val hubLock = Mutex()
    private val _hub = MutableStateFlow<DeviceHub?>(null)
    val hub: StateFlow<DeviceHub?> = _hub.asStateFlow()

    /** Our TLS key pair, once the identity's loaded: the file server's SSH host key too. */
    @Volatile private var hostKey: java.security.KeyPair? = null
    val phoneFiles by lazy {
        dev.vyoma.connect.platform.AndroidPhoneFileServer(app) { hostKey ?: error("no identity yet") }
    }

    suspend fun obtainHub(): DeviceHub = hubLock.withLock {
        _hub.value ?: withContext(Dispatchers.IO) {
            val keyDir = File(app.noBackupFilesDir, "identity")
            val credStore = FileCredentialStore(File(keyDir, "credentials.p12"))
            val credentials = credStore.load()
                ?: LocalCredentials.generate(DeviceIdentity.newDeviceId()).also(credStore::save)
            hostKey = java.security.KeyPair(credentials.certificate.publicKey, credentials.privateKey)
            val name = settings.deviceName.first() ?: defaultDeviceName()
            val trustStore = FileTrustStore(File(keyDir, "trusted.json"))
            val demo = if (BuildConfig.DEMO) Demo.prepare(File(keyDir, "demo"), trustStore) else null
            DeviceHub(
                credentials = credentials,
                trustStore = trustStore,
                pluginSpecs = pluginSpecs(),
                scope = appScope,
                deviceName = name,
                deviceType = if (app.resources.configuration.smallestScreenWidthDp >= 600) DeviceType.TABLET else DeviceType.PHONE,
                lanConfig = network.lanConfig().let { if (demo != null) Demo.lanConfig(it) else it },
            ).also { hub -> demo?.let { Demo.start(hub, it, appScope) } }
        }.also { _hub.value = it }
    }

    private fun pluginSpecs() = listOf(
        PingPlugin.spec,
        BatteryPlugin.spec(battery.state),
        ClipboardPlugin.spec(clipboard),
        SharePlugin.spec(shareSink),
        NotificationsPlugin.spec(NotificationBridge),
        MprisPlugin.spec,
        FindMyPhonePlugin.spec { from ->
            Ringer.start(app, from)
            Notifications.ringing(app, from)
        },
        LockPlugin.spec,
        RunCommandPlugin.spec,
        SystemVolumePlugin.spec,
        MousePadPlugin.spec,
        PresenterPlugin.spec,
        SmsPlugin.spec(sms, dev.vyoma.connect.platform.TextsLock),
        TelephonyPlugin.spec(calls),
        ContactsPlugin.spec(contacts, dev.vyoma.connect.platform.TextsLock),
        dev.vyoma.connect.protocol.plugin.builtin.SftpPlugin.spec(phoneFiles),
        ConnectivityReportPlugin.spec(signal.states),
        CompanionPlugin.spec(shareSink),
    )

    fun defaultDeviceName(): String =
        Settings.Global.getString(app.contentResolver, Settings.Global.DEVICE_NAME)
            ?: android.os.Build.MODEL
}

val Application.container: AppContainer get() = (this as VyomaApp).container

/** "cols,rows,c1,c2,…" as saved above; null if it doesn't add up. */
private fun decodeWallpaper(saved: String): CanvasWallpaper? {
    val n = saved.split(',').mapNotNull { it.trim().toIntOrNull() }
    if (n.size < 3) return null
    val (cols, rows) = n[0] to n[1]
    val colors = n.drop(2)
    return if (cols > 0 && rows > 0 && colors.size == cols * rows) CanvasWallpaper(cols, rows, colors.toIntArray()) else null
}
