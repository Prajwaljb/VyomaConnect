package dev.vyoma.connect.platform

import android.Manifest
import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.PixelFormat
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.ClipEntry
import dev.vyoma.connect.protocol.plugin.builtin.ClipboardBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.put
import java.io.File

/**
 * The phone clipboard, synced with paired PCs, plus a short history.
 *
 * Android only lets the focused app read the clipboard. We read it:
 *  - whenever Vyoma itself is in front (listener + [readNow] on resume),
 *  - from the tile / widget / notification action, which open a tiny focusable window,
 *  - automatically in the background when the user granted READ_LOGS and overlay access over ADB:
 *    the system logs a "Denying clipboard access to <us>" line on every copy, which we take as
 *    the cue to grab focus for a moment with a 1px overlay and read the new clip.
 */
class AndroidClipboard(private val context: Context, private val scope: CoroutineScope) : ClipboardBridge {
    private val cm = context.getSystemService(ClipboardManager::class.java)
    private val main = Handler(Looper.getMainLooper())

    private val _current = MutableStateFlow<ClipEntry?>(null)
    override val current: StateFlow<ClipEntry?> = _current.asStateFlow()

    private val historyFile = File(context.filesDir, "clipboard.json")
    private val _history = MutableStateFlow<List<ClipEntry>>(emptyList())
    val history: StateFlow<List<ClipEntry>> = _history.asStateFlow()

    private var watcher: Job? = null

    init {
        scope.launch(Dispatchers.IO) {
            _history.value = runCatching { decode(historyFile.readText()) }.getOrNull().orEmpty()
            if (_current.value == null) _current.value = _history.value.firstOrNull()
        }
        main.post { cm.addPrimaryClipChangedListener { readNow() } }
    }

    /** Reads the clipboard if we're allowed to right now (foreground/focused). Returns the text if it changed. */
    fun readNow(): String? {
        val clip = runCatching { cm.primaryClip }.getOrNull() ?: return null
        if (clip.itemCount == 0) return null
        val text = clip.getItemAt(0).coerceToText(context)?.toString()?.takeIf { it.isNotBlank() } ?: return null
        if (text == _current.value?.text) return null
        // Respect apps that mark copies as sensitive (password managers): keep them local.
        if (clip.description?.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true) return null
        record(ClipEntry(text, System.currentTimeMillis(), origin = null))
        return text
    }

    override fun applyRemote(text: String, fromDeviceId: String) {
        record(ClipEntry(text, System.currentTimeMillis(), origin = fromDeviceId))
        main.post { runCatching { cm.setPrimaryClip(ClipData.newPlainText("Vyoma", text)) } }
    }

    /** Copies a history item back onto the clipboard (and so to the PC). */
    fun copy(text: String) {
        record(ClipEntry(text, System.currentTimeMillis(), origin = null))
        main.post { runCatching { cm.setPrimaryClip(ClipData.newPlainText("Vyoma", text)) } }
    }

    fun clearHistory() {
        _history.value = emptyList()
        persist()
    }

    private fun record(entry: ClipEntry) {
        _current.value = entry
        _history.update { old -> (listOf(entry) + old.filterNot { it.text == entry.text }).take(HISTORY_SIZE) }
        persist()
    }

    private fun persist() {
        val snapshot = encode(_history.value)
        scope.launch(Dispatchers.IO) {
            runCatching {
                val tmp = File(historyFile.path + ".tmp")
                tmp.writeText(snapshot)
                tmp.renameTo(historyFile)
            }
        }
    }

    // ---- background sync via the logcat signal ----------------------------------------------

    /** Both one-time ADB grants are in place. */
    fun backgroundSyncAvailable(): Boolean =
        context.checkSelfPermission(Manifest.permission.READ_LOGS) == PackageManager.PERMISSION_GRANTED && canDrawOverlays()

    fun canDrawOverlays(): Boolean = Settings.canDrawOverlays(context)

    /**
     * Starts following the system log. Start it while the app is in front: Android 13+ asks the
     * user once per process to allow log access, and only shows that prompt to a foreground app.
     */
    fun startBackgroundSync() {
        if (watcher?.isActive == true || !backgroundSyncAvailable()) return
        watcher = scope.launch(Dispatchers.IO) {
            val marker = "Denying clipboard access to ${context.packageName}"
            val process = runCatching {
                ProcessBuilder("logcat", "-T", "1", "-v", "brief", "ClipboardService:E", "*:S").redirectErrorStream(true).start()
            }.getOrElse {
                VLog.w(TAG, "can't read logcat: ${it.message}")
                return@launch
            }
            VLog.i(TAG, "background clipboard sync on")
            try {
                process.inputStream.bufferedReader().useLines { lines ->
                    for (line in lines) {
                        if (!isActive) break
                        if (marker in line) main.post(::grabFocusAndRead)
                    }
                }
            } finally {
                process.destroy()
                VLog.i(TAG, "background clipboard sync stopped")
            }
        }
    }

    private var overlay: View? = null

    /** A 1px, invisible, focusable overlay: focus is what the clipboard check wants. */
    @SuppressLint("ClickableViewAccessibility")
    private fun grabFocusAndRead() {
        if (overlay != null || !canDrawOverlays()) return
        val wm = context.getSystemService(WindowManager::class.java)
        val view = object : View(context) {
            override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
                super.onWindowFocusChanged(hasWindowFocus)
                if (hasWindowFocus) {
                    readNow()
                    removeOverlay(wm)
                }
            }
        }
        val params = WindowManager.LayoutParams(
            1, 1,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or WindowManager.LayoutParams.FLAG_ALT_FOCUSABLE_IM,
            PixelFormat.TRANSLUCENT,
        ).apply { gravity = Gravity.TOP or Gravity.START }
        runCatching { wm.addView(view, params) }.onSuccess {
            overlay = view
            // Never linger if focus doesn't come.
            main.postDelayed({ removeOverlay(wm) }, 600)
        }
    }

    private fun removeOverlay(wm: WindowManager) {
        val v = overlay ?: return
        overlay = null
        runCatching { wm.removeViewImmediate(v) }
    }

    private companion object {
        const val TAG = "Clipboard"
        const val HISTORY_SIZE = 30
        const val MAX_STORED_CHARS = 20_000

        fun encode(list: List<ClipEntry>) = JsonArray(list.map {
            buildJsonObject {
                put("text", it.text.take(MAX_STORED_CHARS))
                put("timestamp", it.timestamp)
                it.origin?.let { o -> put("origin", o) }
            }
        }).toString()

        fun decode(text: String): List<ClipEntry> = Json.parseToJsonElement(text).jsonArray.mapNotNull { el ->
            val o = el as? JsonObject ?: return@mapNotNull null
            val t = (o["text"] as? JsonPrimitive)?.content ?: return@mapNotNull null
            ClipEntry(t, o["timestamp"]?.jsonPrimitive?.long ?: 0, (o["origin"] as? JsonPrimitive)?.content)
        }
    }
}
