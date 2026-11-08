package dev.vyoma.connect.mirror

import android.content.Context
import android.content.Intent
import android.graphics.Point
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Handler
import android.os.Looper
import android.util.Size
import android.view.Display
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.service.LinkService
import dev.vyoma.connect.webcam.H264Encoder
import dev.vyoma.connect.webcam.LiveStream
import dev.vyoma.connect.webcam.companionLink
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * The phone's screen in a window on the PC (Vyoma's companion, desktop/companion/src/mirror.rs),
 * driven from there with the mouse and keyboard through [MirrorControl]. Android asks the user each
 * time (MediaProjection); the link service then holds the mediaProjection foreground type and hands
 * the grant to [begin]. One virtual display mirrors the screen for the whole grant (Android allows
 * only one per grant); each link to the PC gets a fresh encoder and stream, and so does a turn of
 * the phone (the picture's shape changes). Closing the PC's window ends it, as Stop does.
 */
object PhoneMirror {
    data class Status(
        val on: Boolean = false,
        /** The PC's name while its companion is linked. */
        val pc: String? = null,
        /** The PC's window shows the phone. */
        val open: Boolean = false,
        val error: String? = null,
        /** When sharing began (the Live Update's timer). */
        val since: Long = 0,
    )

    private const val TAG = "Mirror"
    private const val FPS = 60
    private const val BITRATE = 8_000_000
    /** The picture's long side at most (the PC scales the window to its screen). */
    private const val MAX_SIDE = 1920
    /** ~0.5 s at the bitrate: more waiting is dropped (skips to the next keyframe), not delayed. */
    private const val BACKLOG = 512 * 1024

    private const val RATE = 48_000
    /** 100 ms of stereo sound: more waiting is dropped (the PC plays a touch slower), not delayed. */
    private const val SOUND_BACKLOG = RATE * 4 / 10

    /** The phone's sound plays on the PC too; the running mirror follows it. */
    val sound = MutableStateFlow(true)

    /** The phone's sound is going to the PC right now: its speaker is silenced and the volume keys work the PC. */
    @Volatile var soundOnPc = false
        private set
    /** The companion the mirror is streaming to (for copies and volume keys). */
    @Volatile private var current: CompanionPlugin? = null

    private val _status = MutableStateFlow(Status())
    val status: StateFlow<Status> = _status.asStateFlow()
    private var job: Job? = null
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null

    /** The screen-capture prompt; its result goes to [LinkService.mirror]. */
    fun consentIntent(context: Context): Intent {
        val mpm = context.getSystemService(MediaProjectionManager::class.java)
        // The whole screen, never one app: the PC's clicks land on the screen as a whole.
        return if (android.os.Build.VERSION.SDK_INT >= 34) {
            mpm.createScreenCaptureIntent(android.media.projection.MediaProjectionConfig.createConfigForDefaultDisplay())
        } else mpm.createScreenCaptureIntent()
    }

    /** From the link service, once it's in the foreground as mediaProjection (Android's order). */
    fun begin(service: Context, code: Int, data: Intent) {
        val app = service.applicationContext as android.app.Application
        stop(app, keepForeground = true)
        Speaker(app).restore()
        val p = runCatching { app.getSystemService(MediaProjectionManager::class.java).getMediaProjection(code, data) }
            .onFailure { VLog.w(TAG, "no projection: ${it.message}") }.getOrNull()
        if (p == null) {
            _status.value = Status(error = "Android didn't allow the screen to be shared")
            LinkService.projecting(app, false)
            return
        }
        projection = p
        // Android ends it too: the user's "Stop sharing" in the status bar, another app's capture.
        p.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() { if (projection === p) stop(app) }
        }, Handler(Looper.getMainLooper()))
        val first = pictureSize(realSize(app))
        display = runCatching {
            p.createVirtualDisplay(
                "Vyoma mirror", first.width, first.height, app.resources.displayMetrics.densityDpi,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, null, null, null,
            )
        }.onFailure { VLog.w(TAG, "no virtual display: ${it.message}") }.getOrNull()
        if (display == null) {
            _status.value = Status(error = "Couldn't capture the screen")
            stop(app)
            return
        }
        _status.value = Status(on = true, since = System.currentTimeMillis())
        job = app.container.appScope.launch { follow(app) }
    }

    fun stop(context: Context) = stop(context, keepForeground = false)

    private fun stop(context: Context, keepForeground: Boolean) {
        val wasOn = _status.value.on || projection != null
        job?.cancel()
        job = null
        display?.release()
        display = null
        projection?.let { projection = null; runCatching { it.stop() } }
        if (wasOn && !keepForeground) LinkService.projecting(context.applicationContext, false)
        _status.value = _status.value.copy(on = false, open = false)
    }

    private suspend fun follow(app: android.app.Application) {
        val hub = app.container.hub.filterNotNull().first()
        companionLink(hub).collectLatest { c ->
            if (c == null) {
                _status.value = _status.value.copy(pc = null, open = false)
                return@collectLatest
            }
            val (plugin, pc) = c
            _status.value = _status.value.copy(pc = pc, error = null)
            while (true) {
                plugin.resetMirror()
                try {
                    session(app, plugin)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    VLog.w(TAG, "stream ended: ${e.message}")
                }
                // The stream can break a moment before the PC's word on why arrives: wait for it.
                val said = withTimeoutOrNull(1_500) { plugin.mirror.first { it.closed || it.error != null } } ?: plugin.mirror.value
                if (said.error != null) {
                    _status.value = _status.value.copy(open = false, error = said.error)
                    stop(app)
                    return@collectLatest
                }
                // The PC's window was closed: that's the user stopping it.
                if (said.closed) {
                    VLog.i(TAG, "the PC's window was closed")
                    stop(app)
                    return@collectLatest
                }
                _status.value = _status.value.copy(open = false)
                delay(2_000)
            }
        }
    }

    /** One stream to the PC, until it ends. */
    private suspend fun session(app: Context, plugin: CompanionPlugin): Unit = coroutineScope {
        val vd = display ?: return@coroutineScope
        val video = LiveStream(BACKLOG)
        current = plugin
        // Out of power save while it streams: a dozing radio held the PC's acks back, and the picture
        // stalled in bursts of 250 ms to 1.5 s (as the PC screen view, which takes the same lock).
        val wifi = (app.getSystemService(Context.WIFI_SERVICE) as android.net.wifi.WifiManager)
            .createWifiLock(android.net.wifi.WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "vyoma:mirror")
            .apply { setReferenceCounted(false); acquire() }
        var size = pictureSize(realSize(app))
        var encoder = H264Encoder(size.width, size.height, BITRATE, FPS, video, "mirror-encoder")
        vd.resize(encoder.width, encoder.height, app.resources.displayMetrics.densityDpi)
        vd.surface = encoder.surface
        try {
            launch { plugin.mirror.collect { s -> _status.value = _status.value.copy(open = s.open) } }
            launch {
                val control = MirrorControl(app)
                plugin.mirrorInput.collect { control.perform(it) }
            }
            // The phone turned: the picture changes shape, so a new encoder (a new SPS in the same
            // stream) at the new size.
            launch {
                rotations(app).drop(1).collect { real ->
                    val next = pictureSize(real)
                    if (next == size) return@collect
                    VLog.i(TAG, "turned: ${next.width}x${next.height}")
                    vd.surface = null
                    encoder.close()
                    size = next
                    encoder = H264Encoder(size.width, size.height, BITRATE, FPS, video, "mirror-encoder")
                    vd.resize(encoder.width, encoder.height, app.resources.displayMetrics.densityDpi)
                    vd.surface = encoder.surface
                }
            }
            launch { sound.collectLatest { on -> if (on) playOnPc(app, plugin) } }
            VLog.i(TAG, "mirroring ${encoder.width}x${encoder.height}")
            plugin.sendMirror(encoder.width, encoder.height, video)
        } finally {
            wifi.release()
            if (current === plugin) current = null
            // Stop may have released the display already.
            runCatching { vd.surface = null }
            encoder.close()
            video.close()
            coroutineContext[Job]?.children?.forEach { it.cancel() }
        }
    }

    /**
     * What the phone plays (media, games, anything that allows capture) to the PC, through the same
     * screen-sharing grant, until cancelled. Apps that opt out of capture, and calls, stay silent;
     * the phone keeps playing too (Android doesn't let an app take its sound away).
     */
    @android.annotation.SuppressLint("MissingPermission")
    private suspend fun playOnPc(app: Context, plugin: CompanionPlugin) {
        val p = projection ?: return
        if (app.checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) != android.content.pm.PackageManager.PERMISSION_GRANTED) return
        val config = android.media.AudioPlaybackCaptureConfiguration.Builder(p)
            .addMatchingUsage(android.media.AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(android.media.AudioAttributes.USAGE_GAME)
            .addMatchingUsage(android.media.AudioAttributes.USAGE_UNKNOWN)
            .build()
        val format = android.media.AudioFormat.Builder()
            .setEncoding(android.media.AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(RATE)
            .setChannelMask(android.media.AudioFormat.CHANNEL_IN_STEREO)
            .build()
        val record = runCatching {
            android.media.AudioRecord.Builder().setAudioPlaybackCaptureConfig(config).setAudioFormat(format)
                .setBufferSizeInBytes(RATE * 4 / 5).build()
        }.onFailure { VLog.w(TAG, "no sound capture: ${it.message}") }.getOrNull() ?: return
        val pcm = LiveStream(SOUND_BACKLOG)
        val speaker = Speaker(app)
        val running = java.util.concurrent.atomic.AtomicBoolean(true)
        val reader = Thread({
            val chunk = ByteArray(RATE / 100 * 4) // 10 ms
            while (running.get()) {
                val n = record.read(chunk, 0, chunk.size)
                if (n > 0) pcm.offer(chunk.copyOf(n)) else if (n < 0) break
            }
        }, "mirror-sound")
        try {
            record.startRecording()
            reader.start()
            speaker.silence()
            soundOnPc = true
            plugin.sendMirrorAudio(RATE, 2, pcm)
        } finally {
            soundOnPc = false
            speaker.restore()
            running.set(false)
            pcm.close()
            reader.join(200)
            runCatching { record.stop() }
            record.release()
        }
    }

    /**
     * The phone's media volume, down to nothing while its sound plays on the PC (the capture is taken
     * before the volume, so the PC still gets it all) and back after. The level to restore is saved,
     * so a crash meanwhile doesn't leave the phone silent: [begin] puts it back.
     */
    /** The app died while the PC played its sound: the phone was left silent. */
    fun restoreVolume(context: Context) = Speaker(context).restore()

    private class Speaker(context: Context) {
        private val audio = context.getSystemService(android.media.AudioManager::class.java)
        private val prefs = context.getSharedPreferences("mirror", Context.MODE_PRIVATE)

        fun silence() {
            val was = audio.getStreamVolume(android.media.AudioManager.STREAM_MUSIC)
            if (!prefs.contains(RESTORE)) prefs.edit().putInt(RESTORE, was).apply()
            runCatching { audio.setStreamVolume(android.media.AudioManager.STREAM_MUSIC, 0, 0) }
        }

        fun restore() {
            val was = prefs.getInt(RESTORE, -1)
            if (was < 0) return
            runCatching { audio.setStreamVolume(android.media.AudioManager.STREAM_MUSIC, was, 0) }
            prefs.edit().remove(RESTORE).apply()
        }

        companion object { const val RESTORE = "restoreVolume" }
    }

    /** From [MirrorAccessibility]: a volume key while the sound is on the PC. */
    fun volumeKey(context: Context, up: Boolean) {
        val plugin = current ?: return
        (context.applicationContext as android.app.Application).container.appScope.launch { plugin.mirrorVolume(up) }
    }

    /** From [MirrorControl]: text copied in the mirror, onto the PC's clipboard. */
    fun copyToPc(context: Context, text: String) {
        val plugin = current ?: return
        (context.applicationContext as android.app.Application).container.appScope.launch { plugin.mirrorCopy(text) }
    }

    @Suppress("DEPRECATION")
    private fun realSize(context: Context): Size {
        val d = context.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY)
        val p = Point()
        d.getRealSize(p)
        return Size(p.x, p.y)
    }

    /** The screen's size whenever it changes (a turn), starting with the current one. */
    private fun rotations(context: Context) = callbackFlow {
        val dm = context.getSystemService(DisplayManager::class.java)
        val listener = object : DisplayManager.DisplayListener {
            override fun onDisplayChanged(id: Int) { if (id == Display.DEFAULT_DISPLAY) trySend(realSize(context)) }
            override fun onDisplayAdded(id: Int) {}
            override fun onDisplayRemoved(id: Int) {}
        }
        trySend(realSize(context))
        dm.registerDisplayListener(listener, Handler(Looper.getMainLooper()))
        awaitClose { dm.unregisterDisplayListener(listener) }
    }.distinctUntilChanged()

    /** The screen scaled so its long side is at most [MAX_SIDE], both sides a multiple of 8 (encoders like that). */
    private fun pictureSize(real: Size): Size {
        val scale = minOf(1f, MAX_SIDE.toFloat() / maxOf(real.width, real.height))
        fun even(v: Int) = maxOf(8, ((v * scale) / 8f).toInt() * 8)
        return Size(even(real.width), even(real.height))
    }

    /** Where a picture point (0..1) is on the phone's screen right now, in pixels. */
    internal fun screenPoint(context: Context, x: Float, y: Float): Pair<Float, Float> {
        val s = realSize(context)
        return (x * s.width).coerceIn(0f, s.width - 1f) to (y * s.height).coerceIn(0f, s.height - 1f)
    }
}
