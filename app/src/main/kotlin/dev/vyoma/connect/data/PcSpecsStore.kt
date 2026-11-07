package dev.vyoma.connect.data

import android.content.Context
import dev.vyoma.connect.protocol.plugin.builtin.PcSpecs
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject

/**
 * Each PC's specs as its companion last sent them, by the PC's device id, so its details page shows
 * them at once and while it's away. Kept as the body that came in, read with [PcSpecs.parse].
 */
object PcSpecsStore {
    private const val PREFS = "pc_specs"
    private val _all = MutableStateFlow<Map<String, PcSpecs>>(emptyMap())
    val all: StateFlow<Map<String, PcSpecs>> = _all.asStateFlow()

    fun load(context: Context) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        _all.value = prefs.all.mapNotNull { (id, raw) ->
            val body = (raw as? String)?.let { runCatching { Json.parseToJsonElement(it) as? JsonObject }.getOrNull() }
            body?.let(PcSpecs::parse)?.let { id to it }
        }.toMap()
    }

    fun put(context: Context, pcId: String, specs: PcSpecs, body: JsonObject) {
        if (_all.value[pcId] == specs) return
        _all.value += pcId to specs
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(pcId, body.toString()).apply()
    }

    /** The PC was unpaired: what it was goes with it. */
    fun forget(context: Context, pcId: String) {
        _all.value -= pcId
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().remove(pcId).apply()
    }
}
