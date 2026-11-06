package dev.vyoma.connect.platform

import android.os.Handler
import android.os.Looper
import android.view.FrameMetrics
import android.view.Window
import dev.vyoma.connect.protocol.VLog

/**
 * The PC screen after the decoder hands a picture over, which [ScreenDecoder]'s figures can't see:
 * a TextureView's picture waits for the app's main thread and the GPU. Logs every few seconds while
 * anything is drawn. Everything here runs on the main thread.
 */
class ViewerTimings : Window.OnFrameMetricsAvailableListener {
    private val handler = Handler(Looper.getMainLooper())
    private var mark = System.nanoTime()
    private var pictures = 0
    private var lastPicture = 0L
    private var lateGaps = 0
    private var worstGapNs = 0L
    private var frames = 0
    private var slowFrames = 0
    private var worstNs = 0L
    private var worstMainNs = 0L
    private var worstGpuNs = 0L

    fun start(window: Window) {
        mark = System.nanoTime()
        window.addOnFrameMetricsAvailableListener(this, handler)
    }

    fun stop(window: Window) {
        runCatching { window.removeOnFrameMetricsAvailableListener(this) }
        lastPicture = 0
    }

    /** The TextureView took a new picture. A long gap while pictures keep coming is a hiccup; a longer one is a still screen. */
    fun pictureArrived() {
        val now = System.nanoTime()
        pictures++
        val gap = now - lastPicture
        if (lastPicture != 0L && gap in LATE_GAP_NS..STILL_GAP_NS) {
            lateGaps++
            worstGapNs = maxOf(worstGapNs, gap)
        }
        lastPicture = now
    }

    override fun onFrameMetricsAvailable(window: Window, metrics: FrameMetrics, dropped: Int) {
        val total = metrics.getMetric(FrameMetrics.TOTAL_DURATION)
        frames++
        if (total > SLOW_FRAME_NS) slowFrames++
        if (total > worstNs) {
            worstNs = total
            worstMainNs = MAIN_THREAD.sumOf { metrics.getMetric(it) }
            worstGpuNs = metrics.getMetric(FrameMetrics.GPU_DURATION)
        }
        val now = System.nanoTime()
        if (now - mark < LOG_EVERY_NS) return
        VLog.d(TAG, "%d pictures, %d late (worst %d ms); %d frames, %d slow, worst %d ms (main %d, GPU %d)".format(
            pictures, lateGaps, worstGapNs / MS, frames, slowFrames, worstNs / MS, worstMainNs / MS, worstGpuNs / MS,
        ))
        mark = now
        pictures = 0; lateGaps = 0; worstGapNs = 0
        frames = 0; slowFrames = 0; worstNs = 0
    }

    private companion object {
        const val TAG = "ViewerTimings"
        const val MS = 1_000_000L
        const val LOG_EVERY_NS = 5_000 * MS
        const val LATE_GAP_NS = 50 * MS
        const val STILL_GAP_NS = 500 * MS
        const val SLOW_FRAME_NS = 17 * MS
        val MAIN_THREAD = intArrayOf(
            FrameMetrics.UNKNOWN_DELAY_DURATION, FrameMetrics.INPUT_HANDLING_DURATION, FrameMetrics.ANIMATION_DURATION,
            FrameMetrics.LAYOUT_MEASURE_DURATION, FrameMetrics.DRAW_DURATION, FrameMetrics.SYNC_DURATION,
        ).asList()
    }
}
