package dev.vyoma.connect.webcam

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.SessionConfiguration
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaRecorder
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.util.Range
import android.util.Size
import android.view.OrientationEventListener
import android.view.Surface
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.service.LinkService
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelChildren
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * The phone as the PC's webcam and microphone (with Vyoma's companion on the PC, see
 * desktop/companion/src/webcam.rs). The camera feeds the hardware H.264 encoder through its input
 * surface; the encoded stream and raw 48 kHz mic audio go to the PC as two live payloads. It runs
 * in the app's scope (the screen can close, the phone can sleep) under the link service, which
 * holds camera and microphone foreground types meanwhile, and follows the companion link: a relink
 * restarts the stream.
 *
 * Frames come out in the sensor's orientation (landscape); how the phone is held decides the turn
 * the PC applies ([rotation]), so turning the phone needs no new stream.
 */
object PhoneWebcam {
    data class Options(val front: Boolean = true, val mic: Boolean = true, val hd: Boolean = false)

    data class Status(
        val on: Boolean = false,
        val options: Options = Options(),
        /** The PC is showing the camera / has the mic. */
        val camera: Boolean = false,
        val mic: Boolean = false,
        val pc: String? = null,
        val error: String? = null,
        /** The camera in use: how far it zooms (below 1 is its ultra-wide) and whether it has a torch. */
        val zoomRange: ClosedFloatingPointRange<Float> = 1f..1f,
        val torchable: Boolean = false,
        /** The picture's shape as the preview shows it: encoded size (landscape). */
        val size: Size? = null,
        /** When the camera turned on (the Live Update's timer). */
        val since: Long = 0,
    )

    private const val TAG = "Webcam"
    private const val FPS = 30
    private const val RATE = 48_000
    /** Most encoded video that may wait unsent (~0.5 s at 4 Mbit/s): more is dropped, not delayed. */
    private const val VIDEO_BACKLOG = 256 * 1024
    /** 100 ms of mic audio. */
    private const val MIC_BACKLOG = RATE * 2 / 10
    /** Past this a phone's zoom is digital mush. */
    private const val MAX_ZOOM = 8f
    private const val CAMERA_RETRY_MS = 2_000L

    private val _status = MutableStateFlow(Status())
    val status: StateFlow<Status> = _status.asStateFlow()
    private var job: Job? = null

    /** Which camera and whether the mic is on: the running stream follows these without restarting. */
    private val front = MutableStateFlow(true)
    private val micOn = MutableStateFlow(true)
    /** The screen's preview (null: closed); zoom and torch for the camera in use. */
    private val preview = MutableStateFlow<Surface?>(null)
    val zoom = MutableStateFlow(1f)
    val torch = MutableStateFlow(false)
    @Volatile private var capturing: CameraCaptureSession? = null

    fun attachPreview(surface: Surface) { preview.value = surface }

    /**
     * The preview's surface is going: stop the camera drawing into it right now (from the surface
     * callback, before it's freed), then the stream carries on without it.
     */
    fun detachPreview() {
        if (preview.value == null) return
        runCatching { capturing?.stopRepeating() }
        runCatching { capturing?.abortCaptures() }
        preview.value = null
    }

    fun start(context: Context, options: Options) {
        val app = context.applicationContext as android.app.Application
        stop(app)
        front.value = options.front
        micOn.value = options.mic
        _status.value = Status(on = true, options = options, since = System.currentTimeMillis())
        LinkService.mediaInUse(app, camera = true, mic = options.mic)
        job = app.container.appScope.launch { follow(app, options.hd) }
    }

    fun stop(context: Context) {
        job?.cancel()
        job = null
        if (_status.value.on) LinkService.mediaInUse(context.applicationContext, camera = false, mic = false)
        _status.value = _status.value.copy(on = false, camera = false, mic = false)
    }

    /**
     * Switching cameras or the mic happens inside the running stream: the PC's camera device stays
     * open, so the app watching it never sees it go (a restart made calls drop the camera, user
     * 2026-11-04). Only a new quality restarts, as the picture's size changes.
     */
    fun change(context: Context, options: Options) {
        val was = _status.value
        if (was.on && options.hd != was.options.hd) return start(context, options)
        _status.value = was.copy(options = options, error = null)
        if (!was.on) return
        front.value = options.front
        if (micOn.value != options.mic) {
            micOn.value = options.mic
            LinkService.mediaInUse(context.applicationContext, camera = true, mic = options.mic)
        }
    }

