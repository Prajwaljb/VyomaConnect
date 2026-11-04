package dev.vyoma.connect.platform

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** What reaches the PC from one app's notifications. */
enum class NotifMode(val label: String) {
    SHOW("On"),
    /** The app's name and "New notification": nothing it says (for chats, banking, 2FA codes). */
    PRIVATE("Text hidden"),
    OFF("Off"),
}

/**
 * Per-app choices for notifications on the PC, plus which apps posted lately (so the list can put
 * them first, and name apps the launcher list can't see). Every app is [NotifMode.SHOW] until changed.
 */
object NotificationFilters {
    private const val PREFS = "notification_filters"
    private const val MODE = "mode."
    private const val SEEN = "seen."
    private const val LABEL = "label."
    /** Only note a repeat post this much later: a chatty app shouldn't write prefs per message. */
    private const val SEEN_WRITE_MS = 60_000L

    @Volatile private var prefs: SharedPreferences? = null
    private val _modes = MutableStateFlow<Map<String, NotifMode>>(emptyMap())
    val modes: StateFlow<Map<String, NotifMode>> = _modes.asStateFlow()

    /** Package → (last posted, app name). */
    private val _recent = MutableStateFlow<Map<String, Pair<Long, String>>>(emptyMap())
    val recent: StateFlow<Map<String, Pair<Long, String>>> = _recent.asStateFlow()

    fun load(context: Context): NotificationFilters {
        if (prefs != null) return this
        synchronized(this) {
            if (prefs != null) return this
            val p = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val all = p.all
            _modes.value = all.mapNotNull { (k, v) ->
                if (!k.startsWith(MODE)) return@mapNotNull null
                NotifMode.entries.firstOrNull { it.name == v }?.let { k.removePrefix(MODE) to it }
            }.toMap()
            _recent.value = all.mapNotNull { (k, v) ->
                if (!k.startsWith(SEEN) || v !is Long) return@mapNotNull null
                val pkg = k.removePrefix(SEEN)
                pkg to (v to ((all[LABEL + pkg] as? String) ?: pkg))
            }.toMap()
            prefs = p
        }
        return this
    }

    fun mode(context: Context, pkg: String): NotifMode = load(context)._modes.value[pkg] ?: NotifMode.SHOW

    fun set(context: Context, pkg: String, mode: NotifMode) {
        val p = load(context).prefs ?: return
        _modes.value = if (mode == NotifMode.SHOW) _modes.value - pkg else _modes.value + (pkg to mode)
        p.edit().apply { if (mode == NotifMode.SHOW) remove(MODE + pkg) else putString(MODE + pkg, mode.name) }.apply()
        NotificationBridge.refilter(context, pkg)
    }

    /** [pkg] just posted something the PC could show. */
    fun posted(context: Context, pkg: String, label: String, at: Long) {
        val p = load(context).prefs ?: return
        val was = _recent.value[pkg]
        if (was != null && at - was.first < SEEN_WRITE_MS && was.second == label) return
        _recent.value = _recent.value + (pkg to (at to label))
        p.edit().putLong(SEEN + pkg, at).putString(LABEL + pkg, label).apply()
    }
}
