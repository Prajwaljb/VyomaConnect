package dev.vyoma.connect.widget

import android.app.Application
import android.content.Context
import dev.vyoma.connect.protocol.VLog
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.Build
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.core.graphics.scale
import androidx.glance.action.Action
import androidx.glance.appwidget.GlanceAppWidgetManager
import androidx.glance.appwidget.action.actionSendBroadcast
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.updateAll
import androidx.glance.background
import androidx.glance.layout.height
import androidx.glance.layout.size
import androidx.glance.layout.width
import dev.vyoma.connect.platform.AppLock
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.Transfer
import dev.vyoma.connect.protocol.plugin.builtin.Transfers
import dev.vyoma.connect.service.CommandReceiver
import dev.vyoma.connect.service.Commands
import dev.vyoma.connect.ui.ClipboardActivity
import dev.vyoma.connect.ui.MainActivity
import dev.vyoma.connect.ui.ShareActivity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Where the PC is, as the widgets say it. */
enum class PcLink {
    /** Nothing paired yet. */
    NONE,
    CONNECTED,
    /** Gone without a word (another network, Wi-Fi off, crashed): the phone still looks for it. */
    AWAY,
    /** It said it was going (shut down, asleep, Vyoma quit): the phone waits until it says it's back. */
    OFF,
}

/** Everything a widget shows. Written by the service, cached on disk; widgets never touch the network. */
data class WidgetSnapshot(
    val pcName: String? = null,
    val link: PcLink = PcLink.NONE,
    /** Why it went, for [PcLink.OFF]: "shutdown", "sleep" or "stop". */
    val offWhy: String? = null,
    /** When it stopped being connected (ms), for AWAY and OFF: a clock time never goes stale on a widget. */
    val since: Long? = null,
    val pcBattery: Int? = null,
    val pcCharging: Boolean = false,
    val phoneBattery: Int? = null,
    val phoneCharging: Boolean = false,
    val title: String? = null,
    val artist: String? = null,
    val playing: Boolean = false,
    val art: Bitmap? = null,
    val clips: List<String> = emptyList(),
    /** The fingerprint lock is on: a widget can't ask, so what was copied stays out of it (and off the disk). */
    val clipsHidden: Boolean = false,
    val transferFraction: Float? = null,
    val transferLabel: String? = null,
) {
    val connected: Boolean get() = link == PcLink.CONNECTED
}

object Widgets {
    private val state = MutableStateFlow<WidgetSnapshot?>(null)

    /**
     * Called by the service when anything visible changes. Saves the snapshot first: Android may
     * later start our process just to draw a widget, and the live state won't exist then.
     */
    suspend fun refresh(context: Context) {
        if (sampleMode) return
        val snap = read(context)
        // Nothing a widget shows changed: no disk write, no redraw of five widgets.
        if (snap == state.value) return
        if (snap.pcBattery != state.value?.pcBattery) VLog.d("Widgets", "PC battery ${snap.pcBattery}% (connected=${snap.connected})")
        state.value = snap
        withContext(Dispatchers.IO) { runCatching { save(context, snap) } }
        redrawAll(context)
    }

    @Volatile private var sampleMode = false

    private suspend fun redrawAll(context: Context) =
        listOf(PcWidget(), ShortcutsWidget(), NowPlayingWidget(), ClipsWidget()).forEach { runCatching { it.updateAll(context) } }

    /** Redraws the shortcuts of one widget after its tiles were picked again. */
    suspend fun redrawShortcuts(context: Context, appWidgetId: Int) = runCatching {
        ShortcutsWidget().update(context, GlanceAppWidgetManager(context).getGlanceIdBy(appWidgetId))
    }

    /** Debug widget lab: show sample content (live refreshes paused) and redraw. */
    suspend fun showSample(context: Context, snap: WidgetSnapshot) {
        sampleMode = true
        state.value = snap
        redrawAll(context)
    }

