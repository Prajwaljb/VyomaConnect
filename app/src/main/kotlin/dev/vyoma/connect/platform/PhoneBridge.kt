package dev.vyoma.connect.platform

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.media.AudioManager
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.ContactsContract
import android.provider.Telephony
import android.telephony.ServiceState
import android.telephony.SignalStrength
import android.telephony.SmsManager
import android.telephony.SubscriptionManager
import android.telephony.TelephonyCallback
import android.telephony.TelephonyDisplayInfo
import android.telephony.TelephonyManager
import android.util.Base64
import androidx.core.content.ContextCompat
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.CallEvent
import dev.vyoma.connect.protocol.plugin.builtin.CallSource
import dev.vyoma.connect.protocol.plugin.builtin.ContactStore
import dev.vyoma.connect.protocol.plugin.builtin.SignalState
import dev.vyoma.connect.protocol.plugin.builtin.SmsMessage
import dev.vyoma.connect.protocol.plugin.builtin.SmsStore
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.callbackFlow
import java.util.concurrent.Executor

/** The runtime permissions behind "Calls & messages on PC". */
object PhonePermissions {
    val all = arrayOf(
        Manifest.permission.READ_SMS,
        Manifest.permission.SEND_SMS,
        Manifest.permission.READ_PHONE_STATE,
        Manifest.permission.READ_CALL_LOG,
        Manifest.permission.READ_CONTACTS,
    )

    fun granted(context: Context, permission: String) =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

    fun allGranted(context: Context) = all.all { granted(context, it) }
}

// ---- SMS ---------------------------------------------------------------------------------------

/** Plain SMS from the system provider. MMS isn't read yet. */
class AndroidSms(private val context: Context) : SmsStore {
    private val resolver get() = context.contentResolver

    override fun available() = PhonePermissions.granted(context, Manifest.permission.READ_SMS)

    override fun conversations(): Sequence<SmsMessage> = sequence {
        // One pass, newest first; the first row seen for each thread is its latest message.
        val seen = HashSet<Long>()
        readRows(null, null) { rows -> for (m in rows) if (seen.add(m.threadId)) yield(m) }
    }

    override fun thread(threadId: Long, before: Long, limit: Int?): List<SmsMessage> =
        query("${Telephony.Sms.THREAD_ID} = ? AND ${Telephony.Sms.DATE} <= ?", arrayOf(threadId.toString(), before.toString()), limit)

    override fun newerThan(after: Long): List<SmsMessage> =
        query("${Telephony.Sms.DATE} > ?", arrayOf(after.toString()), null)

    override fun newestTimestamp(): Long = query(null, null, 1).firstOrNull()?.date ?: Long.MIN_VALUE

    override fun send(addresses: List<String>, body: String, subscriptionId: Int) {
        check(PhonePermissions.granted(context, Manifest.permission.SEND_SMS)) { "SEND_SMS not granted" }
        val base = context.getSystemService(SmsManager::class.java)
        val manager = if (subscriptionId >= 0) base.createForSubscriptionId(subscriptionId) else base
        // Not the default SMS app, so Android files these under Sent for us.
        for (address in addresses) {
            val parts = manager.divideMessage(body)
            if (parts.size > 1) manager.sendMultipartTextMessage(address, null, parts, null, null)
            else manager.sendTextMessage(address, null, body, null, null)
        }
        VLog.i(TAG, "sent ${body.length} chars to ${addresses.size} recipient(s)")
    }

