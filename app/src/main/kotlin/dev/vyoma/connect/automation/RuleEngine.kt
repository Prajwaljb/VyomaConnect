package dev.vyoma.connect.automation

import android.app.AlarmManager
import android.app.Application
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.media.AudioManager
import android.os.SystemClock
import android.view.KeyEvent
import dev.vyoma.connect.AppContainer
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.plugin.builtin.AwayRules
import dev.vyoma.connect.protocol.plugin.builtin.ClipboardPlugin
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import dev.vyoma.connect.protocol.plugin.builtin.MprisPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PcPower
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.protocol.plugin.builtin.RunCommandPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SystemVolumePlugin
import dev.vyoma.connect.service.Commands
import dev.vyoma.connect.service.Notifications
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONArray
import org.json.JSONObject

/**
 * Runs the rules. A rule is "on" while its trigger and every condition hold; turning on runs its
 * actions, turning off (or being switched off or deleted) puts back what it changed, if it
 * undoes. Which rules are on, and how to undo them, survive the process (a call can end while
 * Android has killed us). Battery thresholds have a 3% hysteresis so they don't flap.
 */
class RuleEngine(private val app: Application, private val c: AppContainer) {
    val store by lazy { RuleStore(app) }
    private val signals by lazy { Signals(app, c, c.appScope) }
    private val prefs = app.getSharedPreferences("automation", Context.MODE_PRIVATE)
    private val clock = MutableStateFlow(System.currentTimeMillis())
    private val lock = Mutex()
    private val on = HashMap<String, List<Undo>>()
    private var started = false
    private var startedAt = 0L
    private var wake: Job? = null

    fun start() {
        synchronized(this) {
            if (started) return
            started = true
        }
        startedAt = System.currentTimeMillis()
        loadOn()
        c.appScope.launch {
            combine(signals.snapshot, store.rules, clock) { s, rules, now -> Triple(s, rules, now) }.collect { (s, rules, now) ->
                lock.withLock { evaluate(s, rules, now) }
                schedule(rules, s, now)
            }
        }
        // Inputs settle in the first seconds (the PC's link, Wi-Fi): look again once they have.
        c.appScope.launch {
            delay(SETTLE_MS)
            clock.value = System.currentTimeMillis()
        }
        // Walk-away rules live on the PC: keep every companion told.
        c.appScope.launch {
            store.rules.map(::awayRules).distinctUntilChanged().collect { rules ->
                CompanionPlugin.away.value = rules
                companions().forEach { it.sendAway() }
            }
        }
    }

    /** Paired Bluetooth devices, (address, name), to pick from. */
    fun pairedBluetooth(): List<Pair<String, String>> = signals.pairedBluetooth()

    /** The fingerprint of the Wi-Fi this phone is on, if any. */
    fun currentWifi(): String? = signals.snapshot.value.wifiPrint

    /** A scheduled time came (or the phone rebooted): look again. */
    fun onAlarm() {
        start()
        clock.value = System.currentTimeMillis()
    }

    /** "Test now": runs the actions once, right away, without arming an undo. */
    suspend fun test(rule: Rule) {
        lock.withLock {
            val (ran, failed) = perform(rule, away = false)
            record(rule, "Tested" + outcome(ran, failed), failed.isNotEmpty() && ran.isEmpty())
        }
    }

    // Evaluation

    private suspend fun evaluate(s: Snapshot, rules: List<Rule>, now: Long) {
        if (!s.ready || now - startedAt < SETTLE_MS - 100) return
        val byId = rules.associateBy { it.id }
        // Switched off or deleted while on: put things back.
        for (id in on.keys.toList()) {
            val rule = byId[id]
            if (rule == null || !rule.enabled) {
                val undos = on.remove(id).orEmpty()
                if (rule?.undo == true && undos.isNotEmpty()) revert(rule, undos, "Switched off")
            }
        }
        for (rule in rules) {
            if (!rule.enabled) continue
            val wasOn = rule.id in on
            val holds = rule.trigger.holds(s, now, wasOn) && rule.conditions.all { it.holds(s, now, wasOn) }
            if (holds && !wasOn) {
                val away = rule.trigger is Trigger.PcLink && !rule.trigger.connected
                val (ran, failed, undos) = perform(rule, away)
                on[rule.id] = undos
                record(rule, if (ran.isEmpty() && failed.isEmpty()) "Ran on the PC" else outcome(ran, failed).removePrefix(": "), failed.isNotEmpty() && ran.isEmpty())
            } else if (!holds && wasOn) {
                val undos = on.remove(rule.id).orEmpty()
                if (rule.undo && undos.isNotEmpty()) revert(rule, undos, "Undone")
            }
        }
        saveOn()
    }

