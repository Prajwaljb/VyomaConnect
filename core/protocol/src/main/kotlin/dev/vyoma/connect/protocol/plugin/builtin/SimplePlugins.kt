package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put

// ---- battery ----------------------------------------------------------------------------------

data class BatteryState(val charge: Int, val charging: Boolean, val low: Boolean = false)

/** Reports our battery and keeps the peer's (laptops report theirs; desktops send charge -1). */
class BatteryPlugin(private val ctx: PluginContext, private val local: StateFlow<BatteryState?>?) : Plugin {
    private val _remote = MutableStateFlow<BatteryState?>(null)
    val remote: StateFlow<BatteryState?> = _remote.asStateFlow()

    override suspend fun onStart() {
        local ?: return
        ctx.scope.launch { local.filterNotNull().distinctUntilChanged().collect(::report) }
    }

    override suspend fun onLinkAdded() {
        local?.value?.let { report(it) }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        when (packet.type) {
            PacketTypes.BATTERY -> {
                val charge = packet.int("currentCharge") ?: -1
                _remote.value = if (charge < 0) null else BatteryState(charge, packet.bool("isCharging") ?: false, packet.int("thresholdEvent") == 1)
            }
            PacketTypes.BATTERY_REQUEST -> local?.value?.let { report(it) }
        }
        return true
    }

    private suspend fun report(state: BatteryState) = ctx.send(packet(PacketTypes.BATTERY) {
        put("currentCharge", state.charge)
        put("isCharging", state.charging)
        put("thresholdEvent", if (state.low) 1 else 0)
    })

    companion object {
        const val ID = "battery"
        fun spec(local: StateFlow<BatteryState?>?) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.BATTERY, PacketTypes.BATTERY_REQUEST),
            outgoing = setOf(PacketTypes.BATTERY),
        ) { BatteryPlugin(it, local) }
    }
}

// ---- clipboard --------------------------------------------------------------------------------

/** One clipboard value. [origin] is the device it came from (null if copied here) so we don't echo it back. */
data class ClipEntry(val text: String, val timestamp: Long, val origin: String? = null)

/** The platform clipboard, as the plugin sees it. */
interface ClipboardBridge {
    val current: StateFlow<ClipEntry?>
    fun applyRemote(text: String, fromDeviceId: String)
}

class ClipboardPlugin(private val ctx: PluginContext, private val bridge: ClipboardBridge) : Plugin {
    override suspend fun onStart() {
        val initial = bridge.current.value
        initial?.let { clip ->
            ctx.send(packet(PacketTypes.CLIPBOARD_CONNECT) {
                put("content", clip.text)
                put("timestamp", clip.timestamp)
            })
        }
        ctx.scope.launch {
            bridge.current.filterNotNull().distinctUntilChanged().collect { clip ->
                // The value we started with already went out as clipboard.connect.
                if (clip == initial) return@collect
                if (clip.origin != ctx.device.id) ctx.send(packet(PacketTypes.CLIPBOARD) { put("content", clip.text) })
            }
        }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        val content = packet.string("content") ?: return true
        when (packet.type) {
            PacketTypes.CLIPBOARD -> bridge.applyRemote(content, ctx.device.id)
            PacketTypes.CLIPBOARD_CONNECT -> {
                val ts = packet.long("timestamp") ?: 0
                // 0 means "unknown age": never let it overwrite what we have.
                if (ts != 0L && ts > (bridge.current.value?.timestamp ?: 0)) bridge.applyRemote(content, ctx.device.id)
            }
        }
        return true
    }

    /** Pushes the current clipboard now (manual send from a tile, widget or notification). */
    suspend fun push(text: String) = ctx.send(packet(PacketTypes.CLIPBOARD) { put("content", text) })

    companion object {
        const val ID = "clipboard"
        fun spec(bridge: ClipboardBridge) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.CLIPBOARD, PacketTypes.CLIPBOARD_CONNECT),
            outgoing = setOf(PacketTypes.CLIPBOARD, PacketTypes.CLIPBOARD_CONNECT),
        ) { ClipboardPlugin(it, bridge) }
    }
}

// ---- find my phone / find my PC ---------------------------------------------------------------

/** The peer asks us to ring; we can ask the PC to ring too (its "find this device" plugin). */
class FindMyPhonePlugin(private val ctx: PluginContext, private val ring: (deviceName: String) -> Unit) : Plugin {
    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        ring(ctx.device.name)
        return true
    }

    suspend fun ringPeer() = ctx.send(packet(PacketTypes.FIND_MY_PHONE_REQUEST))

    companion object {
        const val ID = "findmyphone"
        fun spec(ring: (String) -> Unit) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.FIND_MY_PHONE_REQUEST),
            outgoing = setOf(PacketTypes.FIND_MY_PHONE_REQUEST),
        ) { FindMyPhonePlugin(it, ring) }
    }
}

