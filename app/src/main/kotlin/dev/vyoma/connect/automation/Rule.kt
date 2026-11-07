package dev.vyoma.connect.automation

import dev.vyoma.connect.protocol.link.TransportKind
import org.json.JSONArray
import org.json.JSONObject
import java.util.Calendar
import java.util.UUID

/**
 * A "When / Only if / Do" rule, like Samsung's Modes & Routines. Every trigger is a state that
 * turns on and off (a call is on, the PC is connected); the rule runs when its trigger turns on
 * while every condition holds, and with [undo] puts back what it changed when the trigger turns
 * off again. A time without an end is a moment, so it has nothing to undo.
 */
data class Rule(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val enabled: Boolean = true,
    val trigger: Trigger,
    val conditions: List<Trigger> = emptyList(),
    val actions: List<Action>,
    val undo: Boolean = false,
    /** Which suggestion it came from, so the suggestion isn't offered twice. */
    val suggestion: String? = null,
) {
    /** Undo only means something for a trigger that ends and an action that can be put back. */
    val canUndo: Boolean get() = trigger.ends && actions.any { it.undoable }
}

/** Days of the week as a bit mask, Monday = bit 0. */
const val EVERY_DAY = 0b1111111

sealed interface Trigger {
    /** False for a moment in time: it fires and is over, nothing to undo. */
    val ends: Boolean get() = true

    data class PcLink(val connected: Boolean, val afterMin: Int = 0) : Trigger
    data class Call(val ringingOnly: Boolean) : Trigger
    data class PhoneCharging(val charging: Boolean) : Trigger
    data class PhoneBatteryBelow(val percent: Int) : Trigger
    data class PcBatteryBelow(val percent: Int) : Trigger
    data class PcCharging(val charging: Boolean) : Trigger
    data class PcLocked(val locked: Boolean) : Trigger
    data class PcPlaying(val playing: Boolean) : Trigger
    data class Headphones(val connected: Boolean) : Trigger
    data class HomeWifi(val home: Boolean) : Trigger
    /** One paired device, by address; [name] is what it was called when picked. */
    data class BluetoothDevice(val address: String, val name: String, val connected: Boolean) : Trigger
    /** How the PC's link runs right now. */
    data class PcVia(val via: TransportKind) : Trigger
    /** One Wi-Fi network: [print] is its fingerprint (see Signals), [name] what the user called it. */
    data class WifiNetwork(val print: String, val name: String, val on: Boolean) : Trigger
    /** Minutes after midnight; [end] null is a moment, else a period (it may run past midnight). */
    data class Time(val start: Int, val end: Int?, val days: Int = EVERY_DAY) : Trigger {
        override val ends: Boolean get() = end != null
    }
}

enum class RingerMode { VIBRATE, NORMAL }

sealed interface Action {
    /** Whether it can be put back when its rule's trigger ends. */
    val undoable: Boolean get() = false
    /** Runs on the PC (so it needs it connected), as opposed to on this phone. */
    val onPc: Boolean get() = true
    /** Needs the companion on the PC. */
    val companion: Boolean get() = false

    data object LockPc : Action
    data class MutePc(val mute: Boolean) : Action { override val undoable get() = true }
    data class PcVolume(val percent: Int) : Action { override val undoable get() = true }
    data class PcMedia(val play: Boolean) : Action { override val undoable get() = true }
    data class PcBrightness(val percent: Int) : Action {
        override val undoable get() = true
        override val companion get() = true
    }
    data class RunCommand(val key: String, val name: String) : Action
    data object SendClipboard : Action
    data class MessagePc(val text: String) : Action
    data object PcScreenshot : Action { override val companion get() = true }
    data object WakePcScreen : Action { override val companion get() = true }
    data object SleepPc : Action { override val companion get() = true }

    data class NotifyMe(val text: String) : Action { override val onPc get() = false }
    data class PhoneRinger(val mode: RingerMode) : Action {
        override val onPc get() = false
        override val undoable get() = true
    }
    data object PausePhoneMedia : Action {
        override val onPc get() = false
        override val undoable get() = true
    }
}

/**
 * What the companion can do on the PC by itself once the phone has gone (a "PC goes away" rule
 * can't reach the PC from here any more): lock, pause and mute.
 */
fun Action.runsOnPcWhenAway(): Boolean = this == Action.LockPc || this == Action.PcMedia(play = false) || this == Action.MutePc(mute = true)