    private fun Trigger.holds(s: Snapshot, now: Long, wasOn: Boolean): Boolean = when (this) {
        is Trigger.PcLink -> if (connected) s.pcConnected else !s.pcConnected && s.pcAwaySince != null && now - s.pcAwaySince >= afterMin * 60_000L
        is Trigger.Call -> if (ringingOnly) s.call == CallState.RINGING else s.call != CallState.NONE
        is Trigger.PhoneCharging -> s.phoneBattery != null && s.phoneCharging == charging
        is Trigger.PhoneBatteryBelow -> s.phoneBattery != null && s.phoneBattery < percent + if (wasOn) HYSTERESIS else 0
        is Trigger.PcBatteryBelow -> s.pcBattery != null && s.pcBattery < percent + if (wasOn) HYSTERESIS else 0
        is Trigger.PcCharging -> s.pcCharging == charging
        is Trigger.PcLocked -> s.pcLocked == locked
        is Trigger.PcPlaying -> if (playing) s.pcPlaying else s.pcConnected && !s.pcPlaying
        is Trigger.Headphones -> s.headphones == connected
        is Trigger.HomeWifi -> if (home) s.wifi == WifiState.HOME else s.wifi != WifiState.HOME
        is Trigger.BluetoothDevice -> (address in s.bluetooth) == connected
        is Trigger.PcVia -> s.pcVia == via
        is Trigger.WifiNetwork -> (s.wifiPrint == print) == on
        is Trigger.Time -> TimeRules.holds(this, now)
    }

    // Timing: wake for the next time boundary (or a "gone for N min" deadline).

    private fun schedule(rules: List<Rule>, s: Snapshot, now: Long) {
        val enabled = rules.filter { it.enabled }
        val times = enabled.flatMap { r -> (listOf(r.trigger) + r.conditions).filterIsInstance<Trigger.Time>() }
        val next = buildList {
            times.forEach { add(TimeRules.nextBoundary(it, now)) }
            s.pcAwaySince?.let { since ->
                enabled.forEach { r -> (r.trigger as? Trigger.PcLink)?.takeIf { !it.connected && it.afterMin > 0 }?.let { add(since + it.afterMin * 60_000L) } }
            }
            if (now - startedAt < SETTLE_MS) add(startedAt + SETTLE_MS)
        }.filter { it > now }.minOrNull()
        wake?.cancel()
        if (next != null) wake = c.appScope.launch {
            delay(next - System.currentTimeMillis())
            clock.value = System.currentTimeMillis()
        }
        // Times have to wake the phone (and us, if Android stopped us); the rest happen while we run.
        val alarm = times.minOfOrNull { TimeRules.nextBoundary(it, now) }
        val am = app.getSystemService(AlarmManager::class.java)
        val pending = PendingIntent.getBroadcast(app, 0, Intent(app, RuleAlarmReceiver::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        if (alarm == null) am.cancel(pending)
        else if (am.canScheduleExactAlarms()) am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, alarm, pending)
        else am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, alarm, pending)
    }

    // Actions

    /** Runs [rule]'s actions: what ran, what couldn't (and why), and how to put it back. */
    private suspend fun perform(rule: Rule, away: Boolean): Triple<List<String>, List<String>, List<Undo>> {
        val ran = mutableListOf<String>()
        val failed = mutableListOf<String>()
        val undos = mutableListOf<Undo>()
        val pc = Commands.primary(c)
        for (a in rule.actions) {
            val name = a.look().label
            if (a.onPc && away) {
                // The companion does these once the phone's gone; the rest can't reach the PC.
                if (!a.runsOnPcWhenAway()) failed += "$name (PC was away)"
                continue
            }
            if (a.onPc && pc == null) { failed += "$name (PC wasn't connected)"; continue }
            val result = runCatching { act(a, pc, rule) }
            result.onSuccess { undo -> ran += name; undo?.let(undos::add) }
                .onFailure { e -> failed += "$name (${e.message ?: "failed"})"; VLog.w(TAG, "${rule.name}: $name failed", e) }
        }
        return Triple(ran, failed, undos)
    }

