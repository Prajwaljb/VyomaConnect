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
import kotlinx.coroutines.flow.first
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
import kotlinx.coroutines.sync.withLock

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

    /** Done once the saved history is in memory: nothing is written before, so a copy picked up at start can't overwrite it. */
    private val loaded = kotlinx.coroutines.CompletableDeferred<Unit>()
    /** One write at a time, each of the history as it is then, so an older one can't land last. */
    private val writing = kotlinx.coroutines.sync.Mutex()

    init {
        scope.launch(Dispatchers.IO) {
            val saved = if (clearedEarly) emptyList() else runCatching { decode(historyFile.readText()) }.getOrNull().orEmpty()
            // Copies recorded while it loaded stay on top of what was saved.
            _history.update { early ->
                val keys = early.map { sameText(it.text) }.toSet()
                (early + saved.filterNot { sameText(it.text) in keys }).take(HISTORY_SIZE)
            }
            if (_current.value == null) _current.value = _history.value.firstOrNull()
            loaded.complete(Unit)
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

    @Volatile private var clearedEarly = false

    fun clearHistory() {
        if (!loaded.isCompleted) clearedEarly = true
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
        scope.launch(Dispatchers.IO) {
            loaded.await()
            writing.withLock {
                runCatching {
                    val tmp = File(historyFile.path + ".tmp")
                    tmp.writeText(encode(_history.value))
                    tmp.renameTo(historyFile)
                }
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

    /** When (elapsedRealtime) an Ask again tapped during Android's minute of refusals runs; 0 = none waiting. */
    private val _askAt = MutableStateFlow(0L)
    val askAt: StateFlow<Long> = _askAt.asStateFlow()

    /** Vyoma's window has focus: Android asks only an app in front, and its question takes the focus while it shows. */
    private val inFront = MutableStateFlow(false)
    @Volatile private var lastTouch = 0L
    /** When the current reader started (0 = none), and whether it has read a line (Android said yes). */
    @Volatile private var readerAt = 0L
    @Volatile private var readable = false
    /** Android turned a reader down (no line came), and until when it turns new ones down without asking. */
    @Volatile private var refused = false
    @Volatile private var quietUntil = 0L
    private var askLater: Job? = null

    /** A touch on Vyoma's screen: its question waits for the finger to rest, so a tap meant for Vyoma can't answer it. */
    fun touched() { lastTouch = SystemClock.elapsedRealtime() }

    /** Vyoma's window gained or lost focus. */
    fun front(focus: Boolean) {
        inFront.value = focus
        if (focus) checkRefused()
    }

    /**
     * Follows the system log. Android 13+ asks about each new logcat reader whether it may see
     * other apps' lines, only while the app is in front, and turns one down without asking
     * otherwise; either "no" stands for about a minute, so readers in that time are turned down
     * silently. So a reader starts only with Vyoma in front and the finger at rest (the question
     * once popped up under a tap meant for Vyoma, which answered it), opening Vyoma asks once per
     * run, and [again] (Ask again) inside that minute waits it out, shown as [askAt].
     */
    fun startBackgroundSync(again: Boolean = false) {
        if (!backgroundSyncAvailable()) {
            _syncState.value = SyncState.NEEDS_GRANT
            return
        }
        if (_syncState.value == SyncState.ON || (!again && (watcher?.isActive == true || refused))) return
        val now = SystemClock.elapsedRealtime()
        if (now < quietUntil) {
            if (again && askLater?.isActive != true) {
                _askAt.value = quietUntil
                askLater = scope.launch {
                    delay(quietUntil - now)
                    _askAt.value = 0L
                    startBackgroundSync(again = true)
                }
            }
            return
        }
        askLater?.cancel()
        _askAt.value = 0L
        watcher?.cancel()
        readerAt = 0L
        _syncState.value = SyncState.WAITING
        watcher = scope.launch(Dispatchers.IO) {
            // In front, settled (on a cold start Android counted Vyoma as in front a moment late), finger at rest.
            delay(SETTLE_MS)
            while (true) {
                inFront.first { it }
                val wait = QUIET_TOUCH_MS - (SystemClock.elapsedRealtime() - lastTouch)
                if (wait <= 0 && inFront.value) break
                delay(wait.coerceAtLeast(100))
            }
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
            readable = false
            readerAt = SystemClock.elapsedRealtime()
            VLog.i(TAG, "background clipboard sync starting")
            checkRefused()
            try {
                process.inputStream.bufferedReader().useLines { lines ->
                    for (line in lines) {
                        if (!isActive) break
                        readable = true
                        if (_syncState.value != SyncState.ON) {
                            _syncState.value = SyncState.ON
                            VLog.i(TAG, "background clipboard sync on: the system log is readable")
                        }
                        if (marker in line) main.post(::grabFocusAndRead)
                    }
                }
            } finally {
                process.destroy()
                if (watcher == coroutineContext[Job]) {
                    readerAt = 0L
                    _syncState.value = if (backgroundSyncAvailable()) SyncState.WAITING else SyncState.NEEDS_GRANT
                }
                VLog.i(TAG, "background clipboard sync stopped")
            }
        }
    }

    /**
     * A reader that has had Vyoma in front a while (asked and answered, or turned down without
     * asking) and still read nothing was turned down.
     */
    private fun checkRefused() {
        if (readerAt == 0L || readable) return
        scope.launch {
            delay(REFUSAL_SEEN_MS)
            val now = SystemClock.elapsedRealtime()
            if (readerAt == 0L || readable || !inFront.value || now - readerAt < REFUSAL_SEEN_MS || _syncState.value != SyncState.WAITING) return@launch
            if (!refused || now >= quietUntil) {
                if (now >= quietUntil) quietUntil = now + REFUSED_FOR_MS
                if (!refused) VLog.i(TAG, "Android turned log access down")
                refused = true
            }
        }
    }

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
        /** How long the finger must rest before Android's question may pop up. */
        private const val QUIET_TOUCH_MS = 1_200L
        /** In front this long with nothing read: turned down. */
        private const val REFUSAL_SEEN_MS = 1_500L
        /** Android's memory of a "no" (LogcatManagerService: 60 s), plus a little. */
        private const val REFUSED_FOR_MS = 62_000L
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