    private suspend fun follow(app: Context, hd: Boolean) {
        val hub = (app as android.app.Application).container.hub.filterNotNull().first()
        companionLink(hub).collectLatest { c ->
            if (c == null) {
                _status.value = _status.value.copy(camera = false, mic = false, pc = null, error = null)
                return@collectLatest
            }
            val (plugin, pc) = c
            _status.value = _status.value.copy(pc = pc)
            while (true) {
                plugin.resetWebcam()
                try {
                    session(app, plugin, hd)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    VLog.w(TAG, "stream ended: ${e.message}")
                }
                // The PC said why it can't (no camera device set up): stay off, say so.
                plugin.webcam.value.error?.let { err ->
                    _status.value = _status.value.copy(camera = false, mic = false, error = err)
                    stop(app)
                    return@collectLatest
                }
                _status.value = _status.value.copy(camera = false, mic = false)
                delay(2_000)
            }
        }
    }

    private fun cameraFor(cameras: CameraManager, wantFront: Boolean): String =
        cameras.cameraIdList.firstOrNull {
            cameras.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) ==
                (if (wantFront) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK)
        } ?: cameras.cameraIdList.first()

    /** The clockwise turn that makes [id]'s frames upright with the phone held at [held]. */
    private fun upright(cameras: CameraManager, id: String, held: Int): Int {
        val chars = cameras.getCameraCharacteristics(id)
        val sensor = chars.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
        val isFront = chars.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_FRONT
        return if (isFront) (sensor - held + 360) % 360 else (sensor + held) % 360
    }

    /**
     * One stream to the PC until it hangs up or this is cancelled. One encoder and one payload for
     * its whole life; the camera feeding the encoder ([front]) and the mic ([micOn]) come and go
     * inside it.
     */
    @OptIn(ExperimentalCoroutinesApi::class)
    private suspend fun session(app: Context, plugin: CompanionPlugin, hd: Boolean) = coroutineScope {
        val cameras = app.getSystemService(CameraManager::class.java)
        // A size both cameras give an encoder, so switching never changes it.
        val want = if (hd) Size(1920, 1080) else Size(1280, 720)
        val size = listOf(true, false).map { pickSize(cameras.getCameraCharacteristics(cameraFor(cameras, it)), want) }.minBy { it.width }
        val video = LiveStream(VIDEO_BACKLOG)
        val encoder = H264Encoder(size.width, size.height, if (hd) 8_000_000 else 4_000_000, FPS, video, "webcam-encoder")
        val turns = Turns(app)
        try {
            launch {
                plugin.webcam.collect { s -> _status.value = _status.value.copy(camera = s.camera, mic = s.mic) }
            }
            _status.value = _status.value.copy(size = size)
            // The camera in use: closed and the other opened on a switch, into the same encoder. The
            // preview (when the screen is open) joins the session; zoom and torch only change the request.
            launch {
                front.collectLatest { f ->
                    val id = cameraFor(cameras, f)
                    val chars = cameras.getCameraCharacteristics(id)
                    val range = chars.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)?.let { it.lower..it.upper.coerceAtMost(MAX_ZOOM) } ?: 1f..1f
                    val torchable = chars.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true
                    zoom.value = 1f
                    torch.value = false
                    _status.value = _status.value.copy(zoomRange = range, torchable = torchable)
                    // Another app can take the camera at any time. The stream stays up (the PC's camera
                    // device with it, holding the last picture) and this one comes back when it's free.
                    while (true) {
                        val lost = CompletableDeferred<Unit>()
                        val camera = try {
                            open(cameras, id, encoder.handler) { lost.complete(Unit) }
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            VLog.w(TAG, "can't open the camera: ${e.message}")
                            delay(CAMERA_RETRY_MS)
                            continue
                        }
                        try {
                            coroutineScope {
                                launch {
                                    lost.await()
                                    error("another app took the camera")
                                }
                                VLog.i(TAG, "camera ${if (f) "front" else "back"} ${size.width}x${size.height}")
                                preview.collectLatest { shown ->
                                    val targets = listOfNotNull(encoder.surface, shown).toMutableList()
                                    // A camera that can't add the preview still streams without it.
                                    val session = runCatching { session(camera, targets, encoder.handler) }.getOrElse { e ->
                                        if (e is CancellationException || shown == null) throw e
                                        VLog.w(TAG, "no preview: ${e.message}")
                                        targets.retainAll { it !== shown }
                                        session(camera, targets, encoder.handler)
                                    }
                                    capturing = session
                                    try {
                                        val builder = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                                            targets.forEach(::addTarget)
                                            set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(FPS, FPS))
                                            set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
                                        }
                                        // A dragged zoom slider changes many times a frame; the camera takes one
                                        // new request per frame at most (the latest), or it queues and lags.
                                        combine(zoom, torch) { z, t -> z to t }.collect { (z, t) ->
                                            builder.set(CaptureRequest.CONTROL_ZOOM_RATIO, z.coerceIn(range))
                                            if (torchable) builder.set(CaptureRequest.FLASH_MODE, if (t) CaptureRequest.FLASH_MODE_TORCH else CaptureRequest.FLASH_MODE_OFF)
                                            runCatching { session.setRepeatingRequest(builder.build(), null, encoder.handler) }
                                            delay(1000L / FPS)
                                        }
                                    } finally {
                                        capturing = null
                                        runCatching { session.close() }
                                    }
                                }
                            }
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            VLog.w(TAG, "camera lost: ${e.message}")
                        } finally {
                            camera.close()
                        }
                        delay(CAMERA_RETRY_MS)
                    }
                }
            }
            // The turn the PC applies follows both the phone and the camera; only real changes go out.
            launch {
                combine(turns.held, front) { held, f -> upright(cameras, cameraFor(cameras, f), held) }
                    .distinctUntilChanged()
                    .collect { r ->
                        plugin.turnWebcam(r)
                        encoder.sync()
                    }
            }
            launch {
                micOn.collectLatest { on ->
                    if (!on) return@collectLatest
                    val pcm = LiveStream(MIC_BACKLOG)
                    val mic = Mic(app, pcm)
                    try {
                        mic.start()
                        plugin.sendMic(RATE, 1, pcm)
                    } finally {
                        mic.close()
                        pcm.close()
                    }
                }
            }
            plugin.sendWebcam(encoder.width, encoder.height, upright(cameras, cameraFor(cameras, front.value), turns.held.value), video)
        } finally {
            turns.close()
            video.close()
            this.coroutineContext.cancelChildren()
            encoder.close()
        }
    }

    /** The requested size if the camera gives it to an encoder, else its largest 16:9 below it. */
    private fun pickSize(chars: CameraCharacteristics, want: Size): Size {
        val sizes = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)?.getOutputSizes(MediaCodec::class.java).orEmpty()
        if (sizes.any { it == want }) return want
        return sizes.filter { it.width * 9 == it.height * 16 && it.width <= want.width }.maxByOrNull { it.width } ?: Size(1280, 720)
    }

    @SuppressLint("MissingPermission")
    private suspend fun open(cameras: CameraManager, id: String, handler: Handler, onLost: () -> Unit): CameraDevice = suspendCancellableCoroutine { k ->
        cameras.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(camera: CameraDevice) {
                if (k.isActive) k.resume(camera) else camera.close()
            }
            override fun onDisconnected(camera: CameraDevice) {
                camera.close()
                if (k.isActive) k.resumeWithException(IllegalStateException("another app took the camera")) else onLost()
            }
            override fun onError(camera: CameraDevice, error: Int) {
                camera.close()
                if (k.isActive) k.resumeWithException(IllegalStateException("camera error $error")) else onLost()
            }
        }, handler)
    }

    private suspend fun session(camera: CameraDevice, targets: List<Surface>, handler: Handler): CameraCaptureSession =
        suspendCancellableCoroutine { k ->
            camera.createCaptureSession(
                SessionConfiguration(
                    SessionConfiguration.SESSION_REGULAR, targets.map(::OutputConfiguration), { handler.post(it) },
                    object : CameraCaptureSession.StateCallback() {
                        override fun onConfigured(session: CameraCaptureSession) { if (k.isActive) k.resume(session) else session.close() }
                        override fun onConfigureFailed(session: CameraCaptureSession) {
                            if (k.isActive) k.resumeWithException(IllegalStateException("camera couldn't start"))
                        }
                    },
                ),
            )
        }

    /** The mic, raw: 48 kHz mono 16-bit, with the phone's echo cancelling and noise suppression. */
    private class Mic(context: Context, private val out: LiveStream) {
        private val record: AudioRecord?
        @Volatile private var running = false
        private var thread: Thread? = null

        init {
            val granted = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
            val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            @SuppressLint("MissingPermission")
            val r = if (granted) runCatching {
                AudioRecord(MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(min, RATE / 5))
            }.getOrNull() else null
            record = r
        }

        fun start() {
            val r = record ?: return
            running = true
            r.startRecording()
            thread = Thread({
                val chunk = ByteArray(RATE / 100 * 2) // 10 ms
                while (running) {
                    val n = r.read(chunk, 0, chunk.size)
                    if (n < 0) { // audioserver died: this would answer at once, for ever
                        VLog.w(TAG, "mic stopped: $n")
                        break
                    }
                    if (n > 0) out.offer(chunk.copyOf(n)) // full: the PC is behind, this 10 ms is skipped
                }
            }, "webcam-mic").apply { priority = Thread.MAX_PRIORITY; start() }
        }

        fun close() {
            running = false
            thread?.join(200)
            runCatching { record?.stop() }
            record?.release()
        }
    }

    /** How the phone is held, snapped to 90° steps with some give (no flicker at 45°); flat keeps the last. */
    private class Turns(context: Context) {
        val held = MutableStateFlow(0)
        private var last = -1
        private val listener = object : OrientationEventListener(context) {
            override fun onOrientationChanged(o: Int) {
                if (o == ORIENTATION_UNKNOWN) return
                val snapped = ((o + 45) / 90 * 90) % 360
                if (last >= 0 && snapped != last) {
                    // Only past 30° into the new quarter.
                    val off = ((o - snapped + 540) % 360) - 180
                    if (kotlin.math.abs(off) > 30) return
                }
                if (snapped != last) {
                    last = snapped
                    held.value = snapped
                }
            }
        }.also { if (it.canDetectOrientation()) it.enable() }

        fun close() = listener.disable()
    }
}