    private suspend fun act(a: Action, pc: Device?, rule: Rule): Undo? {
        val audio = app.getSystemService(AudioManager::class.java)
        return when (a) {
            Action.LockPc -> { pc!!.plugin<LockPlugin>(LockPlugin.ID).need("lock").setLocked(true); null }
            is Action.MutePc -> {
                val (plugin, sink) = sink(pc!!)
                if (sink.muted == a.mute) null else { plugin.setMuted(sink.name, a.mute); Undo.Mute(sink.name, sink.muted) }
            }
            is Action.PcVolume -> {
                val (plugin, sink) = sink(pc!!)
                plugin.setVolume(sink.name, sink.maxVolume * a.percent / 100)
                Undo.Volume(sink.name, sink.volume)
            }
            is Action.PcMedia -> {
                val playing = c.media.nowPlaying.value?.player?.isPlaying == true
                if (playing == a.play) null else {
                    c.media.control { p, n -> p.action(n, if (a.play) MprisPlugin.MediaAction.PLAY else MprisPlugin.MediaAction.PAUSE) }
                    Undo.PcMedia(play = !a.play)
                }
            }
            is Action.PcBrightness -> {
                val companion = companionOf(pc!!).need("the companion")
                companion.requestControls()
                val displays = withTimeoutOrNull(4_000) { companion.controls.filterNotNull().first { it.displays.isNotEmpty() } }?.displays
                    ?: error("no screen reported")
                displays.forEach { companion.setBrightness(it.id, a.percent / 100f) }
                Undo.Brightness(displays.associate { it.id to it.level })
            }
            is Action.RunCommand -> { pc!!.plugin<RunCommandPlugin>(RunCommandPlugin.ID).need("commands").run(a.key); null }
            Action.SendClipboard -> {
                val text = c.clipboard.current.value?.text ?: error("nothing copied yet")
                pc!!.plugin<ClipboardPlugin>(ClipboardPlugin.ID).need("clipboard").push(text)
                null
            }
            is Action.MessagePc -> { pc!!.plugin<PingPlugin>(PingPlugin.ID).need("ping").ping(a.text.ifBlank { rule.name }); null }
            Action.PcScreenshot -> { companionOf(pc!!).need("the companion").screenshot.request(); null }
            Action.WakePcScreen -> { companionOf(pc!!).need("the companion").wake(); null }
            Action.SleepPc -> { companionOf(pc!!).need("the companion").power(PcPower.SLEEP).getOrThrow(); null }
            is Action.NotifyMe -> { Notifications.automation(app, rule.id, rule.name, a.text.ifBlank { rule.summary() }); null }
            is Action.PhoneRinger -> {
                val want = if (a.mode == RingerMode.VIBRATE) AudioManager.RINGER_MODE_VIBRATE else AudioManager.RINGER_MODE_NORMAL
                val was = audio.ringerMode
                if (was == want) null else {
                    try { audio.ringerMode = want } catch (e: SecurityException) { error("Do Not Disturb is on") }
                    Undo.Ringer(was)
                }
            }
            Action.PausePhoneMedia -> if (!audio.isMusicActive) null else { mediaKey(audio, KeyEvent.KEYCODE_MEDIA_PAUSE); Undo.PhoneMedia }
        }
    }

    private suspend fun revert(rule: Rule, undos: List<Undo>, label: String) {
        val pc = Commands.primary(c)
        val audio = app.getSystemService(AudioManager::class.java)
        var done = 0
        for (u in undos.reversed()) {
            runCatching {
                when (u) {
                    is Undo.Mute -> pc?.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.setMuted(u.sink, u.muted) ?: return@runCatching
                    is Undo.Volume -> pc?.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.setVolume(u.sink, u.volume) ?: return@runCatching
                    is Undo.PcMedia -> if (pc == null) return@runCatching else c.media.control { p, n -> p.action(n, if (u.play) MprisPlugin.MediaAction.PLAY else MprisPlugin.MediaAction.PAUSE) }
                    is Undo.Brightness -> {
                        val companion = pc?.let(::companionOf) ?: return@runCatching
                        u.levels.forEach { (id, level) -> companion.setBrightness(id, level) }
                    }
                    is Undo.Ringer -> audio.ringerMode = u.mode
                    Undo.PhoneMedia -> mediaKey(audio, KeyEvent.KEYCODE_MEDIA_PLAY)
                }
                done++
            }.onFailure { VLog.w(TAG, "${rule.name}: undo failed", it) }
        }
        if (done > 0) record(rule, label)
    }