    override val changes: Flow<Unit> = callbackFlow {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                trySend(Unit)
            }
        }
        resolver.registerContentObserver(Telephony.Sms.CONTENT_URI, true, observer)
        awaitClose { resolver.unregisterContentObserver(observer) }
    }

    private fun query(selection: String?, args: Array<String>?, limit: Int?): List<SmsMessage> {
        val out = ArrayList<SmsMessage>()
        readRows(selection, args, limit) { out.addAll(it) }
        return out
    }

    private inline fun readRows(selection: String?, args: Array<String>?, limit: Int? = null, block: (Sequence<SmsMessage>) -> Unit) {
        if (!available()) return
        val order = "${Telephony.Sms.DATE} DESC" + (limit?.let { " LIMIT $it" } ?: "")
        val cursor = runCatching { resolver.query(Telephony.Sms.CONTENT_URI, COLUMNS, selection, args, order) }
            .onFailure { VLog.w(TAG, "query failed: ${it.message}") }
            .getOrNull() ?: return
        cursor.use { c ->
            block(generateSequence { if (c.moveToNext()) c else null }.map { row ->
                val address = row.getString(1)
                SmsMessage(
                    addresses = listOfNotNull(address?.takeIf { it.isNotBlank() }),
                    body = row.getString(2).orEmpty(),
                    date = row.getLong(3),
                    type = row.getInt(4),
                    read = row.getInt(5) != 0,
                    threadId = row.getLong(6),
                    uid = row.getLong(0),
                    subscriptionId = if (row.isNull(7)) -1 else row.getInt(7),
                )
            })
        }
    }

    private companion object {
        const val TAG = "Sms"
        val COLUMNS = arrayOf(
            Telephony.Sms._ID, Telephony.Sms.ADDRESS, Telephony.Sms.BODY, Telephony.Sms.DATE,
            Telephony.Sms.TYPE, Telephony.Sms.READ, Telephony.Sms.THREAD_ID, Telephony.Sms.SUBSCRIPTION_ID,
        )
    }
}

// ---- contacts ----------------------------------------------------------------------------------

class AndroidContacts(private val context: Context) : ContactStore {
    override fun available() = PhonePermissions.granted(context, Manifest.permission.READ_CONTACTS)

    override fun timestamps(): Map<String, Long> {
        val out = LinkedHashMap<String, Long>()
        context.contentResolver.query(
            ContactsContract.Contacts.CONTENT_URI,
            arrayOf(ContactsContract.Contacts.LOOKUP_KEY, ContactsContract.Contacts.CONTACT_LAST_UPDATED_TIMESTAMP),
            null, null, null,
        )?.use { c ->
            while (c.moveToNext()) {
                val key = c.getString(0) ?: continue
                out[key] = if (c.isNull(1)) 0 else c.getLong(1)
            }
        }
        return out
    }

    override fun vcards(keys: Collection<String>): Map<String, String> = keys.mapNotNull { key ->
        val uri = Uri.withAppendedPath(ContactsContract.Contacts.CONTENT_VCARD_URI, Uri.encode(key))
        runCatching { context.contentResolver.openInputStream(uri)?.use { key to it.bufferedReader().readText() } }
            .onFailure { VLog.w("Contacts", "no vCard for a contact: ${it.message}") }
            .getOrNull()
    }.toMap()

    /** Name and base64 photo for a phone number, for call alerts. */
    fun lookup(number: String): Pair<String?, String?> {
        if (!available()) return null to null
        val uri = Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number))
        return runCatching {
            context.contentResolver.query(
                uri, arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME, ContactsContract.PhoneLookup.PHOTO_THUMBNAIL_URI), null, null, null,
            )?.use { c ->
                if (!c.moveToFirst()) return@use null to null
                val photo = c.getString(1)?.let { p ->
                    context.contentResolver.openInputStream(Uri.parse(p))?.use { Base64.encodeToString(it.readBytes(), Base64.NO_WRAP) }
                }
                c.getString(0) to photo
            } ?: (null to null)
        }.getOrDefault(null to null)
    }
}

// ---- calls -------------------------------------------------------------------------------------

/**
 * Turns PHONE_STATE broadcasts into ringing / talking / missed events. The number only comes
 * with READ_CALL_LOG, and Android then delivers each change twice: we use the copy with it.
 */
class CallMonitor(private val context: Context, private val contacts: AndroidContacts) : CallSource {
    private val _events = MutableSharedFlow<CallEvent>(extraBufferCapacity = 8)
    override val events: SharedFlow<CallEvent> = _events.asSharedFlow()