/**
 * A live stream's bytes waiting to be sent: [offer] adds a piece unless [maxBytes] already wait
 * (then it's refused, and the caller drops it), [read] blocks until there's some or it's closed.
 */
class LiveStream(private val maxBytes: Int) : java.io.InputStream() {
    private val lock = Object()
    private val queue = ArrayDeque<ByteArray>()
    private var queued = 0
    private var current: ByteArray? = null
    private var pos = 0
    private var closed = false

    /** How full it is, 0..1: what the encoder's bitrate follows. */
    val fill: Float get() = synchronized(lock) { queued.toFloat() / maxBytes }

    fun offer(bytes: ByteArray): Boolean = synchronized(lock) {
        if (closed || queued + bytes.size > maxBytes) return false
        queue.addLast(bytes)
        queued += bytes.size
        lock.notifyAll()
        true
    }

    override fun read(): Int {
        val one = ByteArray(1)
        return if (read(one, 0, 1) < 0) -1 else one[0].toInt() and 0xFF
    }

    override fun read(b: ByteArray, off: Int, len: Int): Int = synchronized(lock) {
        while (current == null) {
            if (queue.isNotEmpty()) {
                current = queue.removeFirst().also { queued -= it.size }
                pos = 0
            } else if (closed) return -1
            else lock.wait()
        }
        val c = current!!
        val n = minOf(len, c.size - pos)
        System.arraycopy(c, pos, b, off, n)
        pos += n
        if (pos == c.size) current = null
        n
    }

    override fun close() = synchronized(lock) {
        closed = true
        lock.notifyAll()
    }
}