// ---- lock the PC ------------------------------------------------------------------------------

class LockPlugin(private val ctx: PluginContext) : Plugin {
    private val _locked = MutableStateFlow<Boolean?>(null)
    val locked: StateFlow<Boolean?> = _locked.asStateFlow()

    override suspend fun onStart() {
        ctx.send(packet(PacketTypes.LOCK_REQUEST) { put("requestLocked", true) })
    }

    override suspend fun onLinkAdded() = onStart()

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        packet.bool("isLocked")?.let { _locked.value = it }
        return true
    }

    suspend fun setLocked(locked: Boolean) = ctx.send(packet(PacketTypes.LOCK_REQUEST) { put("setLocked", locked) })

    companion object {
        const val ID = "lock"
        val spec = PluginSpec(ID, incoming = setOf(PacketTypes.LOCK), outgoing = setOf(PacketTypes.LOCK_REQUEST), ::LockPlugin)
    }
}

// ---- run commands on the PC -------------------------------------------------------------------

data class RemoteCommand(val key: String, val name: String, val command: String)

class RunCommandPlugin(private val ctx: PluginContext) : Plugin {
    private val _commands = MutableStateFlow<List<RemoteCommand>>(emptyList())
    val commands: StateFlow<List<RemoteCommand>> = _commands.asStateFlow()

    override suspend fun onStart() {
        ctx.send(packet(PacketTypes.RUNCOMMAND_REQUEST) { put("requestCommandList", true) })
    }

    override suspend fun onLinkAdded() = onStart()

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.type != PacketTypes.RUNCOMMAND) return true
        // commandList is a JSON object serialized into a string: {"<key>": {"name": .., "command": ..}}
        val list = packet.body["commandList"]
        val obj = when (list) {
            is JsonObject -> list
            is JsonPrimitive -> list.contentOrNull?.let { runCatching { Json.parseToJsonElement(it).jsonObject }.getOrNull() }
            else -> null
        } ?: return true
        _commands.value = obj.mapNotNull { (key, value) ->
            val o = value as? JsonObject ?: return@mapNotNull null
            RemoteCommand(key, o.str("name") ?: key, o.str("command").orEmpty())
        }.sortedBy { it.name.lowercase() }
        return true
    }

    suspend fun run(key: String) = ctx.send(packet(PacketTypes.RUNCOMMAND_REQUEST) { put("key", key) })

    /** Opens the command editor on the PC. */
    suspend fun openSetup() = ctx.send(packet(PacketTypes.RUNCOMMAND_REQUEST) { put("setup", true) })

    companion object {
        const val ID = "runcommand"
        val spec = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.RUNCOMMAND, PacketTypes.RUNCOMMAND_OUTPUT),
            outgoing = setOf(PacketTypes.RUNCOMMAND_REQUEST),
            ::RunCommandPlugin,
        )
    }
}

// ---- PC audio outputs -------------------------------------------------------------------------

data class AudioSink(
    val name: String,
    val description: String,
    val volume: Int,
    val maxVolume: Int,
    val muted: Boolean,
    val isDefault: Boolean,
)

class SystemVolumePlugin(private val ctx: PluginContext) : Plugin {
    private val _sinks = MutableStateFlow<List<AudioSink>>(emptyList())
    val sinks: StateFlow<List<AudioSink>> = _sinks.asStateFlow()

    override suspend fun onStart() {
        ctx.send(packet(PacketTypes.SYSTEMVOLUME_REQUEST) { put("requestSinks", true) })
    }

