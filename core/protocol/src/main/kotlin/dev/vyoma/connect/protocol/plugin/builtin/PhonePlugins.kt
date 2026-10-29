package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import dev.vyoma.connect.protocol.putStrings
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject

// ---- SMS ---------------------------------------------------------------------------------------

/** One text message, in the shape KDE Connect's SMS app expects (packet version 2). */
data class SmsMessage(
    val addresses: List<String>,
    val body: String,
    /** Unix millis. */
    val date: Long,
    /** Telephony.Sms.MESSAGE_TYPE_*: 1 inbox, 2 sent, ... */
    val type: Int,
    val read: Boolean,
    val threadId: Long,
    val uid: Long,
    val subscriptionId: Int,
    val multiTarget: Boolean = addresses.size > 1,
) {
    fun toJson(): JsonObject = buildJsonObject {
        put("addresses", JsonArray(addresses.map { buildJsonObject { put("address", it) } }))
        put("body", body)
        put("date", date)
        put("type", type)
        put("read", if (read) 1 else 0)
        put("thread_id", threadId)
        put("_id", uid)
        put("sub_id", subscriptionId)
        put("event", EVENT_TEXT or (if (multiTarget) EVENT_MULTI_TARGET else 0))
    }

    private companion object {
        const val EVENT_TEXT = 0x1
        const val EVENT_MULTI_TARGET = 0x2
    }
}

/** The phone's message store. Methods block; the plugin calls them off the device dispatcher. */
interface SmsStore {
    /** False until the user grants SMS access; the plugin then answers nothing. */
    fun available(): Boolean

    /** The newest message of every thread, newest thread first. */
    fun conversations(): Sequence<SmsMessage>

    /** Messages in [threadId] at or before [before] (millis), newest first, at most [limit]. */
    fun thread(threadId: Long, before: Long, limit: Int?): List<SmsMessage>

    /** Every message newer than [after] (millis), across threads. */
    fun newerThan(after: Long): List<SmsMessage>

    fun newestTimestamp(): Long

    fun send(addresses: List<String>, body: String, subscriptionId: Int)

    /** Emits whenever the store changes (new message received or sent). */
    val changes: Flow<Unit>
}

class SmsPlugin(private val ctx: PluginContext, private val store: SmsStore) : Plugin {
    // Only push live updates once this peer has shown interest, like upstream.
    @Volatile private var requested = false
    private val newestLock = Mutex()
    private var newest = Long.MIN_VALUE

    @OptIn(kotlinx.coroutines.FlowPreview::class)
    override suspend fun onStart() {
        ctx.scope.launch(Dispatchers.IO) {
            newestLock.withLock { newest = runCatching { store.newestTimestamp() }.getOrDefault(Long.MIN_VALUE) }
            store.changes.debounce(300).collect { if (requested) pushNew() }
        }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (!store.available()) {
            VLog.d(TAG, "${packet.type} ignored: no SMS permission")
            return true
        }
        when (packet.type) {
            PacketTypes.SMS_REQUEST_CONVERSATIONS -> ctx.scope.launch(Dispatchers.IO) {
                requested = true
                // One packet per thread so the PC can render progressively.
                for (m in store.conversations()) ctx.send(messages(listOf(m)))
            }
            PacketTypes.SMS_REQUEST_CONVERSATION -> {
                val thread = packet.long("threadID") ?: return true
                val start = packet.long("rangeStartTimestamp")?.takeIf { it >= 0 } ?: Long.MAX_VALUE
                val limit = packet.long("numberToRequest")?.takeIf { it >= 0 }?.toInt()
                ctx.scope.launch(Dispatchers.IO) {
                    requested = true
                    ctx.send(messages(store.thread(thread, start, limit)))
                }
            }
            PacketTypes.SMS_REQUEST -> {
                val body = packet.string("messageBody") ?: return true
                val addresses = (packet.body["addresses"] as? JsonArray)
                    ?.mapNotNull { ((it as? JsonObject)?.get("address") as? JsonPrimitive)?.contentOrNull }
                    ?: listOfNotNull(packet.string("phoneNumber"))
                if (addresses.isEmpty()) return true
                val sub = packet.int("subID") ?: -1
                ctx.scope.launch(Dispatchers.IO) {
                    runCatching { store.send(addresses, body, sub) }
                        .onFailure { VLog.w(TAG, "send failed: ${it.message}") }
                }
            }
            // MMS attachments aren't read yet; nothing to answer.
            PacketTypes.SMS_REQUEST_ATTACHMENT -> Unit
        }
        return true
    }

