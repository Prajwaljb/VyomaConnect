package dev.vyoma.connect

import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import android.app.Activity
import android.app.Application
import android.os.Bundle
import android.provider.Settings
import android.util.Log
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
import dev.vyoma.connect.platform.AndroidClipboard
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
        container = AppContainer(this)
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

    private val hubLock = Mutex()
    private val _hub = MutableStateFlow<DeviceHub?>(null)
    val hub: StateFlow<DeviceHub?> = _hub.asStateFlow()

    suspend fun obtainHub(): DeviceHub = hubLock.withLock {
        _hub.value ?: withContext(Dispatchers.IO) {
            val keyDir = File(app.noBackupFilesDir, "identity")
            val credStore = FileCredentialStore(File(keyDir, "credentials.p12"))
            val credentials = credStore.load()
                ?: LocalCredentials.generate(DeviceIdentity.newDeviceId()).also(credStore::save)
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
        SmsPlugin.spec(sms),
        TelephonyPlugin.spec(calls),
        ContactsPlugin.spec(contacts),
        ConnectivityReportPlugin.spec(signal.states),
        CompanionPlugin.spec(shareSink),
    )

    fun defaultDeviceName(): String =
        Settings.Global.getString(app.contentResolver, Settings.Global.DEVICE_NAME)
            ?: android.os.Build.MODEL
}

val Application.container: AppContainer get() = (this as VyomaApp).container