    /**
     * Pushes the generated widget-picker previews (API 35+). The platform rate-limits this, so it
     * runs once per install/update and retries on a later start if any provider was refused.
     */
    suspend fun publishPreviews(context: Context) {
        if (Build.VERSION.SDK_INT < 35) return
        val prefs = context.getSharedPreferences("widgets", Context.MODE_PRIVATE)
        val installed = context.packageManager.getPackageInfo(context.packageName, 0).lastUpdateTime
        if (prefs.getLong("previews_for", 0) == installed) return
        val manager = GlanceAppWidgetManager(context)
        val receivers = listOf(
            PcWidgetReceiver::class, ShortcutsWidgetReceiver::class, NowPlayingWidgetReceiver::class, ClipsWidgetReceiver::class,
        )
        val ok = receivers.all { receiver ->
            runCatching { manager.setWidgetPreviews(receiver) }
                .onFailure { VLog.w("Widgets", "preview for ${receiver.simpleName} failed: ${it.message}") }
                .getOrNull() == GlanceAppWidgetManager.SET_WIDGET_PREVIEWS_RESULT_SUCCESS
        }
        if (ok) prefs.edit().putLong("previews_for", installed).apply()
    }

    /** The latest snapshot: in memory if the service ran in this process, else from disk. */
    internal fun flow(context: Context): StateFlow<WidgetSnapshot?> {
        if (state.value == null) state.compareAndSet(null, runCatching { load(context) }.getOrNull() ?: WidgetSnapshot())
        return state
    }

    @Composable
    internal fun snapshot(context: Context): WidgetSnapshot {
        LocalPreview.current?.let { return it }
        val s by flow(context).collectAsState()
        return s ?: WidgetSnapshot()
    }

    private fun read(context: Context): WidgetSnapshot {
        val c = (context.applicationContext as Application).container
        val primary = Commands.primary(c)
        val known = primary ?: c.hub.value?.devices?.value?.values?.firstOrNull { it.state.value.isPaired && !it.state.value.isCompanion }
        val offWhy = known?.let { c.hub.value?.resting?.value?.get(it.state.value.identity.deviceId) }
        val link = when {
            primary != null -> PcLink.CONNECTED
            known == null && state.value?.pcName == null -> PcLink.NONE
            offWhy != null -> PcLink.OFF
            else -> PcLink.AWAY
        }
        val before = state.value ?: runCatching { load(context) }.getOrNull()
        val since = when {
            link == PcLink.CONNECTED || link == PcLink.NONE -> null
            before?.connected == true -> System.currentTimeMillis()
            else -> before?.since ?: System.currentTimeMillis()
        }
        val pcBattery = primary?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote?.value
        // On its charger at a charge limit, stock KDE Connect says "not charging"; the companion knows better.
        val pcOnAc = primary?.let { pc ->
            c.hub.value?.devices?.value?.values?.firstOrNull { it.state.value.identity.featuresOf == pc.state.value.identity.deviceId }
                ?.plugin<CompanionPlugin>(CompanionPlugin.ID)?.stats?.value?.ac
        } == true
        val phone = c.battery.state.value
        val np = c.media.nowPlaying.value
        val running = Transfers.all.value.filter { it.state == Transfer.State.RUNNING }
        return WidgetSnapshot(
            pcName = known?.name ?: state.value?.pcName,
            link = link,
            offWhy = offWhy,
            since = since,
            pcBattery = pcBattery?.charge,
            pcCharging = pcBattery?.charging == true || pcOnAc,
            phoneBattery = phone?.charge,
            phoneCharging = phone?.charging == true,
            title = np?.player?.title?.takeIf { it.isNotBlank() },
            artist = np?.player?.artist,
            playing = np?.player?.isPlaying == true,
            // RemoteViews have a hard bitmap budget; 480px is still sharp full-bleed on a 4x2 widget.
            art = np?.art?.let(::widgetArt),
            clips = if (AppLock.enabled(context)) emptyList() else c.clipboard.history.value.take(6).map { it.text.replace('\n', ' ').take(80) },
            clipsHidden = AppLock.enabled(context),
            transferFraction = running.takeIf { it.isNotEmpty() }?.let { list ->
                val total = list.sumOf { it.totalBytes.coerceAtLeast(0) }
                if (total > 0) list.sumOf { it.doneBytes }.toFloat() / total else 0f
            },
            transferLabel = running.firstOrNull()?.let { if (running.size == 1) it.fileName else "${running.size} files" },
        )
    }