/** When a [Trigger.Time] holds, and when it next changes. Local time; Monday is day 0. */
internal object TimeRules {
    /** A moment counts for this long, so an alarm Android delivers a little late still fires it. */
    const val MOMENT_MIN = 10
    private const val DAY_MIN = 24 * 60

    private fun end(t: Trigger.Time) = t.end ?: ((t.start + MOMENT_MIN) % DAY_MIN)

    fun holds(t: Trigger.Time, now: Long): Boolean {
        val cal = Calendar.getInstance().apply { timeInMillis = now }
        val minute = cal.get(Calendar.HOUR_OF_DAY) * 60 + cal.get(Calendar.MINUTE)
        val today = (cal.get(Calendar.DAY_OF_WEEK) + 5) % 7
        val yesterday = (today + 6) % 7
        fun day(d: Int) = t.days and (1 shl d) != 0
        val end = end(t)
        return when {
            t.start == end -> day(today)
            t.start < end -> minute in t.start until end && day(today)
            // Past midnight: the evening belongs to its own day, the small hours to the day before.
            else -> (minute >= t.start && day(today)) || (minute < end && day(yesterday))
        }
    }

    /** The next start or end after [now] (a second past the minute, so the check lands inside it). */
    fun nextBoundary(t: Trigger.Time, now: Long): Long {
        val midnight = Calendar.getInstance().apply {
            timeInMillis = now; set(Calendar.HOUR_OF_DAY, 0); set(Calendar.MINUTE, 0); set(Calendar.SECOND, 0); set(Calendar.MILLISECOND, 0)
        }
        val marks = listOf(t.start, end(t))
        return (0..1).flatMap { d ->
            marks.map { m ->
                (midnight.clone() as Calendar).apply { add(Calendar.DAY_OF_MONTH, d); set(Calendar.HOUR_OF_DAY, m / 60); set(Calendar.MINUTE, m % 60); set(Calendar.SECOND, 1) }.timeInMillis // on the clock, as holds() reads it: a day can be 23 hours
            }
        }.filter { it > now }.min()
    }
}

// JSON: plain org.json, one object per rule, a "type" per trigger and action.

internal object RuleJson {
    fun encode(rules: List<Rule>): String = JSONArray().apply { rules.forEach { put(rule(it)) } }.toString()

    fun decode(text: String): List<Rule> = runCatching {
        val a = JSONArray(text)
        (0 until a.length()).mapNotNull { runCatching { rule(a.getJSONObject(it)) }.getOrNull() }
    }.getOrDefault(emptyList())

    private fun rule(r: Rule) = JSONObject().apply {
        put("id", r.id); put("name", r.name); put("enabled", r.enabled); put("undo", r.undo)
        r.suggestion?.let { put("suggestion", it) }
        put("trigger", trigger(r.trigger))
        put("conditions", JSONArray().apply { r.conditions.forEach { put(trigger(it)) } })
        put("actions", JSONArray().apply { r.actions.forEach { put(action(it)) } })
    }

    private fun rule(o: JSONObject): Rule? {
        val actions = o.getJSONArray("actions").let { a -> (0 until a.length()).mapNotNull { action(a.getJSONObject(it)) } }
        val conditions = o.optJSONArray("conditions")?.let { a -> (0 until a.length()).mapNotNull { trigger(a.getJSONObject(it)) } }.orEmpty()
        return Rule(
            id = o.getString("id"),
            name = o.getString("name"),
            enabled = o.optBoolean("enabled", true),
            trigger = trigger(o.getJSONObject("trigger")) ?: return null,
            conditions = conditions,
            actions = actions.ifEmpty { return null },
            undo = o.optBoolean("undo", false),
            suggestion = o.optString("suggestion").takeIf { it.isNotEmpty() },
        )
    }

    private fun trigger(t: Trigger) = JSONObject().apply {
        when (t) {
            is Trigger.PcLink -> { put("type", "pcLink"); put("on", t.connected); put("after", t.afterMin) }
            is Trigger.Call -> { put("type", "call"); put("ringing", t.ringingOnly) }
            is Trigger.PhoneCharging -> { put("type", "phoneCharging"); put("on", t.charging) }
            is Trigger.PhoneBatteryBelow -> { put("type", "phoneBattery"); put("percent", t.percent) }
            is Trigger.PcBatteryBelow -> { put("type", "pcBattery"); put("percent", t.percent) }
            is Trigger.PcCharging -> { put("type", "pcCharging"); put("on", t.charging) }
            is Trigger.PcLocked -> { put("type", "pcLocked"); put("on", t.locked) }
            is Trigger.PcPlaying -> { put("type", "pcPlaying"); put("on", t.playing) }
            is Trigger.Headphones -> { put("type", "headphones"); put("on", t.connected) }
            is Trigger.HomeWifi -> { put("type", "homeWifi"); put("on", t.home) }
            is Trigger.BluetoothDevice -> { put("type", "bluetooth"); put("address", t.address); put("name", t.name); put("on", t.connected) }
            is Trigger.PcVia -> { put("type", "pcVia"); put("via", t.via.name) }
            is Trigger.WifiNetwork -> { put("type", "wifi"); put("print", t.print); put("name", t.name); put("on", t.on) }
            is Trigger.Time -> { put("type", "time"); put("start", t.start); t.end?.let { put("end", it) }; put("days", t.days) }
        }
    }

