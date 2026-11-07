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
    /** Where we last reached it, so we can dial it directly when broadcasts don't get through. */
    val lastAddress: String? = null,
    val lastPort: Int? = null,
    /** For a Vyoma companion: the PC it belongs to, so it stays folded into that PC while offline. */
    val companionOf: String? = null,
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
    private val backup = File(file.path + ".bak")
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
        // A file that can't be read falls back to the last good one: an empty answer here gets saved
        // over it with the next write, and every pairing would be gone.
        return read(file) ?: read(backup) ?: mutableMapOf()
    }

    private fun read(from: File): MutableMap<String, TrustedDevice>? {
        if (!from.exists()) return null
        return runCatching {
            Json.parseToJsonElement(from.readText()).jsonObject.mapNotNull { (id, value) ->
                runCatching {
                    val o = value as? JsonObject ?: return@mapNotNull null
                    val cert = o.str("cert")?.let { Certificates.decode(Base64.getDecoder().decode(it)) } ?: return@mapNotNull null
                    id to TrustedDevice(
                        id, o.str("name").orEmpty(), DeviceType.fromWire(o.str("type")), cert,
                        lastAddress = o.str("address"), lastPort = o.str("port")?.toIntOrNull(),
                        companionOf = o.str("companionOf"),
                    )
                }.getOrNull()
            }.toMap(LinkedHashMap())
        }.getOrNull()
    }

    private fun persist() {
        val json = buildJsonObject {
            for (d in loaded().values) {
                put(d.deviceId, buildJsonObject {
                    put("name", d.name)
                    put("type", d.type.wire)
                    put("cert", Base64.getEncoder().encodeToString(d.certificate.encoded))
                    d.lastAddress?.let { put("address", it) }
                    d.lastPort?.let { put("port", it) }
                    d.companionOf?.let { put("companionOf", it) }
                })
            }
        }
        file.parentFile?.mkdirs()
        val tmp = File(file.parentFile, file.name + ".tmp")
        // Written on every link-up, so a power cut can land here: on disk before it replaces the file.
        java.io.FileOutputStream(tmp).use {
            it.write(json.toString().toByteArray())
            it.fd.sync()
        }
        if (file.exists() && read(file) != null) file.copyTo(backup, overwrite = true)
        if (!tmp.renameTo(file)) {
            file.delete()
            tmp.renameTo(file)
        }
    }

    private fun JsonObject.str(key: String) = (this[key] as? JsonPrimitive)?.contentOrNull
}
