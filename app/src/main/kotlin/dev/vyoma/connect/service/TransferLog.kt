package dev.vyoma.connect.service

import android.content.Context
import androidx.core.content.edit
import dev.vyoma.connect.protocol.plugin.builtin.Transfer
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONArray
import org.json.JSONObject

/** A finished transfer, kept after its notification goes so Files can list what came and went. */
data class FileRecord(
    val name: String,
    val incoming: Boolean,
    val device: String,
    /** Where an incoming file landed (a content URI), for Open. */
    val location: String?,
    val at: Long,
    val ok: Boolean,
)

/** The last [KEEP] finished transfers, newest first, saved across restarts. */
object TransferLog {
    private const val KEEP = 30
    private val _recent = MutableStateFlow<List<FileRecord>?>(null)
    private lateinit var appContext: Context

    fun recent(context: Context): StateFlow<List<FileRecord>?> {
        load(context)
        return _recent.asStateFlow()
    }

    fun add(context: Context, t: Transfer) {
        load(context)
        val record = FileRecord(t.fileName, t.incoming, t.deviceName, t.location, System.currentTimeMillis(), t.state == Transfer.State.DONE)
        _recent.update { (listOf(record) + it.orEmpty()).take(KEEP) }
        save()
    }

    fun clear(context: Context) {
        load(context)
        _recent.value = emptyList()
        save()
    }

    @Synchronized
    private fun load(context: Context) {
        if (_recent.value != null) return
        appContext = context.applicationContext
        val raw = prefs().getString("log", null)
        _recent.value = runCatching {
            val a = JSONArray(raw ?: "[]")
            List(a.length()) { i ->
                val o = a.getJSONObject(i)
                FileRecord(
                    o.getString("n"), o.getBoolean("in"), o.optString("d"),
                    o.optString("l").ifEmpty { null }, o.getLong("t"), o.optBoolean("ok", true),
                )
            }
        }.getOrDefault(emptyList())
    }

    private fun save() {
        val a = JSONArray()
        _recent.value.orEmpty().forEach {
            a.put(JSONObject().put("n", it.name).put("in", it.incoming).put("d", it.device).put("l", it.location ?: "").put("t", it.at).put("ok", it.ok))
        }
        prefs().edit { putString("log", a.toString()) }
    }

    private fun prefs() = appContext.getSharedPreferences("transfer_log", Context.MODE_PRIVATE)
}
