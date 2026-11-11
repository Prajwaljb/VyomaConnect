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
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.withTimeoutOrNull
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
    /** From the phone's texting app: what it says reaches a PC only while its texts are open to it ([TextsGuard.lets]). */
    val texts: Boolean = false,
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

    /** Whether notification key or reply id [id] belongs to the texting app, asked of the platform itself. */
    fun isText(id: String): Boolean = false
}

class NotificationsPlugin(private val ctx: PluginContext, private val source: NotificationSource, private val guard: TextsGuard = TextsGuard.None) : Plugin {
    /** The texting app's notifications (id → reply id, "" for none): answered only while this PC may have the texts. */
    private val textIds = java.util.concurrent.ConcurrentHashMap<String, String>()
    // The source decides as well: this instance only knows the ones it sent (a new link knows none).
    private fun isText(id: String) = textIds.containsKey(id) || textIds.containsValue(id) || source.isText(id)
    private fun textsLocked() = !guard.lets(ctx.device.id, ctx.device.canAskForTexts)

    override suspend fun onStart() {
        ctx.scope.launch {
            source.events.collect { event ->
                when (event) {
                    is NotificationEvent.Posted -> send(event.notification)
                    is NotificationEvent.Removed -> {
                        textIds.remove(event.id)
                        ctx.send(packet(PacketTypes.NOTIFICATION) {
                            put("id", event.id)
                            put("isCancel", true)
                        })
                    }
                }
            }
        }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        when (packet.type) {
            PacketTypes.NOTIFICATION_REQUEST -> {
                // Off the device's dispatcher: this renders every notification's icon.
                if (packet.bool("request") == true) ctx.scope.launch(Dispatchers.IO) { source.active().forEach { send(it.copy(silent = true)) } }
                packet.string("cancel")?.let(source::dismiss)
            }
            PacketTypes.NOTIFICATION_REPLY -> {
                val id = packet.string("requestReplyId") ?: return true
                if (isText(id) && textsLocked()) return refused(packet)
                source.reply(id, packet.string("message").orEmpty())
            }
            PacketTypes.NOTIFICATION_ACTION -> {
                val id = packet.string("key") ?: return true
                if (isText(id) && textsLocked()) return refused(packet)
                source.action(id, packet.string("action") ?: return true)
            }
        }
        return true
    }

    /** Icons a PC already has hold a port for the whole wait: a burst mustn't take them all from file transfers. */
    private val iconPorts = Semaphore(MAX_ICONS_AT_ONCE)

    private fun refused(packet: NetworkPacket): Boolean {
        dev.vyoma.connect.protocol.VLog.i("Notifications", "${packet.type} refused: texts are locked for this PC")
        return true
    }

    private fun send(original: PhoneNotification) {
        if (original.texts) textIds[original.id] = original.replyId.orEmpty()
        // Texts locked for this PC: who wrote, not what (no reply or actions either, which would send or reveal it).
        val n = if (original.texts && textsLocked()) original.copy(
            title = original.appName,
            text = if (original.title.isBlank()) "New text" else "New text from ${original.title}",
            actions = emptyList(),
            replyId = null,
        ) else original
        fun body(withIcon: Boolean) = packet(PacketTypes.NOTIFICATION) {
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
            if (withIcon) n.iconHash?.let { put("payloadHash", it) }
        }
        val icon = n.icon
        ctx.scope.launch(Dispatchers.IO) {
            val hasPort = icon != null && n.iconHash != null && ctx.device.canCarryPayloads &&
                withTimeoutOrNull(ICON_PORT_WAIT_MS) { iconPorts.acquire() } != null
            if (!hasPort) {
                ctx.send(body(withIcon = false)) // late or without its icon, never lost
                return@launch
            }
            // The PC skips the download when it has this icon cached, so don't hold a port for long.
            // A timeout is harmless: the packet itself was already delivered.
            try {
                runCatching {
                    ctx.device.sendWithPayload(body(withIcon = true), PayloadSource(icon!!.size.toLong()) { ByteArrayInputStream(icon) }, timeoutMs = ICON_ACCEPT_MS)
                }
            } finally {
                iconPorts.release()
            }
        }
    }

    companion object {
        const val ID = "notifications"
        private const val ICON_ACCEPT_MS = 4_000
        private const val MAX_ICONS_AT_ONCE = 8
        private const val ICON_PORT_WAIT_MS = 1_000L

        fun spec(source: NotificationSource, guard: TextsGuard = TextsGuard.None) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.NOTIFICATION_REQUEST, PacketTypes.NOTIFICATION_REPLY, PacketTypes.NOTIFICATION_ACTION),
            outgoing = setOf(PacketTypes.NOTIFICATION),
        ) { NotificationsPlugin(it, source, guard) }
    }
}
