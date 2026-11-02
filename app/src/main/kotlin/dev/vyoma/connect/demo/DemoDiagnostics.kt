package dev.vyoma.connect.demo

import android.app.Activity
import android.app.ActivityManager
import android.app.Application
import android.app.ApplicationExitInfo
import android.content.ContentValues
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.os.Debug
import android.os.Environment
import android.os.Handler
import android.os.HandlerThread
import android.os.PowerManager
import android.os.Process
import android.os.SystemClock
import android.provider.MediaStore
import android.provider.Settings
import android.view.FrameMetrics
import android.view.Window
import dev.vyoma.connect.BuildConfig
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.VLog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Research log for the demo build: how smoothly each screen draws on this phone (from the
 * platform's per-frame timings), what the phone is, and the app's own log lines. "Generate log"
 * in Settings writes it to Downloads as a text file to send back.
 */
object DemoDiagnostics {
    /** The screen on show, so frames are filed under it. Set by the navigation. */
    @Volatile var screen: String = "Home"

    private class FrameStats {
        var frames = 0
        var janky = 0
        var frozen = 0
        var totalMs = 0.0
        var worstMs = 0.0
        /** Frame time histogram in whole milliseconds; the last bucket collects everything slower. */
        val histogram = IntArray(HISTOGRAM_MS + 1)

        fun add(ms: Double, deadlineMs: Double) {
            frames++
            totalMs += ms
            if (ms > worstMs) worstMs = ms
            if (ms > deadlineMs) janky++
            if (ms > FROZEN_MS) frozen++
            histogram[ms.toInt().coerceIn(0, HISTOGRAM_MS)]++
        }

        fun percentile(p: Double): Int {
            val target = (frames * p).toInt().coerceAtLeast(1)
            var seen = 0
            histogram.forEachIndexed { ms, n -> seen += n; if (seen >= target) return ms }
            return HISTOGRAM_MS
        }
    }

    private val stats = LinkedHashMap<String, FrameStats>()
    private val lock = Any()
    private var firstFrameMs: Long? = null
    private var installedAt = 0L
    private lateinit var crashFile: File

    private val metricsThread by lazy { HandlerThread("demo-frames").apply { start() } }
    private val listeners = HashMap<Window, Window.OnFrameMetricsAvailableListener>()

    fun install(app: Application) {
        installedAt = SystemClock.uptimeMillis()
        crashFile = File(app.filesDir, "demo-last-crash.txt")
        // A crash is the most useful thing to report: keep its trace for the next log.
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, e ->
            runCatching { crashFile.writeText("${stamp()} on ${thread.name}\n${e.stackTraceToString()}") }
            previous?.uncaughtException(thread, e)
        }
        app.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityResumed(activity: Activity) = track(activity.window)
            override fun onActivityPaused(activity: Activity) = untrack(activity.window)
            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
            override fun onActivityStarted(activity: Activity) = Unit
            override fun onActivityStopped(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = Unit
        })
    }

    private fun track(window: Window) {
        if (window in listeners) return
        val listener = Window.OnFrameMetricsAvailableListener { _, metrics, _ ->
            val now = SystemClock.uptimeMillis()
            if (firstFrameMs == null) firstFrameMs = now - Process.getStartUptimeMillis()
            // The first frame of a window is layout and inflation, not smoothness: counted in startup instead.
            if (metrics.getMetric(FrameMetrics.FIRST_DRAW_FRAME) == 1L) return@OnFrameMetricsAvailableListener
            val ms = metrics.getMetric(FrameMetrics.TOTAL_DURATION) / 1e6
            val deadline = metrics.getMetric(FrameMetrics.DEADLINE) / 1e6
            synchronized(lock) { stats.getOrPut(screen) { FrameStats() }.add(ms, deadline) }
        }
        listeners[window] = listener
        window.addOnFrameMetricsAvailableListener(listener, Handler(metricsThread.looper))
    }

    private fun untrack(window: Window) {
        listeners.remove(window)?.let { runCatching { window.removeOnFrameMetricsAvailableListener(it) } }
    }

    /**
     * After a launch that crashed (in Kotlin, in native code such as the GPU driver, or froze), writes
     * the crash and a full log to Downloads at once: a phone that crashes on every tap can't reach
     * Settings to make one. Each crash is reported once.
     */
    suspend fun reportCrashes(context: Context) = withContext(Dispatchers.IO) {
        runCatching {
            val am = context.getSystemService(ActivityManager::class.java)
            val seen = File(context.filesDir, "demo-crash-seen")
            val since = seen.takeIf { it.exists() }?.readText()?.trim()?.toLongOrNull() ?: 0L
            val exits = am.getHistoricalProcessExitReasons(context.packageName, 0, 8)
                .filter { it.timestamp > since && it.reason in CRASHES }
            seen.writeText((exits.maxOfOrNull { it.timestamp } ?: since).coerceAtLeast(since).toString())
            if (exits.isEmpty()) return@runCatching
            val text = buildString {
                for (e in exits) {
                    section("Crash")
                    line("When", SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.US).format(Date(e.timestamp)))
                    line("Kind", when (e.reason) {
                        ApplicationExitInfo.REASON_CRASH -> "app (Kotlin/Java)"
                        ApplicationExitInfo.REASON_CRASH_NATIVE -> "native (system library or GPU driver)"
                        else -> "froze (not responding)"
                    })
                    line("Description", e.description.orEmpty())
                    e.traceInputStream?.use { it.readBytes() }?.let { bytes ->
                        // Native: a tombstone (protobuf); its frames are readable text runs. ANR: plain text.
                        val trace = if (e.reason == ApplicationExitInfo.REASON_CRASH_NATIVE) readable(bytes) else bytes.decodeToString()
                        appendLine(trace.lines().take(400).joinToString("\n"))
                    }
                }
            }
            VLog.i("DemoDiagnostics", "wrote " + write(context, "vyoma-crash-", text + report(context)))
        }.onFailure { VLog.w("DemoDiagnostics", "couldn't write the crash report", it) }
    }

    /** The runs of printable text in a binary trace, one per line. */
    private fun readable(bytes: ByteArray): String =
        Regex("[\\x20-\\x7e]{6,}").findAll(String(bytes, Charsets.ISO_8859_1)).joinToString("\n") { it.value }

    private val CRASHES = setOf(ApplicationExitInfo.REASON_CRASH, ApplicationExitInfo.REASON_CRASH_NATIVE, ApplicationExitInfo.REASON_ANR)

    /** Writes the log to Downloads; returns the file name. */
    suspend fun writeLog(context: Context): Result<String> = withContext(Dispatchers.IO) {
        runCatching { write(context, "vyoma-demo-log-", report(context)) }
    }

    private fun write(context: Context, prefix: String, text: String): String {
        run {
            val name = prefix + SimpleDateFormat("yyyy-MM-dd_HH-mm-ss", Locale.US).format(Date()) + ".txt"
            val resolver = context.contentResolver
            val values = ContentValues().apply {
                put(MediaStore.Downloads.DISPLAY_NAME, name)
                put(MediaStore.Downloads.MIME_TYPE, "text/plain")
                put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
                put(MediaStore.Downloads.IS_PENDING, 1)
            }
            val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: error("Downloads isn't available")
            try {
                resolver.openOutputStream(uri)!!.use { it.write(text.toByteArray()) }
                resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            } catch (e: Exception) {
                resolver.delete(uri, null, null)
                throw e
            }
            return name
        }
    }

    private suspend fun report(context: Context): String = buildString {
        val container = (context.applicationContext as Application).container
        val pm = context.packageManager.getPackageInfo(context.packageName, 0)
        val power = context.getSystemService(PowerManager::class.java)
        val am = context.getSystemService(ActivityManager::class.java)
        // The app's own context (a crash report at launch) has no display of its own.
        val display = runCatching { context.display }.getOrNull()
            ?: context.getSystemService(android.hardware.display.DisplayManager::class.java)?.getDisplay(android.view.Display.DEFAULT_DISPLAY)
        val res = context.resources
        val upMs = SystemClock.uptimeMillis() - installedAt

        section("Vyoma demo log")
        line("Generated", stamp())
        line("App", "${pm.versionName} (${pm.longVersionCode}) ${BuildConfig.BUILD_TYPE}")
        line("Session length", "${upMs / 60_000} min ${upMs / 1000 % 60} s")

        section("Phone")
        line("Model", "${Build.MANUFACTURER} ${Build.MODEL} (${Build.DEVICE})")
        line("Chip", "${Build.SOC_MANUFACTURER} ${Build.SOC_MODEL}")
        line("Android", "${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}), patch ${Build.VERSION.SECURITY_PATCH}")
        line("Build", Build.DISPLAY)
        line("Haptics", dev.vyoma.connect.designsystem.Haptics.describe())
        line("CPU cores", "${Runtime.getRuntime().availableProcessors()} · ${Build.SUPPORTED_ABIS.joinToString()}")
        val mem = ActivityManager.MemoryInfo().also { am.getMemoryInfo(it) }
        line("RAM", "%.1f GB total, %.1f GB free%s".format(mem.totalMem / 1e9, mem.availMem / 1e9, if (mem.lowMemory) " (low)" else ""))
        line("App memory class", "${am.memoryClass} MB (large ${am.largeMemoryClass} MB)${if (am.isLowRamDevice) ", low-RAM device" else ""}")

        section("Display")
        val dm = res.displayMetrics
        line("Resolution", "${dm.widthPixels} x ${dm.heightPixels} px, ${dm.densityDpi} dpi (x${dm.density})")
        display?.let { d ->
            line("Refresh rate", "%.0f Hz now; modes: %s".format(d.refreshRate, d.supportedModes.map { "%.0f".format(it.refreshRate) }.distinct().joinToString()))
        }
        line("Font scale", res.configuration.fontScale.toString())
        line("Animation scale", Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f).toString())
        line("System dark mode", ((res.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) == android.content.res.Configuration.UI_MODE_NIGHT_YES).toString())
        val theme = container.settings.theme.first()
        line("App theme", "${theme.darkMode}, palette ${theme.palette}")
        val glass = container.settings.liteGlass.first()
        line("Glass", if (!glass.on) "full" else "lighter (" + (if (glass.chosen) "chosen" else "auto: a modest phone, or frames too slow") + ")")

        section("Power")
        line("Battery saver", power.isPowerSaveMode.toString())
        line("Thermal status", when (power.currentThermalStatus) {
            PowerManager.THERMAL_STATUS_NONE -> "none"
            PowerManager.THERMAL_STATUS_LIGHT -> "light"
            PowerManager.THERMAL_STATUS_MODERATE -> "moderate"
            PowerManager.THERMAL_STATUS_SEVERE -> "severe"
            else -> "critical (${power.currentThermalStatus})"
        })
        container.battery.state.value?.let { line("Phone battery", "${it.charge}%${if (it.charging) ", charging" else ""}") }
        line("Background unrestricted", power.isIgnoringBatteryOptimizations(context.packageName).toString())

        section("Performance")
        line("Start to first frame", firstFrameMs?.let { "$it ms (approx., process start to first frame shown)" } ?: "not seen")
        val cpuMs = Process.getElapsedCpuTime()
        val aliveMs = SystemClock.uptimeMillis() - Process.getStartUptimeMillis()
        line("App CPU time", "%.1f s over %.0f s alive (%.1f%% of one core)".format(cpuMs / 1000.0, aliveMs / 1000.0, 100.0 * cpuMs / aliveMs.coerceAtLeast(1)))
        val dmi = Debug.MemoryInfo().also { Debug.getMemoryInfo(it) }
        line("App memory (PSS)", "total ${kb(dmi.getMemoryStat("summary.total-pss"))}, Java ${kb(dmi.getMemoryStat("summary.java-heap"))}, " +
            "native ${kb(dmi.getMemoryStat("summary.native-heap"))}, graphics ${kb(dmi.getMemoryStat("summary.graphics"))}")
        appendLine()
        appendLine("Frames by screen (janky = missed its display deadline; frozen = over $FROZEN_MS ms):")
        appendLine(String.format(Locale.US, "  %-16s %7s %7s %7s %6s %6s %6s %6s %7s %7s", "screen", "frames", "janky", "jank%", "p50", "p90", "p95", "p99", "worst", "frozen"))
        synchronized(lock) {
            if (stats.isEmpty()) appendLine("  (no frames recorded yet)")
            for ((screen, s) in stats) {
                appendLine(String.format(Locale.US, "  %-16s %7d %7d %6.1f%% %4dms %4dms %4dms %4dms %5.0fms %7d",
                    screen.take(16), s.frames, s.janky, 100.0 * s.janky / s.frames.coerceAtLeast(1),
                    s.percentile(0.5), s.percentile(0.9), s.percentile(0.95), s.percentile(0.99), s.worstMs, s.frozen))
            }
        }

        section("Devices")
        container.hub.value?.devices?.value?.values?.forEach { d ->
            val st = d.state.value
            line(st.identity.deviceName, "${if (st.isCompanion) "companion" else st.identity.deviceType.wire}, paired ${st.isPaired}, " +
                "links ${st.transports.joinToString().ifEmpty { "none" }}, plugins ${st.plugins.sorted().joinToString()}")
        } ?: appendLine("  (not started)")

        if (crashFile.exists()) {
            section("Last crash")
            appendLine(crashFile.readText())
        }

        section("App log (this process, newest last)")
        appendLine(appLog())
    }

    /** This process's own logcat lines: an app may always read its own, no permission needed. */
    private fun appLog(): String = runCatching {
        val proc = ProcessBuilder("logcat", "-d", "-v", "threadtime", "--pid=${Process.myPid()}", "-t", "2500")
            .redirectErrorStream(true).start()
        proc.inputStream.bufferedReader().use { it.readText() }.also { proc.waitFor() }
    }.getOrElse { "(couldn't read the log: ${it.message})" }

    private fun StringBuilder.section(title: String) {
        appendLine()
        appendLine("== $title ==")
    }

    private fun StringBuilder.line(key: String, value: String) {
        appendLine("  ${key.padEnd(24)} $value")
    }

    private fun kb(stat: String?) = stat?.toLongOrNull()?.let { "%.1f MB".format(it / 1024.0) } ?: "?"

    private fun stamp() = SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.US).format(Date())

    private const val HISTOGRAM_MS = 250
    private const val FROZEN_MS = 700
}