    private fun trigger(o: JSONObject): Trigger? {
        val on = o.optBoolean("on", true)
        return when (o.optString("type")) {
            "pcLink" -> Trigger.PcLink(on, o.optInt("after", 0))
            "call" -> Trigger.Call(o.optBoolean("ringing", false))
            "phoneCharging" -> Trigger.PhoneCharging(on)
            "phoneBattery" -> Trigger.PhoneBatteryBelow(o.optInt("percent", 20))
            "pcBattery" -> Trigger.PcBatteryBelow(o.optInt("percent", 20))
            "pcCharging" -> Trigger.PcCharging(on)
            "pcLocked" -> Trigger.PcLocked(on)
            "pcPlaying" -> Trigger.PcPlaying(on)
            "headphones" -> Trigger.Headphones(on)
            "homeWifi" -> Trigger.HomeWifi(on)
            "bluetooth" -> Trigger.BluetoothDevice(o.getString("address"), o.optString("name"), on)
            "pcVia" -> runCatching { Trigger.PcVia(TransportKind.valueOf(o.getString("via"))) }.getOrNull()
            "wifi" -> Trigger.WifiNetwork(o.getString("print"), o.optString("name"), on)
            "time" -> Trigger.Time(o.optInt("start"), if (o.has("end")) o.getInt("end") else null, o.optInt("days", EVERY_DAY))
            else -> null
        }
    }

    private fun action(a: Action) = JSONObject().apply {
        when (a) {
            Action.LockPc -> put("type", "lockPc")
            is Action.MutePc -> { put("type", "mutePc"); put("on", a.mute) }
            is Action.PcVolume -> { put("type", "pcVolume"); put("percent", a.percent) }
            is Action.PcMedia -> { put("type", "pcMedia"); put("play", a.play) }
            is Action.PcBrightness -> { put("type", "pcBrightness"); put("percent", a.percent) }
            is Action.RunCommand -> { put("type", "runCommand"); put("key", a.key); put("name", a.name) }
            Action.SendClipboard -> put("type", "sendClipboard")
            is Action.MessagePc -> { put("type", "messagePc"); put("text", a.text) }
            Action.PcScreenshot -> put("type", "pcScreenshot")
            Action.WakePcScreen -> put("type", "wakePc")
            Action.SleepPc -> put("type", "sleepPc")
            is Action.NotifyMe -> { put("type", "notifyMe"); put("text", a.text) }
            is Action.PhoneRinger -> { put("type", "ringer"); put("mode", a.mode.name) }
            Action.PausePhoneMedia -> put("type", "pausePhone")
        }
    }

    private fun action(o: JSONObject): Action? = when (o.optString("type")) {
        "lockPc" -> Action.LockPc
        "mutePc" -> Action.MutePc(o.optBoolean("on", true))
        "pcVolume" -> Action.PcVolume(o.optInt("percent", 50))
        "pcMedia" -> Action.PcMedia(o.optBoolean("play", false))
        "pcBrightness" -> Action.PcBrightness(o.optInt("percent", 50))
        "runCommand" -> Action.RunCommand(o.getString("key"), o.optString("name"))
        "sendClipboard" -> Action.SendClipboard
        "messagePc" -> Action.MessagePc(o.optString("text"))
        "pcScreenshot" -> Action.PcScreenshot
        "wakePc" -> Action.WakePcScreen
        "sleepPc" -> Action.SleepPc
        "notifyMe" -> Action.NotifyMe(o.optString("text"))
        "ringer" -> runCatching { Action.PhoneRinger(RingerMode.valueOf(o.getString("mode"))) }.getOrNull()
        "pausePhone" -> Action.PausePhoneMedia
        else -> null
    }
}
