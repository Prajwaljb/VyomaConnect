package dev.vyoma.connect.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.PowerManager
import androidx.core.content.ContextCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Whether the screen is on, from the system's own screen on/off broadcasts while [start]ed: with it
 * off nobody is looking at a PC's state, so the service looks for an offline PC far less often.
 */
class ScreenOn(private val context: Context) {
    private val power = context.getSystemService(PowerManager::class.java)
    private val _on = MutableStateFlow(power.isInteractive)
    val on: StateFlow<Boolean> = _on.asStateFlow()
    private var receiver: BroadcastReceiver? = null

    fun start() {
        if (receiver != null) return
        val r = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                _on.value = intent.action == Intent.ACTION_SCREEN_ON || power.isInteractive
            }
        }
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_SCREEN_OFF)
        }
        ContextCompat.registerReceiver(context, r, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        receiver = r
        _on.value = power.isInteractive
    }

    fun stop() {
        receiver?.let { runCatching { context.unregisterReceiver(it) } }
        receiver = null
    }
}
