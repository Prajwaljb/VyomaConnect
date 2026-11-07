package dev.vyoma.connect.protocol

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/**
 * A KDE Connect packet: one JSON object per line.
 *
 * `payloadSize`/`payloadTransferInfo` describe an optional binary payload that travels
 * over a separate TLS socket (see [PayloadTransfer]).
 */
class NetworkPacket(
    val type: String,
    val body: JsonObject = EMPTY,
    val id: Long = System.currentTimeMillis(),
    val payloadSize: Long = 0,
    val payloadTransferInfo: JsonObject? = null,
) {
    val hasPayload: Boolean
        get() = !payloadTransferInfo.isNullOrEmpty() // an empty file still has somewhere to fetch it from

    fun withPayload(size: Long, transferInfo: JsonObject) =
        NetworkPacket(type, body, id, size, transferInfo)

    fun serialize(): String = buildJsonObject {
        put("id", id)
        put("type", type)
        put("body", body)
        if (payloadTransferInfo != null) {
            put("payloadSize", payloadSize)
            put("payloadTransferInfo", payloadTransferInfo)
        }
    }.toString() + "\n"

    // Body accessors. Peers are inconsistent about number-vs-string, so be lenient.
    fun string(key: String): String? = body.prim(key)?.contentOrNull
    fun int(key: String): Int? = body.prim(key)?.let { it.intOrNull ?: it.contentOrNull?.toIntOrNull() }
    fun long(key: String): Long? = body.prim(key)?.let { it.longOrNull ?: it.contentOrNull?.toLongOrNull() }
    fun double(key: String): Double? = body.prim(key)?.doubleOrNull
    fun bool(key: String): Boolean? = body.prim(key)?.let { it.booleanOrNull ?: it.contentOrNull?.toBooleanStrictOrNull() }
    fun stringList(key: String): List<String> =
        (body[key] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }.orEmpty()
    fun obj(key: String): JsonObject? = body[key] as? JsonObject
    fun has(key: String): Boolean = body[key].let { it != null && it !is JsonNull }

    override fun toString(): String = "NetworkPacket($type, ${body.toString().take(200)})"

    companion object {
        val EMPTY = JsonObject(emptyMap())

        private val json = Json { ignoreUnknownKeys = true }

        /** Parses one packet line; throws [ProtocolException] on malformed input. */
        fun parse(line: String): NetworkPacket {
            val root = try {
                json.parseToJsonElement(line).jsonObject
            } catch (e: Exception) {
                throw ProtocolException("Malformed packet JSON", e)
            }
            val type = (root["type"] as? JsonPrimitive)?.contentOrNull
                ?: throw ProtocolException("Packet without type")
            val id = (root["id"] as? JsonPrimitive)?.let { it.longOrNull ?: it.contentOrNull?.toLongOrNull() } ?: 0L
            return NetworkPacket(
                type = type,
                body = root["body"] as? JsonObject ?: EMPTY,
                id = id,
                payloadSize = (root["payloadSize"] as? JsonPrimitive)?.longOrNull ?: 0L,
                payloadTransferInfo = root["payloadTransferInfo"] as? JsonObject,
            )
        }
    }
}

private fun JsonObject.prim(key: String): JsonPrimitive? = this[key] as? JsonPrimitive

inline fun packet(type: String, body: JsonObjectBuilder.() -> Unit = {}) =
    NetworkPacket(type, buildJsonObject(body))

fun JsonObjectBuilder.putStrings(key: String, values: Collection<String>) =
    put(key, JsonArray(values.map(::JsonPrimitive)))

fun JsonObjectBuilder.putIfNotNull(key: String, value: JsonElement?) {
    if (value != null) put(key, value)
}

class ProtocolException(message: String, cause: Throwable? = null) : Exception(message, cause)