    override suspend fun onLinkAdded() = onStart()

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        val list = packet.body["sinkList"]
        if (list is JsonArray) {
            _sinks.value = list.mapNotNull { el ->
                val o = el as? JsonObject ?: return@mapNotNull null
                val name = o.str("name") ?: return@mapNotNull null
                AudioSink(
                    name = name,
                    description = o.str("description") ?: name,
                    volume = o.int("volume") ?: 0,
                    maxVolume = (o.int("maxVolume") ?: 65536).coerceAtLeast(1),
                    muted = o.bool("muted") ?: false,
                    isDefault = o.bool("enabled") ?: false,
                )
            }
            return true
        }
        // Incremental update for one sink.
        val name = packet.string("name") ?: return true
        _sinks.value = _sinks.value.map { sink ->
            when {
                sink.name == name -> sink.copy(
                    volume = packet.int("volume") ?: sink.volume,
                    muted = packet.bool("muted") ?: sink.muted,
                    isDefault = packet.bool("enabled") ?: sink.isDefault,
                )
                // A new default implies the others aren't.
                packet.bool("enabled") == true -> sink.copy(isDefault = false)
                else -> sink
            }
        }
        return true
    }

    suspend fun setVolume(sink: String, volume: Int) = ctx.send(packet(PacketTypes.SYSTEMVOLUME_REQUEST) {
        put("name", sink); put("volume", volume)
    }).also { _sinks.value = _sinks.value.map { if (it.name == sink) it.copy(volume = volume) else it } }

    suspend fun setMuted(sink: String, muted: Boolean) = ctx.send(packet(PacketTypes.SYSTEMVOLUME_REQUEST) {
        put("name", sink); put("muted", muted)
    })

    suspend fun makeDefault(sink: String) = ctx.send(packet(PacketTypes.SYSTEMVOLUME_REQUEST) {
        put("name", sink); put("enabled", true)
    })

    companion object {
        const val ID = "systemvolume"
        val spec = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.SYSTEMVOLUME),
            outgoing = setOf(PacketTypes.SYSTEMVOLUME_REQUEST),
            ::SystemVolumePlugin,
        )
    }
}

// ---- touchpad & keyboard ----------------------------------------------------------------------

/** Special keys as numbered by KDE Connect's mousepad protocol. */
enum class SpecialKey(val code: Int) {
    BACKSPACE(1), TAB(2), LEFT(4), UP(5), RIGHT(6), DOWN(7), PAGE_UP(8), PAGE_DOWN(9),
    HOME(10), END(11), ENTER(12), DELETE(13), ESCAPE(14), PRINT_SCREEN(15),
    F1(21), F2(22), F3(23), F4(24), F5(25), F6(26), F7(27), F8(28), F9(29), F10(30), F11(31), F12(32),
}

class MousePadPlugin(private val ctx: PluginContext) : Plugin {
    private val _keyboardAvailable = MutableStateFlow(true)
    val keyboardAvailable: StateFlow<Boolean> = _keyboardAvailable.asStateFlow()

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.type == PacketTypes.MOUSEPAD_KEYBOARDSTATE) packet.bool("state")?.let { _keyboardAvailable.value = it }
        return true
    }

    suspend fun move(dx: Float, dy: Float) = send { put("dx", dx); put("dy", dy) }
    suspend fun scroll(dx: Float, dy: Float) = send { put("scroll", true); put("dx", dx); put("dy", dy) }
    suspend fun click() = send { put("singleclick", true) }
    suspend fun doubleClick() = send { put("doubleclick", true) }
    suspend fun rightClick() = send { put("rightclick", true) }
    suspend fun middleClick() = send { put("middleclick", true) }
    suspend fun press() = send { put("singlehold", true) }
    suspend fun release() = send { put("singlerelease", true) }

    suspend fun type(text: String) = send { put("key", text) }

    suspend fun key(key: SpecialKey, ctrl: Boolean = false, alt: Boolean = false, shift: Boolean = false, meta: Boolean = false) = send {
        put("specialKey", key.code)
        if (ctrl) put("ctrl", true)
        if (alt) put("alt", true)
        if (shift) put("shift", true)
        if (meta) put("super", true)
    }

    /** A character with modifiers, e.g. Ctrl+C. */
    suspend fun chord(char: String, ctrl: Boolean = false, alt: Boolean = false, shift: Boolean = false, meta: Boolean = false) = send {
        put("key", char)
        if (ctrl) put("ctrl", true)
        if (alt) put("alt", true)
        if (shift) put("shift", true)
        if (meta) put("super", true)
    }

    private suspend inline fun send(crossinline body: kotlinx.serialization.json.JsonObjectBuilder.() -> Unit) =
        ctx.send(packet(PacketTypes.MOUSEPAD_REQUEST) { body() })

    companion object {
        const val ID = "mousepad"
        val spec = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.MOUSEPAD_KEYBOARDSTATE, PacketTypes.MOUSEPAD_ECHO),
            outgoing = setOf(PacketTypes.MOUSEPAD_REQUEST),
            ::MousePadPlugin,
        )
    }
}

// ---- JSON helpers -----------------------------------------------------------------------------

internal fun JsonObject.str(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull
internal fun JsonObject.int(key: String): Int? = (this[key] as? JsonPrimitive)?.let { it.intOrNull ?: it.contentOrNull?.toIntOrNull() }
internal fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.let { it.booleanOrNull ?: it.contentOrNull?.toBooleanStrictOrNull() }
