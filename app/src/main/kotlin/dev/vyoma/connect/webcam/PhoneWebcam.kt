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
    )

    private const val TAG = "Webcam"
    private const val FPS = 30
    private const val RATE = 48_000
    /** Most encoded video that may wait unsent (~0.5 s at 4 Mbit/s): more is dropped, not delayed. */
    private const val VIDEO_BACKLOG = 256 * 1024
    /** 100 ms of mic audio. */
    private const val MIC_BACKLOG = RATE * 2 / 10

    private val _status = MutableStateFlow(Status())
    val status: StateFlow<Status> = _status.asStateFlow()
    private var job: Job? = null

    fun start(context: Context, options: Options) {
        val app = context.applicationContext as android.app.Application
        stop(app)
        _status.value = Status(on = true, options = options)
        LinkService.mediaInUse(app, camera = true, mic = options.mic)
        job = (app as android.app.Application).container.appScope.launch { follow(app, options) }
    }

    fun stop(context: Context) {
        job?.cancel()
        job = null
        if (_status.value.on) LinkService.mediaInUse(context.applicationContext, camera = false, mic = false)
        _status.value = _status.value.copy(on = false, camera = false, mic = false)
    }

    /** Changes cameras / mic / quality: a restart, as the PC sees a new stream either way. */
    fun change(context: Context, options: Options) {
        if (_status.value.on) start(context, options) else _status.value = _status.value.copy(options = options, error = null)
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    private suspend fun follow(app: Context, options: Options) {
        val hub = (app as android.app.Application).container.hub.filterNotNull().first()
        hub.devices.flatMapLatest { all ->
            val each = all.values.map { d ->
                d.state.map { st ->
                    val pc = d.identity.companionOf ?: return@map null
                    if (!st.isReachable) return@map null
                    d.plugin<CompanionPlugin>(CompanionPlugin.ID)?.let { it to (all[pc]?.identity?.deviceName ?: "the PC") }
                }
            }
            if (each.isEmpty()) flowOf(null) else combine(each) { found -> found.firstOrNull { it != null } }
        }.distinctUntilChanged { a, b -> a?.first === b?.first }.collectLatest { c ->
            if (c == null) {
                _status.value = _status.value.copy(camera = false, mic = false, pc = null, error = null)
                return@collectLatest
            }
            val (plugin, pc) = c
            _status.value = _status.value.copy(pc = pc)
            while (true) {
                plugin.resetWebcam()
                try {
                    session(app, plugin, options)
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

    /** One stream to the PC: camera (and mic) open until the PC hangs up or this is cancelled. */
    private suspend fun session(app: Context, plugin: CompanionPlugin, options: Options) = coroutineScope {
        val cameras = app.getSystemService(CameraManager::class.java)
        val id = cameras.cameraIdList.firstOrNull {
            cameras.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) ==
                (if (options.front) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK)
        } ?: cameras.cameraIdList.first()
        val chars = cameras.getCameraCharacteristics(id)
        val front = chars.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_FRONT
        val sensor = chars.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
        val size = pickSize(chars, if (options.hd) Size(1920, 1080) else Size(1280, 720))
        val video = LiveStream(VIDEO_BACKLOG)
        val encoder = Encoder(size, if (options.hd) 8_000_000 else 4_000_000, video)
        val turns = Turns(app) { held -> if (front) (sensor - held + 360) % 360 else (sensor + held) % 360 }
        var camera: CameraDevice? = null
        val pcm = if (options.mic) LiveStream(MIC_BACKLOG) else null
        val mic = pcm?.let { Mic(app, it) }
        try {
            camera = open(cameras, id, encoder.handler)
            capture(camera, encoder.surface, encoder.handler)
            launch {
                plugin.webcam.collect { s -> _status.value = _status.value.copy(camera = s.camera, mic = s.mic) }
            }
            launch {
                turns.rotation.collect { r ->
                    plugin.turnWebcam(r)
                    encoder.sync()
                }
            }
            pcm?.let { stream ->
                mic!!.start()
                launch { plugin.sendMic(RATE, 1, stream) }
            }
            VLog.i(TAG, "camera ${if (front) "front" else "back"} ${size.width}x${size.height} to the PC")
            plugin.sendWebcam(size.width, size.height, turns.rotation.value, video)
        } finally {
            turns.close()
            mic?.close()
            pcm?.close()
            video.close()
            runCatching { camera?.close() }
            encoder.close()
            this.coroutineContext.cancelChildren()
        }
    }

    /** The requested size if the camera gives it to an encoder, else its largest 16:9 below it. */
    private fun pickSize(chars: CameraCharacteristics, want: Size): Size {
        val sizes = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)?.getOutputSizes(MediaCodec::class.java).orEmpty()
        if (sizes.any { it == want }) return want
        return sizes.filter { it.width * 9 == it.height * 16 && it.width <= want.width }.maxByOrNull { it.width } ?: Size(1280, 720)
    }

    @SuppressLint("MissingPermission")
    private suspend fun open(cameras: CameraManager, id: String, handler: Handler): CameraDevice = suspendCancellableCoroutine { k ->
        cameras.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(camera: CameraDevice) {
                if (k.isActive) k.resume(camera) else camera.close()
            }
            override fun onDisconnected(camera: CameraDevice) {
                camera.close()
                if (k.isActive) k.resumeWithException(IllegalStateException("another app took the camera"))
            }
            override fun onError(camera: CameraDevice, error: Int) {
                camera.close()
                if (k.isActive) k.resumeWithException(IllegalStateException("camera error $error"))
            }
        }, handler)
    }

    private suspend fun capture(camera: CameraDevice, target: Surface, handler: Handler) {
        val session = suspendCancellableCoroutine { k ->
            camera.createCaptureSession(
                SessionConfiguration(
                    SessionConfiguration.SESSION_REGULAR, listOf(OutputConfiguration(target)), { handler.post(it) },
                    object : CameraCaptureSession.StateCallback() {
                        override fun onConfigured(session: CameraCaptureSession) { if (k.isActive) k.resume(session) }
                        override fun onConfigureFailed(session: CameraCaptureSession) {
                            if (k.isActive) k.resumeWithException(IllegalStateException("camera couldn't start"))
                        }
                    },
                ),
            )
        }
        val request = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
            addTarget(target)
            set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(FPS, FPS))
            set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
        }.build()
        session.setRepeatingRequest(request, null, handler)
    }

    /** The hardware H.264 encoder: camera frames in through [surface], Annex-B out to [out]. */
    private class Encoder(size: Size, bitrate: Int, private val out: LiveStream) {
        private val thread = HandlerThread("webcam-encoder").also { it.start() }
        val handler = Handler(thread.looper)
        private val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        val surface: Surface
        private var config: ByteArray? = null
        private var dropping = false

        init {
            val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, size.width, size.height).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
                setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR)
                setInteger(MediaFormat.KEY_FRAME_RATE, FPS)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2)
                setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
                setInteger(MediaFormat.KEY_LATENCY, 1)
                setInteger(MediaFormat.KEY_PRIORITY, 0)
                // A still scene still sends a frame now and then, so the PC's picture never stalls.
                setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            }
            codec.setCallback(object : MediaCodec.Callback() {
                override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {}
                override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
                    val buf = runCatching { codec.getOutputBuffer(index) }.getOrNull() ?: return
                    val bytes = ByteArray(info.size)
                    buf.position(info.offset)
                    buf.get(bytes)
                    runCatching { codec.releaseOutputBuffer(index, false) }
                    if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                        config = bytes
                        return
                    }
                    val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                    if (dropping && !key) return
                    // SPS/PPS before every keyframe: a decoder restarted on the PC picks up from it.
                    val frame = if (key) (config ?: ByteArray(0)) + bytes else bytes
                    if (out.offer(frame)) dropping = false
                    else if (!dropping) {
                        // The link is behind: skip to the next keyframe instead of falling further back.
                        dropping = true
                        sync()
                    }
                }
                override fun onError(codec: MediaCodec, e: MediaCodec.CodecException) { VLog.w(TAG, "encoder: ${e.message}"); out.close() }
                override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) {}
            }, handler)
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            surface = codec.createInputSurface()
            codec.start()
        }

        fun sync() = runCatching { codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) }) }

        fun close() {
            runCatching { codec.stop() }
            runCatching { codec.release() }
            surface.release()
            thread.quitSafely()
        }
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

    /**
     * How the phone is held, snapped to 90° steps (with some give, so it doesn't flicker at 45°),
     * as the clockwise turn [upright] gives for it. Lying flat keeps the last one.
     */
    private class Turns(context: Context, private val upright: (Int) -> Int) {
        val rotation = MutableStateFlow(upright(0))
        private var held = -1
        private val listener = object : OrientationEventListener(context) {
            override fun onOrientationChanged(o: Int) {
                if (o == ORIENTATION_UNKNOWN) return
                val snapped = ((o + 45) / 90 * 90) % 360
                if (held >= 0 && snapped != held) {
                    // Only past 30° into the new quarter.
                    val off = ((o - snapped + 540) % 360) - 180
                    if (kotlin.math.abs(off) > 30) return
                }
                if (snapped != held) {
                    held = snapped
                    rotation.value = upright(snapped)
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