    private suspend fun pushNew() {
        val fresh = newestLock.withLock {
            val list = store.newerThan(newest)
            list.maxOfOrNull { it.date }?.let { if (it > newest) newest = it }
            list
        }
        if (fresh.isNotEmpty()) ctx.send(messages(fresh))
    }

    private fun messages(list: List<SmsMessage>) = packet(PacketTypes.SMS_MESSAGES) {
        put("messages", JsonArray(list.map { it.toJson() }))
        put("version", 2)
    }

    companion object {
        const val ID = "sms"
        private const val TAG = "Sms"

        fun spec(store: SmsStore) = PluginSpec(
            ID,
            incoming = setOf(
                PacketTypes.SMS_REQUEST, PacketTypes.SMS_REQUEST_CONVERSATIONS,
                PacketTypes.SMS_REQUEST_CONVERSATION, PacketTypes.SMS_REQUEST_ATTACHMENT,
            ),
            outgoing = setOf(PacketTypes.SMS_MESSAGES, PacketTypes.SMS_ATTACHMENT_FILE),
        ) { SmsPlugin(it, store) }
    }
}

// ---- calls -------------------------------------------------------------------------------------

sealed interface CallEvent {
    val number: String?
    val contactName: String?

    /** Contact photo as base64 JPEG/PNG, shown in the PC's call notification. */
    val photo: String?

    data class Ringing(override val number: String?, override val contactName: String?, override val photo: String?) : CallEvent
    data class Talking(override val number: String?, override val contactName: String?, override val photo: String?) : CallEvent
    data class Missed(override val number: String?, override val contactName: String?, override val photo: String? = null) : CallEvent

    /** The call in [previous] ended; the PC withdraws its notification. */
    data class Ended(val previous: CallEvent) : CallEvent {
        override val number get() = previous.number
        override val contactName get() = previous.contactName
        override val photo get() = previous.photo
    }
}

interface CallSource {
    val events: Flow<CallEvent>
    fun muteRinger()
}

class TelephonyPlugin(private val ctx: PluginContext, private val source: CallSource) : Plugin {
    override suspend fun onStart() {
        ctx.scope.launch {
            source.events.collect { event ->
                val (name, cancel) = when (event) {
                    is CallEvent.Ringing -> "ringing" to false
                    is CallEvent.Talking -> "talking" to false
                    is CallEvent.Missed -> "missedCall" to false
                    is CallEvent.Ended -> when (event.previous) {
                        is CallEvent.Talking -> "talking"
                        is CallEvent.Missed -> "missedCall"
                        else -> "ringing"
                    } to true
                }
                ctx.send(packet(PacketTypes.TELEPHONY) {
                    put("event", name)
                    event.number?.let { put("phoneNumber", it) }
                    put("contactName", event.contactName ?: event.number ?: "Unknown")
                    if (!cancel) event.photo?.let { put("phoneThumbnail", it) }
                    if (cancel) put("isCancel", true)
                })
            }
        }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.type == PacketTypes.TELEPHONY_REQUEST_MUTE) source.muteRinger()
        return true
    }

    companion object {
        const val ID = "telephony"

        fun spec(source: CallSource) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.TELEPHONY_REQUEST_MUTE),
            outgoing = setOf(PacketTypes.TELEPHONY),
        ) { TelephonyPlugin(it, source) }
    }
}

// ---- contacts ----------------------------------------------------------------------------------

/** The address book, keyed by Android's stable contact lookup key. Methods block. */
interface ContactStore {
    fun available(): Boolean

    /** Lookup key → last-modified millis. */
    fun timestamps(): Map<String, Long>

    /** Lookup key → vCard text (ending in END:VCARD). Unknown keys are left out. */
    fun vcards(keys: Collection<String>): Map<String, String>
}

