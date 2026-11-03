package dev.vyoma.connect.data

import android.content.Context
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.core.stringSetPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.Palette
import dev.vyoma.connect.designsystem.ThemeSettings
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import dev.vyoma.connect.platform.FrameBudget

private val Context.dataStore by preferencesDataStore("settings")

class SettingsRepository(private val context: Context) {
    private object Keys {
        val palette = stringPreferencesKey("palette")
        val darkMode = stringPreferencesKey("dark_mode")
        /** The user's own choice of lighter glass; absent means automatic. */
        val liteGlass = booleanPreferencesKey("lite_glass")
        /** Set once FrameBudget finds this phone can't keep up with the full glass. */
        val liteGlassAuto = booleanPreferencesKey("lite_glass_auto")
        /** Launches FrameBudget found too slow so far; two turn lighter glass on. */
        val slowLaunches = intPreferencesKey("slow_launches")
        /** The accent glows behind the page; on unless turned off. */
        val canvasGlow = booleanPreferencesKey("canvas_glow")
        val deviceName = stringPreferencesKey("device_name")
        val onboarded = booleanPreferencesKey("onboarded")
        val manualHosts = stringSetPreferencesKey("manual_hosts")
        /** Pause in the background away from Wi-Fi; on unless turned off. */
        val outdoorMode = booleanPreferencesKey("outdoor_mode")
    }

    private val prefs: Flow<Preferences> get() = context.dataStore.data

    val theme: Flow<ThemeSettings> = prefs.map { p ->
        ThemeSettings(
            palette = p[Keys.palette]?.let { runCatching { Palette.valueOf(it) }.getOrNull() } ?: Palette.DYNAMIC,
            darkMode = p[Keys.darkMode]?.let { runCatching { DarkMode.valueOf(it) }.getOrNull() } ?: DarkMode.SYSTEM,
        )
    }.distinctUntilChanged()

    val deviceName: Flow<String?> = prefs.map { it[Keys.deviceName] }.distinctUntilChanged()
    val onboarded: Flow<Boolean> = prefs.map { it[Keys.onboarded] ?: false }.distinctUntilChanged()

    suspend fun setPalette(palette: Palette) = context.dataStore.edit { it[Keys.palette] = palette.name }
    suspend fun setDarkMode(mode: DarkMode) = context.dataStore.edit { it[Keys.darkMode] = mode.name }

    /**
     * Lighter glass: the user's choice if they made one, else what FrameBudget found, else whether
     * this looks like a modest phone ([FrameBudget.looksModest]), so one starts light from the
     * first launch instead of after two slow ones.
     */
    val liteGlass: Flow<LiteGlass> = prefs.map { p ->
        val detected = p[Keys.liteGlassAuto] ?: modest
        LiteGlass(on = p[Keys.liteGlass] ?: detected, chosen = p[Keys.liteGlass] != null, detected = detected)
    }.distinctUntilChanged()
    private val modest by lazy { FrameBudget.looksModest(context) }
    suspend fun setLiteGlass(enabled: Boolean) = context.dataStore.edit { it[Keys.liteGlass] = enabled }
    /** One more launch found too slow; the second turns lighter glass on (one could be a fluke). */
    suspend fun recordSlowLaunch() = context.dataStore.edit {
        val n = (it[Keys.slowLaunches] ?: 0) + 1
        it[Keys.slowLaunches] = n
        if (n >= 2) it[Keys.liteGlassAuto] = true
    }
    val canvasGlow: Flow<Boolean> = prefs.map { it[Keys.canvasGlow] ?: true }.distinctUntilChanged()
    suspend fun setCanvasGlow(enabled: Boolean) = context.dataStore.edit { it[Keys.canvasGlow] = enabled }
    suspend fun setDeviceName(name: String) = context.dataStore.edit { it[Keys.deviceName] = name }
    /** PCs added by address (Tailscale IP, MagicDNS name, another subnet). */
    val manualHosts: Flow<Set<String>> = prefs.map { it[Keys.manualHosts].orEmpty() }.distinctUntilChanged()
    suspend fun addManualHost(host: String) = context.dataStore.edit { it[Keys.manualHosts] = it[Keys.manualHosts].orEmpty() + host }
    suspend fun removeManualHost(host: String) = context.dataStore.edit { it[Keys.manualHosts] = it[Keys.manualHosts].orEmpty() - host }


    val outdoorMode: Flow<Boolean> = prefs.map { it[Keys.outdoorMode] ?: true }.distinctUntilChanged()
    suspend fun setOutdoorMode(enabled: Boolean) = context.dataStore.edit { it[Keys.outdoorMode] = enabled }

    suspend fun setOnboarded() = context.dataStore.edit { it[Keys.onboarded] = true }
}

/** [chosen]: the user set it themselves; [detected]: this phone was found too slow for full glass. */
data class LiteGlass(val on: Boolean, val chosen: Boolean, val detected: Boolean)
