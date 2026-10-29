package dev.vyoma.connect.data

import android.content.Context
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.Palette
import dev.vyoma.connect.designsystem.ThemeSettings
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map

private val Context.dataStore by preferencesDataStore("settings")

class SettingsRepository(private val context: Context) {
    private object Keys {
        val palette = stringPreferencesKey("palette")
        val darkMode = stringPreferencesKey("dark_mode")
        val amoled = booleanPreferencesKey("amoled")
        val deviceName = stringPreferencesKey("device_name")
        val onboarded = booleanPreferencesKey("onboarded")
    }

    private val prefs: Flow<Preferences> get() = context.dataStore.data

    val theme: Flow<ThemeSettings> = prefs.map { p ->
        ThemeSettings(
            palette = p[Keys.palette]?.let { runCatching { Palette.valueOf(it) }.getOrNull() } ?: Palette.DYNAMIC,
            darkMode = p[Keys.darkMode]?.let { runCatching { DarkMode.valueOf(it) }.getOrNull() } ?: DarkMode.SYSTEM,
            amoled = p[Keys.amoled] ?: false,
        )
    }.distinctUntilChanged()

    val deviceName: Flow<String?> = prefs.map { it[Keys.deviceName] }.distinctUntilChanged()
    val onboarded: Flow<Boolean> = prefs.map { it[Keys.onboarded] ?: false }.distinctUntilChanged()

    suspend fun setPalette(palette: Palette) = context.dataStore.edit { it[Keys.palette] = palette.name }
    suspend fun setDarkMode(mode: DarkMode) = context.dataStore.edit { it[Keys.darkMode] = mode.name }
    suspend fun setAmoled(enabled: Boolean) = context.dataStore.edit { it[Keys.amoled] = enabled }
    suspend fun setDeviceName(name: String) = context.dataStore.edit { it[Keys.deviceName] = name }
    suspend fun setOnboarded() = context.dataStore.edit { it[Keys.onboarded] = true }
}
