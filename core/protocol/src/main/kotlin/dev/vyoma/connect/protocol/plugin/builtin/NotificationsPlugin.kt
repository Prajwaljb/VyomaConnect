package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.link.PayloadSource
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import dev.vyoma.connect.protocol.putStrings
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.put
import java.io.ByteArrayInputStream

/** A notification on this phone, flattened for the wire. */
data class PhoneNotification(
    val id: String,
    val appName: String,
    val title: String,
    val text: String,
    val time: Long,
    val clearable: Boolean,
    val actions: List<String> = emptyList(),
    /** Set when the notification has an inline reply; the PC sends replies back with this id. */
    val replyId: String? = null,
    /** App icon as PNG, and its hash so the PC can cache it. */
    val icon: ByteArray? = null,
    val iconHash: String? = null,
    /** True for updates to a notification the user already saw: the PC shouldn't pop it again. */
    val silent: Boolean = false,
)

sealed interface NotificationEvent {
    data class Posted(val notification: PhoneNotification) : NotificationEvent
    data class Removed(val id: String) : NotificationEvent
}

/** The platform's notification listener, as the plugin sees it. */
interface NotificationSource {
    val events: SharedFlow<NotificationEvent>
    fun active(): List<PhoneNotification>
    fun dismiss(id: String)
    fun reply(replyId: String, message: String)
    fun action(id: String, action: String)
}

class NotificationsPlugin(private val ctx: PluginContext, private val source: NotificationSource) : Plugin {
    override suspend fun onStart() {
        ctx.scope.launch {
            source.events.collect { event ->
                when (event) {
                    is NotificationEvent.Posted -> send(event.notification)
                    is NotificationEvent.Removed -> ctx.send(packet(PacketTypes.NOTIFICATION) {
                        put("id", event.id)
                        put("isCancel", true)
                    })
                }
            }
        }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        when (packet.type) {
            PacketTypes.NOTIFICATION_REQUEST -> {
                if (packet.bool("request") == true) source.active().forEach { send(it.copy(silent = true)) }
                packet.string("cancel")?.let(source::dismiss)
            }
            PacketTypes.NOTIFICATION_REPLY -> {
                val id = packet.string("requestReplyId") ?: return true
                source.reply(id, packet.string("message").orEmpty())
            }
            PacketTypes.NOTIFICATION_ACTION -> {
                val id = packet.string("key") ?: return true
                source.action(id, packet.string("action") ?: return true)
            }
        }
        return true
    }

    private fun send(n: PhoneNotification) {
        val body = packet(PacketTypes.NOTIFICATION) {
            put("id", n.id)
            put("appName", n.appName)
            put("title", n.title)
            put("text", n.text)
            put("ticker", listOf(n.title, n.text).filter { it.isNotBlank() }.joinToString(": "))
            put("isClearable", n.clearable)
            put("time", n.time.toString())
            put("silent", n.silent)
            if (n.actions.isNotEmpty()) putStrings("actions", n.actions)
            n.replyId?.let { put("requestReplyId", it) }
            n.iconHash?.let { put("payloadHash", it) }
        }
        val icon = n.icon
        ctx.scope.launch(Dispatchers.IO) {
            if (icon == null || n.iconHash == null || !ctx.device.canCarryPayloads) {
                ctx.send(body)
                return@launch
            }
            // The PC skips the download when it has this icon cached, so don't hold a port for long.
            // A timeout is harmless: the packet itself was already delivered.
            runCatching {
                ctx.device.sendWithPayload(body, PayloadSource(icon.size.toLong()) { ByteArrayInputStream(icon) }, timeoutMs = ICON_ACCEPT_MS)
            }
        }
    }

    companion object {
        const val ID = "notifications"
        private const val ICON_ACCEPT_MS = 4_000

        fun spec(source: NotificationSource) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.NOTIFICATION_REQUEST, PacketTypes.NOTIFICATION_REPLY, PacketTypes.NOTIFICATION_ACTION),
            outgoing = setOf(PacketTypes.NOTIFICATION),
        ) { NotificationsPlugin(it, source) }
    }
}