    private var registered = false
    private var lastState = TelephonyManager.CALL_STATE_IDLE
    private var last: CallEvent? = null
    private var muted = false

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            // With the call log allowed each change comes twice, with the number and without: take that
            // one. Without it the numberless copy is all there is, and a call without a name still counts.
            if (!intent.hasExtra(TelephonyManager.EXTRA_INCOMING_NUMBER) && PhonePermissions.granted(context, Manifest.permission.READ_CALL_LOG)) return
            @Suppress("DEPRECATION")
            val number = intent.getStringExtra(TelephonyManager.EXTRA_INCOMING_NUMBER)?.takeIf { it.isNotBlank() }
            val state = when (intent.getStringExtra(TelephonyManager.EXTRA_STATE)) {
                TelephonyManager.EXTRA_STATE_RINGING -> TelephonyManager.CALL_STATE_RINGING
                TelephonyManager.EXTRA_STATE_OFFHOOK -> TelephonyManager.CALL_STATE_OFFHOOK
                else -> TelephonyManager.CALL_STATE_IDLE
            }
            if (state == lastState) return
            lastState = state
            // Contact lookup touches the provider; keep it off the main thread.
            val pending = goAsync()
            Thread {
                try {
                    onState(state, number)
                } finally {
                    pending.finish()
                }
            }.start()
        }
    }

    @Synchronized
    private fun onState(state: Int, number: String?) {
        val (name, photo) = number?.let(contacts::lookup) ?: (null to null)
        // No number: maybe an app's call (WhatsApp...), which some phones report as a phone call.
        val appCall = if (number == null && state != TelephonyManager.CALL_STATE_IDLE && last?.app == null) appCall() else null
        val caller = name ?: appCall?.caller
        val app = appCall?.app ?: last?.app
        when (state) {
            TelephonyManager.CALL_STATE_RINGING -> emit(CallEvent.Ringing(number, caller, photo, app))
            TelephonyManager.CALL_STATE_OFFHOOK -> emit(CallEvent.Talking(number ?: last?.number, caller ?: last?.contactName, photo ?: last?.photo, app))
            else -> {
                val previous = last ?: return
                _events.tryEmit(CallEvent.Ended(previous))
                if (previous is CallEvent.Ringing) _events.tryEmit(CallEvent.Missed(previous.number, previous.contactName, app = previous.app))
                last = null
                restoreRinger()
            }
        }
    }

    /** The app's call notification can come a moment after the phone state: wait up to 2 s for it. */
    private fun appCall(): NotificationBridge.AppCall? {
        repeat(10) { i ->
            NotificationBridge.appCall(context)?.let { return it }
            if (i < 9) Thread.sleep(200)
        }
        return null
    }

    private fun emit(event: CallEvent) {
        last = event
        _events.tryEmit(event)
    }

    fun start() {
        if (registered) return
        if (!PhonePermissions.granted(context, Manifest.permission.READ_PHONE_STATE)) return
        ContextCompat.registerReceiver(context, receiver, IntentFilter(TelephonyManager.ACTION_PHONE_STATE_CHANGED), ContextCompat.RECEIVER_EXPORTED)
        registered = true
    }

    fun stop() {
        if (!registered) return
        runCatching { context.unregisterReceiver(receiver) }
        registered = false
        restoreRinger()
    }

    override fun muteRinger() {
        val audio = context.getSystemService(AudioManager::class.java)
        if (muted || audio.isStreamMute(AudioManager.STREAM_RING)) return
        runCatching { audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_MUTE, 0) }
            .onSuccess { muted = true }
            .onFailure { VLog.w("Calls", "can't mute ringer: ${it.message}") }
    }

    private fun restoreRinger() {
        if (!muted) return
        muted = false
        runCatching {
            context.getSystemService(AudioManager::class.java).adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_UNMUTE, 0)
        }
    }
}

// ---- mobile signal -----------------------------------------------------------------------------

/** Signal bars and network generation per active SIM, for the PC's status indicator. */
class SignalMonitor(private val context: Context) {
    private val _states = MutableStateFlow<Map<Int, SignalState>>(emptyMap())
    val states: StateFlow<Map<Int, SignalState>> = _states.asStateFlow()

    private val main = Handler(Looper.getMainLooper())
    private val executor = Executor { main.post(it) }
    private val callbacks = HashMap<Int, Pair<TelephonyManager, TelephonyCallback>>()
    private var subsListener: SubscriptionManager.OnSubscriptionsChangedListener? = null

