package dev.vyoma.connect.automation

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** One line of the activity list: a rule ran, was undone, or couldn't. */
data class RunEntry(val ruleId: String, val ruleName: String, val at: Long, val text: String, val failed: Boolean = false)

/** The rules, kept as one small JSON file, and the last few runs. Writes are rare (edits), so they're synchronous. */
class RuleStore(context: Context) {
    private val file = File(context.filesDir, "rules.json")
    private val logFile = File(context.filesDir, "rules-log.json")

    private val _rules = MutableStateFlow(if (file.exists()) RuleJson.decode(file.readText()) else emptyList())
    val rules: StateFlow<List<Rule>> = _rules.asStateFlow()

    private val _log = MutableStateFlow(readLog())
    val log: StateFlow<List<RunEntry>> = _log.asStateFlow()

    fun rule(id: String): Rule? = _rules.value.firstOrNull { it.id == id }

    @Synchronized
    fun save(rule: Rule) = write { list -> if (list.any { it.id == rule.id }) list.map { if (it.id == rule.id) rule else it } else list + rule }

    @Synchronized
    fun delete(id: String) = write { list -> list.filterNot { it.id == id } }

    @Synchronized
    fun setEnabled(id: String, enabled: Boolean) = write { list -> list.map { if (it.id == id) it.copy(enabled = enabled) else it } }

    private fun write(change: (List<Rule>) -> List<Rule>) {
        val next = change(_rules.value)
        file.writeText(RuleJson.encode(next))
        _rules.value = next
    }

    @Synchronized
    fun record(entry: RunEntry) {
        _log.update { (listOf(entry) + it).take(LOG_SIZE) }
        logFile.writeText(JSONArray().apply {
            _log.value.forEach { e -> put(JSONObject().put("id", e.ruleId).put("name", e.ruleName).put("at", e.at).put("text", e.text).put("failed", e.failed)) }
        }.toString())
    }

    private fun readLog(): List<RunEntry> = runCatching {
        val a = JSONArray(logFile.readText())
        (0 until a.length()).map { i ->
            val o = a.getJSONObject(i)
            RunEntry(o.getString("id"), o.getString("name"), o.getLong("at"), o.getString("text"), o.optBoolean("failed"))
        }
    }.getOrDefault(emptyList())

    private companion object {
        const val LOG_SIZE = 30
    }
}
