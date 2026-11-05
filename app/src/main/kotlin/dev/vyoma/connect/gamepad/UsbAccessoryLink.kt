package dev.vyoma.connect.gamepad

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.os.Bundle
import android.os.ParcelFileDescriptor
import dev.vyoma.connect.protocol.VLog
import java.io.FileOutputStream
import java.io.OutputStream

/**
 * The game controller's cable without USB debugging: the PC companion switches the phone into USB
 * accessory mode ("Vyoma / Companion"), Android opens [AccessoryActivity], and the controller then
 * writes its frames here (the companion reads them off USB). See desktop/companion/src/accessory.rs.
 */
object UsbAccessoryLink {
    private const val TAG = "Accessory"
    private var fd: ParcelFileDescriptor? = null
    private var out: OutputStream? = null

    fun attach(context: Context, accessory: UsbAccessory) = synchronized(this) {
        close()
        val usb = context.getSystemService(UsbManager::class.java)
        fd = runCatching { usb.openAccessory(accessory) }.getOrNull()
        out = fd?.let { LatestFrame(FileOutputStream(it.fileDescriptor)) }
        VLog.i(TAG, if (out != null) "PC companion's accessory open" else "couldn't open the PC companion's accessory")
    }

    /** Where to write the controller's frames, while the accessory is open. */
    fun output(context: Context): OutputStream? = synchronized(this) {
        if (out == null) {
            // Opened before (Android only shows the dialog once): pick it up again.
            val usb = context.getSystemService(UsbManager::class.java)
            usb.accessoryList?.firstOrNull { it.manufacturer == "Vyoma" && it.model == "Companion" && usb.hasPermission(it) }?.let { a ->
                fd = runCatching { usb.openAccessory(a) }.getOrNull()
                out = fd?.let { LatestFrame(FileOutputStream(it.fileDescriptor)) }
            }
        }
        out
    }

    fun close() = synchronized(this) {
        runCatching { out?.close() }
        runCatching { fd?.close() }
        out = null
        fd = null
    }

    /** Whether the phone is plugged into a computer (not just a charger). */
    fun cablePlugged(context: Context): Boolean {
        val state = context.registerReceiver(null, IntentFilter("android.hardware.usb.action.USB_STATE"))
        return state?.getBooleanExtra("connected", false) == true
    }
}

/**
 * Writes on a thread of its own, keeping only the newest frame: a USB or Bluetooth write blocks
 * until the PC reads, so with nothing reading (companion not bridging yet, or gone) the controller
 * would freeze. A write pending longer than [STALL_MS] fails the stream, and the controller falls
 * back to Wi-Fi.
 */
internal class LatestFrame(private val out: OutputStream) : OutputStream() {
    private val lock = Object()
    private var next: ByteArray? = null
    @Volatile private var busySince = 0L
    @Volatile private var failed = false
    @Volatile private var closed = false

    init {
        Thread({
            try {
                while (!closed) {
                    val frame = synchronized(lock) {
                        while (next == null && !closed) lock.wait()
                        next.also { next = null }
                    } ?: break
                    busySince = System.nanoTime()
                    out.write(frame)
                    busySince = 0L
                }
            } catch (_: Exception) {
                failed = true
            }
        }, "controller-out").apply { isDaemon = true; start() }
    }

    override fun write(b: ByteArray, off: Int, len: Int) {
        val busy = busySince
        if (failed || closed || (busy != 0L && System.nanoTime() - busy > STALL_MS * 1_000_000)) {
            throw java.io.IOException("frames not read")
        }
        synchronized(lock) { next = b.copyOfRange(off, off + len); lock.notify() }
    }

    override fun write(b: Int) = write(byteArrayOf(b.toByte()), 0, 1)

    override fun close() {
        closed = true
        synchronized(lock) { lock.notify() }
        runCatching { out.close() }
    }

    private companion object { const val STALL_MS = 500L }
}

/** Android opens this when the companion's accessory appears; it hands it over and closes at once. */
class AccessoryActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handle(intent)
        finish()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    private fun handle(intent: Intent?) {
        val accessory = if (android.os.Build.VERSION.SDK_INT >= 33) intent?.getParcelableExtra(UsbManager.EXTRA_ACCESSORY, UsbAccessory::class.java)
        else @Suppress("DEPRECATION") intent?.getParcelableExtra<UsbAccessory>(UsbManager.EXTRA_ACCESSORY)
        if (accessory != null) UsbAccessoryLink.attach(applicationContext, accessory)
    }
}
