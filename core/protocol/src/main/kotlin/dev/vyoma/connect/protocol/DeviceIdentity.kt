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
    /** Set when this peer is the Vyoma companion of another device (the PC's KDE Connect daemon). */
    val companionOf: String? = null,
) {
    /**
     * The PC whose Vyoma features (stats, live screen, files…) this device serves: its PC for a
     * companion, itself for Vyoma's own desktop app, which needs no companion; null for stock peers.
     */
    val featuresOf: String? get() = companionOf ?: deviceId.takeIf { OWN_FEATURES in incomingCapabilities }

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

        /** What a PC that serves Vyoma's features itself takes. */
        private const val OWN_FEATURES = "vyoma.stats.request"

        // KDE Connect v8 generates [a-zA-Z0-9_]{32,38}; older peers used shorter hex ids and dashes.
        private val validId = Regex("^[a-zA-Z0-9_-]{16,64}$")
        private const val FORBIDDEN_NAME_CHARS = "\"',;:.!?()[]<>"
        private const val MAX_NAME_LENGTH = 32

        fun newDeviceId(): String = UUID.randomUUID().toString().replace('-', '_')

        fun isValidDeviceId(id: String) = validId.matches(id)

        fun sanitizeName(name: String): String =
            name.filterNot { it in FORBIDDEN_NAME_CHARS }.trim().take(MAX_NAME_LENGTH).ifEmpty { "Unnamed" }

        /**
         * Whether a hello before TLS says it's a Vyoma companion. Only for telling one from another
         * KDE Connect app on this phone; TLS and the pinned certificate decide if it gets anywhere.
         */
        fun helloFromCompanion(packet: NetworkPacket): Boolean =
            packet.string("vyomaHost") != null || packet.bool("vyomaCompanion") == true

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
                companionOf = packet.string("vyomaHost")?.takeIf(::isValidDeviceId),
            )
        }
    }
}
