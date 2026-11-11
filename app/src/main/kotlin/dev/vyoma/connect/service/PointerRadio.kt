package dev.vyoma.connect.service

import android.content.Context
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PointerChannel
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * The touchpad's Bluetooth channel (Settings > Steadier over Bluetooth), kept for a while after the
 * touchpad leaves the screen. Opening it again means paging the PC's radio, which takes seconds on a
 * crowded 2.4 GHz band, and the moves would go over Wi-Fi meanwhile; kept, resting (no idle frames),
 * it's there at once when the touchpad comes back from Home, another app or a screen lock.
 */
object PointerRadio {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val _current = MutableStateFlow<PointerChannel?>(null)
    /** The channel while the touchpad is on screen and it's up; null: send over the link. */
    val current: StateFlow<PointerChannel?> = _current.asStateFlow()

    private var channel: PointerChannel? = null
    private var owner: CompanionPlugin? = null
    private var linger: Job? = null

    /**
     * Keeps the channel to [companion] up while the touchpad is on screen; returns only if the PC
     * refuses it (then the link it is, until the touchpad comes back). Call on the main thread.
     */
    suspend fun hold(context: Context, companion: CompanionPlugin) {
        linger?.cancel()
        linger = null
        if (owner !== companion) drop()
        owner = companion
        try {
            var retry = FIRST_RETRY_MS
            while (true) {
                val live = channel?.takeIf { it.alive }
                if (live != null) {
                    live.resting = false
                    _current.value = live
                    while (live.alive) delay(250)
                    _current.value = null
                    channel = null
                    retry = FIRST_RETRY_MS
                    continue
                }
                val opened = companion.openPointer(BluetoothPointerChannel(context, companion.pcId))
                // The PC said no: don't ask it again every few seconds.
                channel = opened.getOrElse { return } ?: run {
                    delay(retry)
                    retry = (retry * 2).coerceAtMost(MAX_RETRY_MS)
                    null
                }
            }
        } finally {
            _current.value = null
            channel?.resting = true
            linger = scope.launch {
                delay(LINGER_MS)
                drop()
            }
        }
    }

    private fun drop() {
        channel?.close()
        channel = null
        _current.value = null
        owner?.closePointer()
        owner = null
    }

    private const val LINGER_MS = 30_000L
    private const val FIRST_RETRY_MS = 1_000L
    private const val MAX_RETRY_MS = 5_000L
}