class ContactsPlugin(private val ctx: PluginContext, private val store: ContactStore) : Plugin {
    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (!store.available()) return true
        when (packet.type) {
            PacketTypes.CONTACTS_REQUEST_ALL_UIDS_TIMESTAMPS -> ctx.scope.launch(Dispatchers.IO) {
                val all = store.timestamps()
                ctx.send(packet(PacketTypes.CONTACTS_RESPONSE_UIDS_TIMESTAMPS) {
                    for ((key, ts) in all) put(key, ts)
                    putStrings("uids", all.keys)
                })
            }
            PacketTypes.CONTACTS_REQUEST_VCARDS_BY_UIDS -> {
                val wanted = packet.stringList("uids").distinct()
                if (wanted.isEmpty()) return true
                ctx.scope.launch(Dispatchers.IO) {
                    val stamps = store.timestamps()
                    // Chunk so a big address book doesn't make one enormous packet.
                    for (chunk in wanted.chunked(VCARD_CHUNK)) {
                        val cards = store.vcards(chunk)
                        ctx.send(packet(PacketTypes.CONTACTS_RESPONSE_VCARDS) {
                            for ((key, card) in cards) put(key, withMetadata(card, key, stamps[key] ?: 0))
                            putStrings("uids", cards.keys)
                        })
                    }
                }
            }
        }
        return true
    }

    /** The PC uses these lines to tie the card back to this phone and to skip unchanged ones. */
    private fun withMetadata(card: String, key: String, timestamp: Long): String {
        val head = mergeVCards(card).removeSuffix(VCARD_END).trimEnd('\r', '\n')
        return "$head\nX-KDECONNECT-ID-DEV-${ctx.device.id}:$key\nREV:$timestamp\n$VCARD_END"
    }

    companion object {
        const val ID = "contacts"
        private const val VCARD_END = "END:VCARD"
        private const val VCARD_CHUNK = 100

        /**
         * Android returns one vCard per linked raw contact (Google, SIM, WhatsApp...) for a single
         * person. The PC only reads the first, so fold the others' extra lines (numbers, emails...)
         * into it, skipping duplicates and the per-card header lines.
         */
        fun mergeVCards(text: String): String {
            val cards = text.replace("\r\n", "\n").split("BEGIN:VCARD").map { it.trim() }.filter { it.isNotEmpty() }
                .map { body -> unfold(body.removeSuffix(VCARD_END).trim()).filter { it.isNotBlank() } }
            if (cards.isEmpty()) return "BEGIN:VCARD\nVERSION:2.1\n$VCARD_END"
            val single = setOf("VERSION", "N", "FN")
            val seen = HashSet<String>()
            val lines = ArrayList<String>()
            cards.forEachIndexed { i, card ->
                for (line in card) {
                    if (i > 0 && propertyName(line) in single) continue
                    if (seen.add(identity(line))) lines.add(line)
                }
            }
            return (listOf("BEGIN:VCARD") + lines + VCARD_END).joinToString("\n")
        }

        /** vCard continuation lines start with a space; keep each property on one logical line. */
        private fun unfold(body: String): List<String> {
            val out = ArrayList<String>()
            for (line in body.lines()) {
                if ((line.startsWith(" ") || line.startsWith("\t")) && out.isNotEmpty()) out[out.lastIndex] += "\n" + line
                else out.add(line)
            }
            return out
        }

        private fun propertyName(line: String) = line.substringBefore(':').substringBefore(';').uppercase()

        /** Same number written differently by two accounts ("+91 98…" vs "098…") counts once. */
        private fun identity(line: String): String {
            val name = propertyName(line)
            val value = line.substringAfter(':')
            return when (name) {
                "TEL" -> "TEL:" + value.filter(Char::isDigit).takeLast(10)
                "EMAIL" -> "EMAIL:" + value.trim().lowercase()
                else -> line
            }
        }

        fun spec(store: ContactStore) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.CONTACTS_REQUEST_ALL_UIDS_TIMESTAMPS, PacketTypes.CONTACTS_REQUEST_VCARDS_BY_UIDS),
            outgoing = setOf(PacketTypes.CONTACTS_RESPONSE_UIDS_TIMESTAMPS, PacketTypes.CONTACTS_RESPONSE_VCARDS),
        ) { ContactsPlugin(it, store) }
    }
}

// ---- mobile signal -----------------------------------------------------------------------------

/** One SIM's signal: [level] 0..4 and a short network name ("LTE", "5G", ...). */
data class SignalState(val networkType: String, val level: Int)

class ConnectivityReportPlugin(private val ctx: PluginContext, private val signal: StateFlow<Map<Int, SignalState>>) : Plugin {
    override suspend fun onStart() {
        ctx.scope.launch {
            signal.collect { if (it.isNotEmpty()) send(it) }
        }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.type == PacketTypes.CONNECTIVITY_REPORT_REQUEST) signal.value.takeIf { it.isNotEmpty() }?.let { send(it) }
        return true
    }

    private suspend fun send(states: Map<Int, SignalState>) = ctx.send(packet(PacketTypes.CONNECTIVITY_REPORT) {
        putJsonObject("signalStrengths") {
            for ((sub, s) in states) putJsonObject(sub.toString()) {
                put("networkType", s.networkType)
                put("signalStrength", s.level)
            }
        }
    })

    companion object {
        const val ID = "connectivity_report"

        fun spec(signal: StateFlow<Map<Int, SignalState>>) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.CONNECTIVITY_REPORT_REQUEST),
            outgoing = setOf(PacketTypes.CONNECTIVITY_REPORT),
        ) { ConnectivityReportPlugin(it, signal) }
    }
}
