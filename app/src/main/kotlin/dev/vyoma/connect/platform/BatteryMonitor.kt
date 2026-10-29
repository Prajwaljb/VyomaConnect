package dev.vyoma.connect.platform

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import androidx.core.content.ContextCompat
import dev.vyoma.connect.protocol.plugin.builtin.BatteryState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Phone battery as a flow. ACTION_BATTERY_CHANGED is sticky, so the first value is immediate. */
class BatteryMonitor(context: Context) {
    private val _state = MutableStateFlow<BatteryState?>(null)
    val state: StateFlow<BatteryState?> = _state.asStateFlow()

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) = update(intent)
    }

    init {
        ContextCompat.registerReceiver(
            context.applicationContext, receiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED,
        )?.let(::update)
    }

    private fun update(intent: Intent) {
        val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, 100).coerceAtLeast(1)
        if (level < 0) return
        val percent = level * 100 / scale
        val plugged = intent.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0
        // Same threshold Android uses for its own low-battery warning.
        _state.value = BatteryState(percent, plugged, low = !plugged && percent <= 15)
    }
}
