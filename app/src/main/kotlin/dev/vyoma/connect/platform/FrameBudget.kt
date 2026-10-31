package dev.vyoma.connect.platform

import android.app.Activity
import android.app.Application
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.view.FrameMetrics
import android.view.Window
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.ui.PcScreenActivity

/**
 * Finds phones that can't keep up with the full liquid glass, from how long the app's own frames
 * take. Only frames drawn back to back count (a scroll, an animation): a lone frame after a pause
 * is slow on any phone, its CPU still clocked down. When the median of a window of such frames
 * takes at least [SLOW_RATIO] times the display's frame interval, twice running, [onSlow] is called
 * once and watching stops. A phone that keeps up just keeps being watched, cheaply, until it stops.
 *
 * Measured (median frame time over the display's interval while scrolling): a Galaxy S24 FE
 * 0.95-1.55x at 120 Hz; a moto g52 2.4-2.8x at 90 Hz (frames of ~27-31 ms against 11), which
 * stutters. A OnePlus Nord CE 3 at 120 Hz runs smooth and falls in between, so the line sits
 * above where it might reach.
 */
class FrameBudget(private val onSlow: (ratio: Float) -> Unit) {
    private val thread by lazy { HandlerThread("frame-budget").apply { start() } }
    private val listeners = HashMap<Window, Window.OnFrameMetricsAvailableListener>()
    @Volatile private var done = false

    // Touched only on [thread].
    private val window = FloatArray(WINDOW)
    private var count = 0
    private var lastVsync = 0L
    private var slowWindows = 0

    fun install(app: Application) {
        app.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            // The PC screen's video draws on its own schedule; its frames say nothing about the glass.
            override fun onActivityResumed(activity: Activity) { if (activity !is PcScreenActivity) watch(activity) }
            override fun onActivityPaused(activity: Activity) = unwatch(activity.window)
            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
            override fun onActivityStarted(activity: Activity) = Unit
            override fun onActivityStopped(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = Unit
        })
    }

    /** Stops watching for good (the user chose a glass setting, or lite glass is already on). */
    fun stop() {
        done = true
    }

    private fun watch(activity: Activity) {
        val w = activity.window
        if (done || w in listeners) return
        val display = activity.display
        val listener = Window.OnFrameMetricsAvailableListener { _, metrics, _ ->
            // The display's own frame interval, read each time: it changes (adaptive 60/120 Hz).
            // FrameMetrics' DEADLINE isn't it: it's about two intervals, which halved every ratio.
            val hz = display?.refreshRate?.takeIf { it > 1f } ?: 60f
            sample(metrics, (1e9f / hz).toLong())
        }
        listeners[w] = listener
        w.addOnFrameMetricsAvailableListener(listener, Handler(thread.looper))
    }

    private fun unwatch(w: Window) {
        listeners.remove(w)?.let { runCatching { w.removeOnFrameMetricsAvailableListener(it) } }
        lastVsync = 0L
    }

    private fun sample(m: FrameMetrics, interval: Long) {
        if (done || m.getMetric(FrameMetrics.FIRST_DRAW_FRAME) == 1L) return
        val vsync = m.getMetric(FrameMetrics.INTENDED_VSYNC_TIMESTAMP)
        // Back to back means within two intervals: a slow phone misses some vsyncs while scrolling.
        val streaming = lastVsync > 0 && vsync - lastVsync <= interval * 2
        lastVsync = vsync
        if (!streaming) return
        window[count++] = m.getMetric(FrameMetrics.TOTAL_DURATION).toFloat() / interval
        if (count < WINDOW) return
        count = 0
        val median = window.sorted()[WINDOW / 2]
        // In testers' demo logs: how close this phone runs to its limit.
        VLog.d(TAG, "median frame %.2fx the display's interval over %d frames".format(median, WINDOW))
        slowWindows = if (median >= SLOW_RATIO) slowWindows + 1 else 0
        if (slowWindows >= 2) {
            done = true
            VLog.i(TAG, "frames take %.1fx the display's interval: lighter glass".format(median))
            onSlow(median)
        }
    }

    private companion object {
        const val TAG = "FrameBudget"
        const val WINDOW = 240
        const val SLOW_RATIO = 2.2f
    }
}