    /** The same picture stays the same object: a snapshot compares its art by identity, and saves it by that too. */
    private fun widgetArt(art: Bitmap): Bitmap {
        if (art.width <= 480) return art
        if (art !== scaledFrom) {
            scaled = art.scale(480, 480 * art.height / art.width)
            scaledFrom = art
        }
        return scaled ?: art
    }

    @Volatile private var scaledFrom: Bitmap? = null
    @Volatile private var scaled: Bitmap? = null

    // ---- disk cache ---------------------------------------------------------------------------

    private fun dir(context: Context) = File(context.filesDir, "widgets").apply { mkdirs() }

    private fun save(context: Context, s: WidgetSnapshot) {
        val json = JSONObject()
            .put("pcName", s.pcName).put("link", s.link.name).put("offWhy", s.offWhy)
            // Connected when last saved: if the process dies now, that's the last time it was seen.
            .put("since", if (s.connected) System.currentTimeMillis() else s.since)
            .put("pcBattery", s.pcBattery).put("pcCharging", s.pcCharging)
            .put("phoneBattery", s.phoneBattery).put("phoneCharging", s.phoneCharging)
            .put("title", s.title).put("artist", s.artist).put("playing", s.playing)
            .put("clips", JSONArray(s.clips))
            .put("clipsHidden", s.clipsHidden)
        File(dir(context), "snapshot.json").writeText(json.toString())
        // The art only when it's a different image: a lossless PNG of a 1024 px frame took most
        // of the background CPU when it was re-encoded on every refresh.
        if (s.art === savedArt) return // before the first save this is NOT_SYNCED: a cover left by the last run goes
        val art = File(dir(context), "art.webp")
        if (s.art != null) art.outputStream().use { s.art.compress(Bitmap.CompressFormat.WEBP_LOSSY, 85, it) } else art.delete()
        File(dir(context), "art.png").delete()
        savedArt = s.art
    }

    private val NOT_SYNCED = Any()
    @Volatile private var savedArt: Any? = NOT_SYNCED

    private fun load(context: Context): WidgetSnapshot? {
        val file = File(dir(context), "snapshot.json").takeIf { it.exists() } ?: return null
        val j = JSONObject(file.readText())
        fun JSONObject.intOrNull(k: String) = if (isNull(k)) null else optInt(k)
        fun JSONObject.strOrNull(k: String) = if (isNull(k)) null else optString(k)
        val clips = j.optJSONArray("clips")?.let { a -> List(a.length()) { a.getString(it) } }.orEmpty()
        return WidgetSnapshot(
            pcName = j.strOrNull("pcName"),
            // Whatever we cached, the link isn't up until the service says so.
            link = runCatching { PcLink.valueOf(j.optString("link")) }.getOrDefault(PcLink.AWAY)
                .let { if (it == PcLink.CONNECTED) PcLink.AWAY else it }
                .let { if (it == PcLink.NONE && j.strOrNull("pcName") != null) PcLink.AWAY else it },
            offWhy = j.strOrNull("offWhy"),
            since = if (j.isNull("since")) null else j.optLong("since"),
            pcBattery = j.intOrNull("pcBattery"),
            pcCharging = j.optBoolean("pcCharging"),
            phoneBattery = j.intOrNull("phoneBattery"),
            phoneCharging = j.optBoolean("phoneCharging"),
            title = j.strOrNull("title"),
            artist = j.strOrNull("artist"),
            playing = false,
            art = File(dir(context), "art.webp").takeIf { it.exists() }?.let { BitmapFactory.decodeFile(it.path) },
            clips = clips,
            clipsHidden = j.optBoolean("clipsHidden"),
        )
    }

    // ---- actions ------------------------------------------------------------------------------

    internal fun command(context: Context, action: String): Action =
        actionSendBroadcast(Intent(context, CommandReceiver::class.java).setAction(action))

    internal fun openApp(context: Context, screen: String? = null): Action =
        actionStartActivity(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).apply {
            screen?.let { putExtra(MainActivity.EXTRA_SCREEN, it) }
        })

    internal fun sendClipboard(context: Context): Action = actionStartActivity(ClipboardActivity.intent(context))

    internal fun pickFiles(context: Context): Action =
        actionStartActivity(Intent(context, ShareActivity::class.java).setAction(ShareActivity.ACTION_PICK).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

