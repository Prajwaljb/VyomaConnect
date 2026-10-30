package dev.vyoma.connect.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.link.TransportKind
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.net.InetAddress

/**
 * The USB cable as a link. While a cable is plugged in we keep dialling the adb reverse tunnel
 * (`adb reverse tcp:1765 tcp:1716`, set up by the companion or by hand); refused connections cost
 * nothing. USB tethering needs nothing here: discovery already broadcasts on the tether
 * interface and [LanBackend.defaultTransport] labels those links USB. While a device is on the
 * cable the hub refuses Wi-Fi links to it, and pulling the cable drops its USB links at once so
 * Wi-Fi takes over without waiting for the dead tunnel to time out.
 */
class UsbLink(private val context: Context, private val hub: DeviceHub, private val scope: CoroutineScope) {
    private var receiver: BroadcastReceiver? = null
    private var dialer: Job? = null

    /** Whether a cable is in right now (the sticky USB state), for before the hub starts. */
    fun cableIn(): Boolean =
        context.registerReceiver(null, IntentFilter(ACTION_USB_STATE), Context.RECEIVER_NOT_EXPORTED)?.getBooleanExtra("connected", false) == true

    fun start() {
        if (receiver != null) return
        val r = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = onUsbState(intent.getBooleanExtra("connected", false))
        }
        receiver = r
        // Sticky: the current state arrives straight away.
        context.registerReceiver(r, IntentFilter(ACTION_USB_STATE), Context.RECEIVER_NOT_EXPORTED)?.let {
            onUsbState(it.getBooleanExtra("connected", false))
        }
    }

    fun stop() {
        receiver?.let { runCatching { context.unregisterReceiver(it) } }
        receiver = null
        dialer?.cancel()
        dialer = null
    }

    private fun onUsbState(connected: Boolean) {
        if (!connected) {
            if (dialer != null) hub.cableUnplugged()
            dialer?.cancel()
            dialer = null
            return
        }
        if (dialer?.isActive == true) return
        VLog.d(TAG, "cable in: trying the adb tunnel on ${LanBackend.USB_REVERSE_PORT}")
        hub.cablePlugged()
        dialer = scope.launch {
            val loopback = InetAddress.getLoopbackAddress()
            val pluggedAt = System.currentTimeMillis()
            // Every 2 s while the companion is likely still setting the tunnel up (it takes a few
            // seconds after plug-in), then backing off to every 30 s if nothing answers.
            // While the cable link is up, sleep until it drops instead of polling.
            var wait = 5_000L
            while (isActive) {
                if (usbLinked().first()) {
                    usbLinked().first { !it }
                    wait = 2_000
                    continue
                }
                hub.dial(loopback, LanBackend.USB_REVERSE_PORT)
                wait = if (System.currentTimeMillis() - pluggedAt < FAST_DIAL_MS) 2_000 else (wait * 2).coerceAtMost(30_000)
                delay(wait)
            }
        }
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    private fun usbLinked(): Flow<Boolean> = hub.devices.flatMapLatest { all ->
        if (all.isEmpty()) flowOf(false)
        else combine(all.values.map { it.state }) { states -> states.any { TransportKind.USB in it.transports } }
    }.distinctUntilChanged()

    private companion object {
        const val TAG = "UsbLink"
        const val FAST_DIAL_MS = 30_000L
        const val ACTION_USB_STATE = "android.hardware.usb.action.USB_STATE"
    }
}
