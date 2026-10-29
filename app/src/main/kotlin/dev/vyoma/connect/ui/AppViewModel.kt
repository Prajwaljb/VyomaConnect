package dev.vyoma.connect.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.vyoma.connect.container
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.Palette
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.protocol.crypto.Certificates
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.service.LinkService
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.filterNotNull
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

    fun startService() = LinkService.start(getApplication())

    fun refresh() = LinkService.start(getApplication(), refresh = true)

    private fun device(id: String): Device? = container.hub.value?.device(id)

    fun requestPair(id: String) = act(id) { it.requestPairing() }
    fun acceptPair(id: String) = act(id) { it.acceptPairing() }
    fun rejectPair(id: String) = act(id) { it.rejectPairing() }
    fun unpair(id: String) = act(id) { it.unpair() }
    fun ping(id: String) = act(id) { it.plugin<PingPlugin>(PingPlugin.ID)?.ping("Ping from Vyoma") }
    fun forget(id: String) { container.hub.value?.forget(id) }

    fun fingerprint(id: String): String? = device(id)?.peerCertificate?.let(Certificates::sha256Fingerprint)

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
