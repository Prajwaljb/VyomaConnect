package dev.vyoma.connect.protocol

import java.util.UUID
import kotlinx.serialization.json.put

enum class DeviceType(val wire: String) {
    DESKTOP("desktop"),
    LAPTOP("laptop"),
    PHONE("phone"),
    TABLET("tablet"),
    TV("tv");

    companion object {
        fun fromWire(value: String?): DeviceType = entries.firstOrNull { it.wire == value } ?: DESKTOP
    }
}

/** The contents of a `kdeconnect.identity` packet. */
data class DeviceIdentity(
    val deviceId: String,
    val deviceName: String,
    val deviceType: DeviceType,
    val protocolVersion: Int,
    val incomingCapabilities: Set<String>,
    val outgoingCapabilities: Set<String>,
    val tcpPort: Int? = null,
) {
    fun toPacket(tcpPort: Int? = this.tcpPort): NetworkPacket = packet(PacketTypes.IDENTITY) {
        put("deviceId", deviceId)
        put("deviceName", deviceName)
        put("deviceType", deviceType.wire)
        put("protocolVersion", protocolVersion)
        putStrings("incomingCapabilities", incomingCapabilities.sorted())
        putStrings("outgoingCapabilities", outgoingCapabilities.sorted())
        if (tcpPort != null) put("tcpPort", tcpPort)
    }

    companion object {
        const val PROTOCOL_VERSION = 8
        const val MIN_PROTOCOL_VERSION = 7

        // KDE Connect v8 generates [a-zA-Z0-9_]{32,38}; older peers used shorter hex ids and dashes.
        private val validId = Regex("^[a-zA-Z0-9_-]{16,64}$")
        private const val FORBIDDEN_NAME_CHARS = "\"',;:.!?()[]<>"
        private const val MAX_NAME_LENGTH = 32

        fun newDeviceId(): String = UUID.randomUUID().toString().replace('-', '_')

        fun isValidDeviceId(id: String) = validId.matches(id)

        fun sanitizeName(name: String): String =
            name.filterNot { it in FORBIDDEN_NAME_CHARS }.trim().take(MAX_NAME_LENGTH).ifEmpty { "Unnamed" }

        fun fromPacket(packet: NetworkPacket): DeviceIdentity {
            if (packet.type != PacketTypes.IDENTITY) throw ProtocolException("Expected identity, got ${packet.type}")
            val id = packet.string("deviceId") ?: throw ProtocolException("Identity without deviceId")
            if (!isValidDeviceId(id)) throw ProtocolException("Invalid deviceId '$id'")
            return DeviceIdentity(
                deviceId = id,
                deviceName = sanitizeName(packet.string("deviceName").orEmpty()),
                deviceType = DeviceType.fromWire(packet.string("deviceType")),
                protocolVersion = packet.int("protocolVersion") ?: 0,
                incomingCapabilities = packet.stringList("incomingCapabilities").toSet(),
                outgoingCapabilities = packet.stringList("outgoingCapabilities").toSet(),
                tcpPort = packet.int("tcpPort"),
            )
        }
    }
}
