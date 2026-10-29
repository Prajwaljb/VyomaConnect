package dev.vyoma.connect.protocol.device

import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.crypto.Certificates
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import java.io.File
import java.security.cert.X509Certificate
import java.util.Base64

/** A paired peer: what we pinned at pairing time, so it can be listed while offline. */
data class TrustedDevice(
    val deviceId: String,
    val name: String,
    val type: DeviceType,
    val certificate: X509Certificate,
)

interface TrustStore {
    fun all(): List<TrustedDevice>
    fun get(deviceId: String): TrustedDevice?
    fun put(device: TrustedDevice)
    fun remove(deviceId: String)
}

/** One small JSON file; paired-device lists are tiny and rarely written. */
class FileTrustStore(private val file: File) : TrustStore {
    private val lock = Any()
    private var cache: MutableMap<String, TrustedDevice>? = null

    override fun all(): List<TrustedDevice> = synchronized(lock) { loaded().values.toList() }
    override fun get(deviceId: String): TrustedDevice? = synchronized(lock) { loaded()[deviceId] }

    override fun put(device: TrustedDevice) = synchronized(lock) {
        loaded()[device.deviceId] = device
        persist()
    }

    override fun remove(deviceId: String) = synchronized(lock) {
        if (loaded().remove(deviceId) != null) persist()
    }

    private fun loaded(): MutableMap<String, TrustedDevice> = cache ?: read().also { cache = it }

    private fun read(): MutableMap<String, TrustedDevice> {
        if (!file.exists()) return mutableMapOf()
        return runCatching {
            Json.parseToJsonElement(file.readText()).jsonObject.mapNotNull { (id, value) ->
                val o = value as? JsonObject ?: return@mapNotNull null
                val cert = o.str("cert")?.let { Certificates.decode(Base64.getDecoder().decode(it)) } ?: return@mapNotNull null
                id to TrustedDevice(id, o.str("name").orEmpty(), DeviceType.fromWire(o.str("type")), cert)
            }.toMap(LinkedHashMap())
        }.getOrElse { mutableMapOf() }
    }

    private fun persist() {
        val json = buildJsonObject {
            for (d in loaded().values) {
                put(d.deviceId, buildJsonObject {
                    put("name", d.name)
                    put("type", d.type.wire)
                    put("cert", Base64.getEncoder().encodeToString(d.certificate.encoded))
                })
            }
        }
        file.parentFile?.mkdirs()
        val tmp = File(file.parentFile, file.name + ".tmp")
        tmp.writeText(json.toString())
        if (!tmp.renameTo(file)) {
            file.delete()
            tmp.renameTo(file)
        }
    }

    private fun JsonObject.str(key: String) = (this[key] as? JsonPrimitive)?.contentOrNull
}
