package dev.vyoma.connect

import android.app.Application
import android.provider.Settings
import android.util.Log
import dev.vyoma.connect.data.SettingsRepository
import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.FileCredentialStore
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.FileTrustStore
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.service.AndroidNetwork
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
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
        container = AppContainer(this)
    }
}

/**
 * Hand-rolled DI: everything is lazy so cold start does no work beyond what the first
 * screen needs. The hub is created once (key generation happens off the main thread).
 */
class AppContainer(private val app: Application) {
    val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val settings by lazy { SettingsRepository(app) }
    val network by lazy { AndroidNetwork(app) }

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
            DeviceHub(
                credentials = credentials,
                trustStore = FileTrustStore(File(keyDir, "trusted.json")),
                pluginSpecs = listOf(PingPlugin.spec),
                scope = appScope,
                deviceName = name,
                deviceType = if (app.resources.configuration.smallestScreenWidthDp >= 600) DeviceType.TABLET else DeviceType.PHONE,
                lanConfig = network.lanConfig(),
            )
        }.also { _hub.value = it }
    }

    fun defaultDeviceName(): String =
        Settings.Global.getString(app.contentResolver, Settings.Global.DEVICE_NAME)
            ?: android.os.Build.MODEL
}

val Application.container: AppContainer get() = (this as VyomaApp).container
