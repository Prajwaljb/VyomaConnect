package dev.vyoma.connect.debug

import android.app.Activity
import android.appwidget.AppWidgetHost
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Shader
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.util.SizeF
import android.util.TypedValue
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.lifecycle.lifecycleScope
import dev.vyoma.connect.widget.ActionsWidgetReceiver
import dev.vyoma.connect.widget.ClipboardWidgetReceiver
import dev.vyoma.connect.widget.DropWidgetReceiver
import dev.vyoma.connect.widget.MediaWidgetReceiver
import dev.vyoma.connect.widget.StatusWidgetReceiver
import dev.vyoma.connect.widget.WidgetSnapshot
import dev.vyoma.connect.widget.Widgets
import kotlinx.coroutines.launch

/**
 * Hosts our real widgets at a launcher's cell sizes over a busy background, with sample data.
 * Needs `cmd appwidget grantbind --package <pkg>` once. `--es grid`:
 *  - `oneui` (default): One UI Home 5x6 on an S24 FE, w = 71.1n - 13, h = 99.2n - 41 dp (58 x 58 a cell)
 *  - `pixel`: Pixel Launcher / AOSP 4x5 (Android's sizing guide), w = 73n - 16, h = 118n - 16 (57 x 102)
 *  - `compact`: 4x6 and 5x7 grids (ColorOS, HyperOS, OxygenOS), w = 80n - 12, h = 88n - 18 (68 x 70)
 *
 *   adb shell am start -n dev.vyoma.connect.debug/dev.vyoma.connect.debug.WidgetLabActivity --es widget media --ez art true --es grid pixel
 */
