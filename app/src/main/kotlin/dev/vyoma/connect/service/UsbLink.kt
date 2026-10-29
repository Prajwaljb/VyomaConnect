package dev.vyoma.connect.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.link.TransportKind
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
 * interface and [LanBackend.defaultTransport] labels those links USB.
 */
class UsbLink(private val context: Context, private val hub: DeviceHub, private val scope: CoroutineScope) {
    private var receiver: BroadcastReceiver? = null
    private var dialer: Job? = null

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
            dialer?.cancel()
            dialer = null
            return
        }
        if (dialer?.isActive == true) return
        VLog.d(TAG, "cable in: trying the adb tunnel on ${LanBackend.USB_REVERSE_PORT}")
        dialer = scope.launch {
            val loopback = InetAddress.getLoopbackAddress()
            // Quick while a tunnel may be coming up, then every 30 s if nothing answers.
            var wait = 5_000L
            while (isActive) {
                val overUsb = hub.devices.value.values.any { TransportKind.USB in it.state.value.transports }
                if (overUsb) {
                    wait = 5_000
                } else {
                    hub.dial(loopback, LanBackend.USB_REVERSE_PORT)
                    wait = (wait * 2).coerceAtMost(30_000)
                }
                delay(wait)
            }
        }
    }

    private companion object {
        const val TAG = "UsbLink"
        const val ACTION_USB_STATE = "android.hardware.usb.action.USB_STATE"
    }
}