    private fun sink(pc: Device): Pair<SystemVolumePlugin, dev.vyoma.connect.protocol.plugin.builtin.AudioSink> {
        val plugin = pc.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID).need("volume")
        val sink = plugin.sinks.value.let { list -> list.firstOrNull { it.isDefault } ?: list.firstOrNull() } ?: error("no speakers reported")
        return plugin to sink
    }

    private fun mediaKey(audio: AudioManager, code: Int) {
        val t = SystemClock.uptimeMillis()
        audio.dispatchMediaKeyEvent(KeyEvent(t, t, KeyEvent.ACTION_DOWN, code, 0))
        audio.dispatchMediaKeyEvent(KeyEvent(t, t, KeyEvent.ACTION_UP, code, 0))
    }

    private fun companionOf(pc: Device): CompanionPlugin? = c.hub.value?.devices?.value?.values
        ?.firstOrNull { it.state.value.identity.companionOf == pc.id && it.state.value.isPaired && it.state.value.isReachable }
        ?.plugin(CompanionPlugin.ID)

    private fun companions(): List<CompanionPlugin> = c.hub.value?.devices?.value?.values.orEmpty()
        .filter { it.state.value.isCompanion && it.state.value.isReachable }
        .mapNotNull { it.plugin(CompanionPlugin.ID) }

    private fun <T> T?.need(what: String): T = this ?: error("$what isn't available")

    private fun outcome(ran: List<String>, failed: List<String>): String = buildString {
        if (ran.isNotEmpty()) append(": ").append(ran.joinToString(", "))
        if (failed.isNotEmpty()) append(if (ran.isEmpty()) ": couldn't " else "; couldn't ").append(failed.joinToString(", ") { it.replaceFirstChar(Char::lowercase) })
    }

    private fun record(rule: Rule, text: String, failed: Boolean = false) {
        VLog.i(TAG, "${rule.name}: $text")
        store.record(RunEntry(rule.id, rule.name, System.currentTimeMillis(), text, failed))
    }

    // What the PC should do on its own when the phone leaves: the enabled "PC goes away" rules, merged.

    private fun awayRules(rules: List<Rule>): AwayRules {
        val away = rules.filter { it.enabled && (it.trigger as? Trigger.PcLink)?.connected == false }
        val actions = away.flatMap { it.actions }
        return AwayRules(
            lock = Action.LockPc in actions,
            pause = Action.PcMedia(play = false) in actions,
            mute = Action.MutePc(mute = true) in actions,
            afterSeconds = away.minOfOrNull { (it.trigger as Trigger.PcLink).afterMin * 60 } ?: 0,
            undo = away.any { it.undo },
        )
    }

    // Persistence of which rules are on, with their undo records.

    private fun loadOn() {
        runCatching {
            val o = JSONObject(prefs.getString(ON, "{}")!!)
            o.keys().forEach { id -> on[id] = Undo.decode(o.getJSONArray(id)) }
        }
    }

    private fun saveOn() {
        val o = JSONObject()
        on.forEach { (id, undos) -> o.put(id, JSONArray().apply { undos.forEach { put(it.encode()) } }) }
        prefs.edit().putString(ON, o.toString()).apply()
    }

    private companion object {
        const val TAG = "Rules"
        const val ON = "on"
        const val SETTLE_MS = 3_000L
        const val HYSTERESIS = 3
    }
}

/** How to put one change back. */
sealed interface Undo {
    data class Mute(val sink: String, val muted: Boolean) : Undo
    data class Volume(val sink: String, val volume: Int) : Undo
    data class PcMedia(val play: Boolean) : Undo
    data class Brightness(val levels: Map<String, Float>) : Undo
    data class Ringer(val mode: Int) : Undo
    data object PhoneMedia : Undo

    fun encode(): JSONObject = JSONObject().apply {
        when (val u = this@Undo) {
            is Mute -> { put("t", "mute"); put("sink", u.sink); put("v", u.muted) }
            is Volume -> { put("t", "volume"); put("sink", u.sink); put("v", u.volume) }
            is PcMedia -> { put("t", "media"); put("v", u.play) }
            is Brightness -> { put("t", "brightness"); put("v", JSONObject().apply { u.levels.forEach { (k, v) -> put(k, v.toDouble()) } }) }
            is Ringer -> { put("t", "ringer"); put("v", u.mode) }
            PhoneMedia -> put("t", "phoneMedia")
        }
    }

    companion object {
        fun decode(a: JSONArray): List<Undo> = (0 until a.length()).mapNotNull { i ->
            val o = a.getJSONObject(i)
            when (o.optString("t")) {
                "mute" -> Mute(o.getString("sink"), o.getBoolean("v"))
                "volume" -> Volume(o.getString("sink"), o.getInt("v"))
                "media" -> PcMedia(o.getBoolean("v"))
                "brightness" -> o.getJSONObject("v").let { m -> Brightness(m.keys().asSequence().associateWith { m.getDouble(it).toFloat() }) }
                "ringer" -> Ringer(o.getInt("v"))
                "phoneMedia" -> PhoneMedia
                else -> null
            }
        }
    }
}

/** A rule's time came, or the phone rebooted. */
class RuleAlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        (context.applicationContext as Application).container.automation.onAlarm()
    }
}
