package dev.vyoma.connect.platform

import kotlinx.coroutines.delay
import android.os.SystemClock
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
        // The text itself, not coerceToText: a copied photo or file would become its content:// address.
        val text = clip.getItemAt(0).text?.toString()?.takeIf { it.isNotBlank() } ?: return null
        if (text == _current.value?.text) return null
        // Respect apps that mark copies as sensitive (password managers): keep them local.
        if (clip.description?.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true) return null
        record(ClipEntry(text, System.currentTimeMillis(), origin = null))
        VLog.i(TAG, "local copy picked up: ${text.length} chars")
        return text
    }

    override fun applyRemote(text: String, fromDeviceId: String) {
        VLog.i(TAG, "clipboard from peer: ${text.length} chars")
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
        // The PC sends an empty clip when it copies something that isn't text (an image, files).
        if (entry.text.isBlank()) return
        _current.value = entry
        // The same text copied again moves to the top; whitespace differences (a trailing newline
        // from one app, not another) don't make it a second entry.
        val key = sameText(entry.text)
        _history.update { old -> (listOf(entry) + old.filterNot { sameText(it.text) == key }).take(HISTORY_SIZE) }
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

    /** Whether copies here really reach the PC by themselves: [SyncState.ON] once the system log is readable. */
    enum class SyncState { NEEDS_GRANT, WAITING, ON }

    private val _syncState = MutableStateFlow(SyncState.NEEDS_GRANT)
    val syncState: StateFlow<SyncState> = _syncState.asStateFlow()
    /**
     * Follows the system log. Call it with Vyoma in front only: Android 13+ asks about each new
     * logcat reader whether it may see other apps' lines, it can only ask an app in front, and it
     * refuses one started in the background without asking, and keeps refusing for about a minute
     * after (what broke this: the service started it at launch, so opening Vyoma never got asked).
     * [again] starts a fresh reader, so Android asks again.
     */
    fun startBackgroundSync(again: Boolean = false) {
        if (!backgroundSyncAvailable()) {
            _syncState.value = SyncState.NEEDS_GRANT
            return
        }
        // A reader Android turned down stays unanswered: opening Vyoma again asks again, once its
        // ~60 s memory of the refusal is over.
        val unanswered = _syncState.value == SyncState.WAITING && SystemClock.elapsedRealtime() - askedAt > ASK_AGAIN_AFTER_MS
        if (!again && watcher?.isActive == true && !unanswered) return
        watcher?.cancel()
        _syncState.value = SyncState.WAITING
        askedAt = SystemClock.elapsedRealtime()
        watcher = scope.launch(Dispatchers.IO) {
            // Android only asks while Vyoma is on screen, and on a cold start the reader got there
            // before Android counted it as such, so it was turned down without asking.
            delay(SETTLE_MS)
            val marker = "Denying clipboard access to ${context.packageName}"
            val process = runCatching {
                // ClipboardService logs through Slog, i.e. the system buffer, the quiet one; warnings
                // and up only. -T 1 replays its last line at once: any line at all proves the log is
                // readable, as Vyoma never writes there.
                ProcessBuilder("logcat", "-b", "system", "-T", "1", "-v", "brief", "*:W").redirectErrorStream(true).start()
            }.getOrElse {
                VLog.w(TAG, "can't read logcat: ${it.message}")
                _syncState.value = SyncState.NEEDS_GRANT
                return@launch
            }
            VLog.i(TAG, "background clipboard sync starting")
            try {
                process.inputStream.bufferedReader().useLines { lines ->
                    for (line in lines) {
                        if (!isActive) break
                        if (_syncState.value != SyncState.ON) {
                            _syncState.value = SyncState.ON
                            VLog.i(TAG, "background clipboard sync on: the system log is readable")
                        }
                        if (marker in line) main.post(::grabFocusAndRead)
                    }
                }
            } finally {
                process.destroy()
                if (watcher == coroutineContext[Job]) _syncState.value = if (backgroundSyncAvailable()) SyncState.WAITING else SyncState.NEEDS_GRANT
                VLog.i(TAG, "background clipboard sync stopped")
            }
        }
    }

    private var askedAt = 0L

    private var overlay: View? = null
    private var focused = false

    /** A 1px, invisible, focusable overlay: focus is what the clipboard check wants. */
    @SuppressLint("ClickableViewAccessibility")
    private fun grabFocusAndRead() {
        if (overlay != null || !canDrawOverlays()) return
        val wm = context.getSystemService(WindowManager::class.java)
        focused = false
        val view = object : View(context) {
            override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
                super.onWindowFocusChanged(hasWindowFocus)
                if (hasWindowFocus) {
                    focused = true
                    if (readNow() == null) VLog.i(TAG, "copy seen, focus came, nothing new to send")
                    // Not from inside this callback: the framework is still dispatching the focus
                    // change and touches the view right after we return, which crashed the app.
                    main.post { removeOverlay(wm) }
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
            main.postDelayed({
                if (overlay === view && !focused) VLog.w(TAG, "copy seen, but Android gave the reading window no focus")
                removeOverlay(wm)
            }, 600)
        }.onFailure { VLog.w(TAG, "copy seen, but the reading window couldn't open: ${it.message}") }
    }

    private fun removeOverlay(wm: WindowManager) {
        val v = overlay ?: return
        overlay = null
        runCatching { wm.removeView(v) }
    }

    private companion object {
        const val TAG = "Clipboard"
        private const val SETTLE_MS = 1_000L
        private const val ASK_AGAIN_AFTER_MS = 65_000L
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
            val t = (o["text"] as? JsonPrimitive)?.content?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
            ClipEntry(t, o["timestamp"]?.jsonPrimitive?.long ?: 0, (o["origin"] as? JsonPrimitive)?.content)
        }.distinctBy { sameText(it.text) } // newest first, so the latest copy of a repeat wins
    }
}

/** Repeats differ only in whitespace when they've been through a terminal or the PC (wrapped lines, CRLF). */
private fun sameText(text: String) = text.trim().replace(WHITESPACE, " ")

private val WHITESPACE = Regex("""\s+""")
