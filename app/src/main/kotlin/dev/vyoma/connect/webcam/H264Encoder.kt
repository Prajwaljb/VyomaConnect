package dev.vyoma.connect.webcam

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map

/**
 * The reachable companion (its plugin) and its PC's name, or null: what the phone's live streams
 * to the PC (webcam, mirror) follow. A new link gives a new plugin.
 */
@OptIn(ExperimentalCoroutinesApi::class)
internal fun companionLink(hub: DeviceHub): Flow<Pair<CompanionPlugin, String>?> =
    hub.devices.flatMapLatest { all ->
        val each = all.values.map { d ->
            d.state.map { st ->
                val pc = d.identity.companionOf ?: return@map null
                if (!st.isReachable) return@map null
                d.plugin<CompanionPlugin>(CompanionPlugin.ID)?.let { it to (all[pc]?.identity?.deviceName ?: "the PC") }
            }
        }
        if (each.isEmpty()) flowOf(null) else combine(each) { found -> found.firstOrNull { it != null } }
    }.distinctUntilChanged { a, b -> a?.first === b?.first }

/**
 * The hardware H.264 encoder: frames in through [surface] (a camera, a virtual display), Annex-B
 * out to [out]. Every keyframe carries SPS/PPS; when [out] is full it skips to the next keyframe.
 */
internal class H264Encoder(width: Int, height: Int, bitrate: Int, fps: Int, private val out: LiveStream, private val name: String) {
    private val thread = HandlerThread(name).also { it.start() }
    val handler = Handler(thread.looper)
    private val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
    val surface: Surface
    private var config: ByteArray? = null
    private var dropping = false

    init {
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
            setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            // A 120 Hz screen doesn't make more frames than asked for.
            setFloat(MediaFormat.KEY_MAX_FPS_TO_ENCODER, fps.toFloat())
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
                    VLog.i("H264Encoder", "$name: the link is behind, skipping to the next keyframe")
                    sync()
                }
            }
            override fun onError(codec: MediaCodec, e: MediaCodec.CodecException) { VLog.w("H264Encoder", "$name: ${e.message}"); out.close() }
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