    @SuppressLint("MissingPermission")
    fun start() {
        if (subsListener != null || !PhonePermissions.granted(context, Manifest.permission.READ_PHONE_STATE)) return
        val sm = context.getSystemService(SubscriptionManager::class.java) ?: return
        val listener = object : SubscriptionManager.OnSubscriptionsChangedListener() {
            override fun onSubscriptionsChanged() = refreshSubscriptions(sm)
        }
        subsListener = listener
        sm.addOnSubscriptionsChangedListener(executor, listener)
    }

    fun stop() {
        subsListener?.let { context.getSystemService(SubscriptionManager::class.java)?.removeOnSubscriptionsChangedListener(it) }
        subsListener = null
        callbacks.values.forEach { (tm, cb) -> tm.unregisterTelephonyCallback(cb) }
        callbacks.clear()
        _states.value = emptyMap()
    }

    @SuppressLint("MissingPermission")
    private fun refreshSubscriptions(sm: SubscriptionManager) {
        val active = runCatching { sm.activeSubscriptionInfoList.orEmpty().map { it.subscriptionId }.toSet() }.getOrDefault(emptySet())
        (callbacks.keys - active).forEach { id ->
            callbacks.remove(id)?.let { (tm, cb) -> tm.unregisterTelephonyCallback(cb) }
            _states.value -= id
        }
        val base = context.getSystemService(TelephonyManager::class.java)
        for (id in active - callbacks.keys) {
            val tm = base.createForSubscriptionId(id)
            val cb = SubCallback(id)
            runCatching { tm.registerTelephonyCallback(executor, cb) }
                .onSuccess { callbacks[id] = tm to cb }
                .onFailure { VLog.w("Signal", "can't watch SIM: ${it.message}") }
        }
    }

    private inner class SubCallback(private val id: Int) :
        TelephonyCallback(), TelephonyCallback.SignalStrengthsListener, TelephonyCallback.DisplayInfoListener, TelephonyCallback.ServiceStateListener {
        private var level = 0
        private var type = "Unknown"
        private var nr = false

        override fun onSignalStrengthsChanged(signalStrength: SignalStrength) {
            level = signalStrength.level.coerceIn(0, 4)
            publish()
        }

        override fun onDisplayInfoChanged(info: TelephonyDisplayInfo) {
            // NSA 5G shows as LTE underneath; the override is what the status bar shows.
            nr = info.overrideNetworkType == TelephonyDisplayInfo.OVERRIDE_NETWORK_TYPE_NR_NSA ||
                info.overrideNetworkType == TelephonyDisplayInfo.OVERRIDE_NETWORK_TYPE_NR_ADVANCED
            type = networkName(info.networkType)
            publish()
        }

        override fun onServiceStateChanged(state: ServiceState) {
            if (state.state != ServiceState.STATE_IN_SERVICE) {
                level = 0
                publish()
            }
        }

        private fun publish() {
            _states.value += id to SignalState(if (nr) "5G" else type, level)
        }
    }

    private fun networkName(type: Int) = when (type) {
        TelephonyManager.NETWORK_TYPE_GPRS -> "GPRS"
        TelephonyManager.NETWORK_TYPE_EDGE -> "EDGE"
        TelephonyManager.NETWORK_TYPE_UMTS, TelephonyManager.NETWORK_TYPE_TD_SCDMA -> "UMTS"
        TelephonyManager.NETWORK_TYPE_HSDPA, TelephonyManager.NETWORK_TYPE_HSUPA,
        TelephonyManager.NETWORK_TYPE_HSPA, TelephonyManager.NETWORK_TYPE_HSPAP -> "HSPA"
        TelephonyManager.NETWORK_TYPE_LTE, TelephonyManager.NETWORK_TYPE_IWLAN -> "LTE"
        TelephonyManager.NETWORK_TYPE_NR -> "5G"
        TelephonyManager.NETWORK_TYPE_GSM -> "GSM"
        TelephonyManager.NETWORK_TYPE_CDMA, TelephonyManager.NETWORK_TYPE_1xRTT -> "CDMA"
        TelephonyManager.NETWORK_TYPE_EVDO_0, TelephonyManager.NETWORK_TYPE_EVDO_A,
        TelephonyManager.NETWORK_TYPE_EVDO_B, TelephonyManager.NETWORK_TYPE_EHRPD -> "CDMA2000"
        TelephonyManager.NETWORK_TYPE_IDEN -> "iDEN"
        else -> "Unknown"
    }
}
