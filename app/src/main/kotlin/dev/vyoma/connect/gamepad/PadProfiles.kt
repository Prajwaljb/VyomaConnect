package dev.vyoma.connect.gamepad

import android.content.Context
import dev.vyoma.connect.protocol.plugin.builtin.PadKind
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.floatOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import java.util.UUID

/** The controller's parts that can be moved and resized in the layout editor. */
enum class PadControl(val label: String) {
    LT("LT"), LB("LB"), RB("RB"), RT("RT"), VIEW("View"), MENU("Menu"), GUIDE("Guide"),
    FACE("Buttons"), DPAD("D-pad"), LSTICK("Left stick"), RSTICK("Right stick"),
}

/** Where a control sits, as a fraction of the screen, and how big it is next to its usual size. */
data class Place(val x: Float, val y: Float, val scale: Float = 1f)

/** When the gyro aims. */
enum class GyroMode(val label: String) { OFF("Off"), ALWAYS("Always"), AIMING("LT held") }

/** What the gyro moves: a mouse (what PC games aim with) or the right stick (for games that take only a pad). */
enum class GyroTarget(val label: String) { MOUSE("Mouse"), STICK("Right stick") }

/** One game's controller: the pad the PC makes, gyro, triggers and the layout. */
data class PadProfile(
    val id: String = UUID.randomUUID().toString(),
    val name: String = "Default",
    val kind: PadKind = PadKind.XBOX,
    val gyro: GyroMode = GyroMode.OFF,
    val gyroTarget: GyroTarget = GyroTarget.MOUSE,
    /** 0.25..3; 1 = about 10 px per degree turned, or the stick full at 120°/s. */
    val gyroSpeed: Float = 1f,
    val invertGyroY: Boolean = false,
    /** Triggers press as deep as the thumb slides down them, instead of all the way at a touch. */
    val analogTriggers: Boolean = false,
    val places: Map<PadControl, Place> = emptyMap(),
)

/** The saved profiles (one per game, named by the user) and which one is in use. */
class PadProfiles(context: Context) {
    private val prefs = context.getSharedPreferences("gamepad", Context.MODE_PRIVATE)
    private val _all = MutableStateFlow(load())
    val all: StateFlow<List<PadProfile>> = _all.asStateFlow()
    private val _current = MutableStateFlow(prefs.getString("current", null)?.takeIf { id -> _all.value.any { it.id == id } } ?: _all.value.first().id)
    val currentId: StateFlow<String> = _current.asStateFlow()

    fun current(): PadProfile = _all.value.firstOrNull { it.id == _current.value } ?: _all.value.first()

    fun select(id: String) {
        _current.value = id
        prefs.edit().putString("current", id).apply()
    }

    /** Saves [p] (replacing the one with its id, or adding it). */
    fun save(p: PadProfile) {
        val list = _all.value.toMutableList()
        val i = list.indexOfFirst { it.id == p.id }
        if (i >= 0) list[i] = p else list += p
        store(list)
    }

    /** A new profile like the current one (its layout and settings), under [name], made current. */
    fun create(name: String): PadProfile {
        val p = current().copy(id = UUID.randomUUID().toString(), name = name)
        save(p)
        select(p.id)
        return p
    }

    /** Removes [id]; the last profile always stays. */
    fun delete(id: String) {
        val list = _all.value.filterNot { it.id == id }
        if (list.isEmpty()) return
        store(list)
        if (_current.value == id) select(list.first().id)
    }

    private fun store(list: List<PadProfile>) {
        _all.value = list
        prefs.edit().putString("profiles", buildJsonArray { list.forEach { add(encode(it)) } }.toString()).apply()
    }

    private fun load(): List<PadProfile> {
        val raw = prefs.getString("profiles", null) ?: return listOf(PadProfile())
        val list = runCatching { (Json.parseToJsonElement(raw) as JsonArray).mapNotNull { runCatching { decode(it.jsonObject) }.getOrNull() } }.getOrNull()
        return list?.takeIf { it.isNotEmpty() } ?: listOf(PadProfile())
    }

    private fun encode(p: PadProfile) = buildJsonObject {
        put("id", p.id)
        put("name", p.name)
        put("kind", p.kind.wire)
        put("gyro", p.gyro.name)
        put("gyroTarget", p.gyroTarget.name)
        put("gyroSpeed", p.gyroSpeed)
        put("invertGyroY", p.invertGyroY)
        put("analogTriggers", p.analogTriggers)
        put("places", buildJsonObject {
            for ((c, pl) in p.places) put(c.name, buildJsonObject { put("x", pl.x); put("y", pl.y); put("scale", pl.scale) })
        })
    }

    private fun decode(o: JsonObject): PadProfile {
        fun s(k: String) = o[k]?.jsonPrimitive?.content
        val places = (o["places"] as? JsonObject).orEmpty().mapNotNull { (k, v) ->
            val c = runCatching { PadControl.valueOf(k) }.getOrNull() ?: return@mapNotNull null
            val p = v.jsonObject
            val x = p["x"]?.jsonPrimitive?.floatOrNull ?: return@mapNotNull null
            val y = p["y"]?.jsonPrimitive?.floatOrNull ?: return@mapNotNull null
            c to Place(x, y, p["scale"]?.jsonPrimitive?.floatOrNull ?: 1f)
        }.toMap()
        return PadProfile(
            id = s("id") ?: UUID.randomUUID().toString(),
            name = s("name") ?: "Default",
            kind = PadKind.entries.firstOrNull { it.wire == s("kind") } ?: PadKind.XBOX,
            gyro = GyroMode.entries.firstOrNull { it.name == s("gyro") } ?: GyroMode.OFF,
            gyroTarget = GyroTarget.entries.firstOrNull { it.name == s("gyroTarget") } ?: GyroTarget.MOUSE,
            gyroSpeed = o["gyroSpeed"]?.jsonPrimitive?.floatOrNull ?: 1f,
            invertGyroY = o["invertGyroY"]?.jsonPrimitive?.booleanOrNull ?: false,
            analogTriggers = o["analogTriggers"]?.jsonPrimitive?.booleanOrNull ?: false,
            places = places,
        )
    }
}