class WidgetLabActivity : androidx.activity.ComponentActivity() {
    private lateinit var host: AppWidgetHost
    private val ids = mutableListOf<Int>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        setReceiversEnabled(true)
        host = AppWidgetHost(this, HOST_ID)
        host.deleteHost() // clear widgets left from an earlier run
        val which = intent.getStringExtra("widget") ?: "status"
        val art = intent.getBooleanExtra("art", true)
        val dark = intent.getBooleanExtra("dark", false)
        val receiver = when (which) {
            "actions" -> ActionsWidgetReceiver::class.java
            "media" -> MediaWidgetReceiver::class.java
            "clipboard" -> ClipboardWidgetReceiver::class.java
            "drop" -> DropWidgetReceiver::class.java
            else -> StatusWidgetReceiver::class.java
        }
        val sizes = (intent.getStringExtra("sizes") ?: "1x1,2x1,3x1,4x1,5x1,2x2,3x2,4x2,5x2,2x3,4x3,5x4")
            .split(',').map { it.split('x').let { (c, r) -> c.toInt() to r.toInt() } }

        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(40), dp(16), dp(40))
        }
        val root = ScrollView(this).apply {
            background = GradientDrawable(
                GradientDrawable.Orientation.TL_BR,
                if (dark) intArrayOf(0xFF1B2A4A.toInt(), 0xFF5A2D2D.toInt(), 0xFF2E4A2B.toInt())
                else intArrayOf(0xFF6FA8FF.toInt(), 0xFFFFB199.toInt(), 0xFF7CC47A.toInt()),
            )
            addView(column)
        }
        setContentView(root)

        val manager = AppWidgetManager.getInstance(this)
        val provider = ComponentName(this, receiver)

        // --ez preview true: inflate this widget's One UI picker previews, one per size slot.
        if (intent.getBooleanExtra("preview", false)) {
            for (slot in listOf("tiny", "small", "wide_small", "medium", "large")) {
                @Suppress("DiscouragedApi")
                val layout = resources.getIdentifier("widget_preview_${which}_$slot", "layout", packageName)
                if (layout == 0) continue
                column.addView(label("$slot preview"))
                column.addView(FrameLayout(this).apply { addView(layoutInflater.inflate(layout, this, false)) })
            }
            return
        }
        val grid = intent.getStringExtra("grid") ?: "oneui"
        column.addView(label("grid: $grid, font scale ${resources.configuration.fontScale}"))
        for ((c, r) in sizes) {
            val (w, h) = when (grid) {
                "pixel" -> 73f * c - 16f to 118f * r - 16f
                "compact" -> 80f * c - 12f to 88f * r - 18f
                else -> 71.1f * c - 13f to 99.2f * r - 41f
            }
            val id = host.allocateAppWidgetId()
            if (!manager.bindAppWidgetIdIfAllowed(id, provider)) {
                // Newer Androids have no `cmd appwidget`: ask once in the system's dialog (tick
                // "Always allow"), then start the lab again.
                column.addView(label("Allow binding (tick Always allow), then start the lab again"))
                @Suppress("DEPRECATION")
                startActivityForResult(
                    android.content.Intent(AppWidgetManager.ACTION_APPWIDGET_BIND)
                        .putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id)
                        .putExtra(AppWidgetManager.EXTRA_APPWIDGET_PROVIDER, provider),
                    1,
                )
                break
            }
            ids += id
            column.addView(label("${c}x$r  (${w.toInt()}x${h.toInt()} dp)"))
            val view = host.createView(this, id, manager.getAppWidgetInfo(id))
            // The stock host pads widgets by 8dp and subtracts that from the size it reports; One UI
            // Home doesn't. Grow the host view by its padding so the content is exactly one cell span.
            val d = resources.displayMetrics.density
            val padW = (view.paddingLeft + view.paddingRight) / d
            val padH = (view.paddingTop + view.paddingBottom) / d
            view.updateAppWidgetSize(Bundle(), listOf(SizeF(w + padW, h + padH)))
            column.addView(FrameLayout(this).apply { addView(view, ViewGroup.LayoutParams(dp(w + padW), dp(h + padH))) })
        }
        host.startListening()

        lifecycleScope.launch {
            Widgets.showSample(
                this@WidgetLabActivity,
                WidgetSnapshot(
                    pcName = "vyoma", connected = true, pcBattery = 82, pcCharging = false,
                    phoneBattery = 64, phoneCharging = true,
                    title = "Midnight City (Extended Mix)", artist = "M83", playing = true,
                    art = if (art) sampleArt() else null,
                    clips = listOf("https://kde.org/connect", "Meeting moved to 4:30 tomorrow", "Wi-Fi: hunter2", "git rebase -i HEAD~3"),
                    transferFraction = if (intent.getBooleanExtra("transfer", false)) 0.42f else null,
                    transferLabel = "photos.zip",
                ),
            )
        }
    }

    override fun onDestroy() {
        host.stopListening()
        ids.forEach(host::deleteAppWidgetId)
        setReceiversEnabled(false)
        super.onDestroy()
    }

    /** The debug manifest disables our widget receivers so they don't double up in the launcher picker. */
    private fun setReceiversEnabled(enabled: Boolean) {
        val state = if (enabled) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DEFAULT
        RECEIVERS.forEach {
            packageManager.setComponentEnabledSetting(ComponentName(this, it), state, PackageManager.DONT_KILL_APP)
        }
    }

    private fun label(text: String) = TextView(this).apply {
        this.text = text
        setTextColor(0xFFFFFFFF.toInt())
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
        setPadding(0, dp(14), 0, dp(6))
    }

    private fun dp(v: Number) = (v.toFloat() * resources.displayMetrics.density).toInt()

    private fun sampleArt(): Bitmap = Bitmap.createBitmap(480, 480, Bitmap.Config.ARGB_8888).also {
        val p = Paint().apply { shader = LinearGradient(0f, 0f, 480f, 480f, 0xFFE94057.toInt(), 0xFF4A00E0.toInt(), Shader.TileMode.CLAMP) }
        Canvas(it).drawRect(0f, 0f, 480f, 480f, p)
    }

    private companion object {
        const val HOST_ID = 7701
        val RECEIVERS = listOf(
            StatusWidgetReceiver::class.java, ActionsWidgetReceiver::class.java, MediaWidgetReceiver::class.java,
            ClipboardWidgetReceiver::class.java, DropWidgetReceiver::class.java,
        )
    }
}
