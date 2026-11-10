package dev.vyoma.connect.platform

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.telecom.TelecomManager
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import androidx.core.graphics.createBitmap
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.service.LinkService
import dev.vyoma.connect.protocol.plugin.builtin.NotificationEvent
import dev.vyoma.connect.protocol.plugin.builtin.NotificationSource
import dev.vyoma.connect.protocol.plugin.builtin.PhoneNotification
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import java.io.ByteArrayOutputStream
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/** Receives the system's notification stream while the user has granted notification access. */
class VyomaNotificationListener : NotificationListenerService() {
    override fun onListenerConnected() {
        NotificationBridge.attach(this)
        // The system rebinds us after updates and reboots: a good moment to bring the link back.
        runCatching { LinkService.start(this) }.onFailure { VLog.w("Notifications", "can't start link service: ${it.message}") }
    }
    override fun onListenerDisconnected() = NotificationBridge.detach(this)
    override fun onNotificationPosted(sbn: StatusBarNotification) = NotificationBridge.posted(this, sbn)
    override fun onNotificationRemoved(sbn: StatusBarNotification) = NotificationBridge.removed(sbn)
}

/** Bridges the listener service to the protocol's [NotificationSource]. */
object NotificationBridge : NotificationSource {
    private const val TAG = "Notifications"
    /** Notification.EXTRA_BUILDER_APPLICATION_INFO, hidden from the SDK but set on every notification. */
    private const val EXTRA_APP_INFO = "android.appInfo"
    @Volatile private var listener: NotificationListenerService? = null

    private val _events = MutableSharedFlow<NotificationEvent>(extraBufferCapacity = 64, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    override val events: SharedFlow<NotificationEvent> = _events.asSharedFlow()

    private class Reply(val key: String, val action: Notification.Action, val input: RemoteInput)
    private val replies = ConcurrentHashMap<String, Reply>()
    private val replyIdsByKey = ConcurrentHashMap<String, String>()
    private val actionsByKey = ConcurrentHashMap<String, List<Notification.Action>>()
    private val seen = ConcurrentHashMap.newKeySet<String>()
    private val icons = ConcurrentHashMap<String, Pair<ByteArray, String>>()

    fun isEnabled(context: Context): Boolean {
        val flat = Settings.Secure.getString(context.contentResolver, "enabled_notification_listeners") ?: return false
        val me = ComponentName(context, VyomaNotificationListener::class.java)
        return flat.split(':').any { ComponentName.unflattenFromString(it) == me }
    }

    fun settingsIntent(context: Context): Intent =
        Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
            .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, ComponentName(context, VyomaNotificationListener::class.java).flattenToString())

    internal fun attach(service: NotificationListenerService) {
        listener = service
        VLog.i(TAG, "listener connected")
    }

    internal fun detach(service: NotificationListenerService) {
        if (listener === service) listener = null
    }

    internal fun posted(context: Context, sbn: StatusBarNotification) {
        val n = convert(context, sbn) ?: return
        _events.tryEmit(NotificationEvent.Posted(n))
    }

    internal fun removed(sbn: StatusBarNotification) {
        val key = sbn.key
        actionsByKey.remove(key)
        replyIdsByKey.remove(key)?.let(replies::remove)
        if (seen.remove(key)) _events.tryEmit(NotificationEvent.Removed(key))
    }

    /**
     * [pkg]'s choice changed (see [NotificationFilters]): what the PC shows of it follows at once:
     * gone when turned off, redrawn (quietly) when hidden or shown again.
     */
    fun refilter(context: Context, pkg: String) {
        val l = listener ?: return
        val mine = runCatching { l.activeNotifications }.getOrNull().orEmpty().filter { it.packageName == pkg }
        for (sbn in mine) {
            val wasShown = sbn.key in seen
            val n = convert(l, sbn)
            when {
                n != null -> _events.tryEmit(NotificationEvent.Posted(n.copy(silent = true)))
                wasShown && seen.remove(sbn.key) -> _events.tryEmit(NotificationEvent.Removed(sbn.key))
            }
        }
    }

    override fun active(): List<PhoneNotification> {
        val l = listener ?: return emptyList()
        return runCatching { l.activeNotifications }.getOrNull().orEmpty().mapNotNull { convert(l, it) }
    }

    override fun dismiss(id: String) {
        runCatching { listener?.cancelNotification(id) }
    }

    override fun reply(replyId: String, message: String) {
        val r = replies[replyId] ?: return
        val l = listener ?: return
        val intent = Intent().addFlags(Intent.FLAG_RECEIVER_FOREGROUND)
        val results = Bundle().apply { putCharSequence(r.input.resultKey, message) }
        RemoteInput.addResultsToIntent(arrayOf(r.input), intent, results)
        runCatching { r.action.actionIntent.send(l, 0, intent) }.onFailure { VLog.w(TAG, "reply failed: ${it.message}") }
    }

    override fun action(id: String, action: String) {
        val match = actionsByKey[id]?.firstOrNull { it.title?.toString() == action } ?: return
        runCatching { match.actionIntent.send() }.onFailure { e -> if (e is PendingIntent.CanceledException) VLog.d(TAG, "action gone") }
    }

    /** An app's call: who's calling, if the app says and the user lets its words through. */
    data class AppCall(val app: String, val caller: String?)

    /**
     * The app ringing or in a call over the internet (WhatsApp, Signal, Telegram...), from its call
     * notification. Phones report these calls without a number; the phone's own calls are skipped.
     */
    fun appCall(context: Context): AppCall? {
        val l = listener ?: return null
        val skip = buildSet {
            add(context.packageName)
            addAll(PHONE_APPS)
            runCatching { context.getSystemService(TelecomManager::class.java)?.defaultDialerPackage }.getOrNull()?.let(::add)
        }
        val calls = runCatching { l.activeNotifications }.getOrNull().orEmpty().filter { sbn ->
            val n = sbn.notification
            sbn.packageName !in skip &&
                (n.category == Notification.CATEGORY_CALL || n.extras.getString(Notification.EXTRA_TEMPLATE) == CALL_STYLE)
        }
        // The ringing one first: it's the call the phone state is about.
        val sbn = calls.maxByOrNull { it.postTime } ?: return null
        val extras = sbn.notification.extras
        val appInfo = if (Build.VERSION.SDK_INT >= 33) extras.getParcelable(EXTRA_APP_INFO, ApplicationInfo::class.java)
        else @Suppress("DEPRECATION") extras.getParcelable(EXTRA_APP_INFO)
        val app = appLabel(context, sbn.packageName, appInfo)
        if (NotificationFilters.load(context).mode(context, sbn.packageName) != NotifMode.SHOW) return AppCall(app, null)
        val person = extras.getParcelable(Notification.EXTRA_CALL_PERSON, android.app.Person::class.java)?.name
        val caller = (person ?: extras.getCharSequence(Notification.EXTRA_TITLE))?.toString()?.trim()?.takeIf { it.isNotEmpty() && it != app }
        return AppCall(app, caller)
    }

    /**
     * Presses Answer, Decline or Hang up ([action], a `CallAction`) on the newest call notification,
     * an app's ([appOnly]) or any. CallStyle notifications carry those as intents; older ones as buttons,
     * found by their words. False when there's no such call or button.
     */
    fun pressCall(context: Context, action: String, appOnly: Boolean): Boolean {
        val l = listener ?: return false
        val calls = runCatching { l.activeNotifications }.getOrNull().orEmpty().filter { sbn ->
            val n = sbn.notification
            sbn.packageName != context.packageName && (!appOnly || sbn.packageName !in PHONE_APPS) &&
                (n.category == Notification.CATEGORY_CALL || n.extras.getString(Notification.EXTRA_TEMPLATE) == CALL_STYLE)
        }
        val n = calls.maxByOrNull { it.postTime }?.notification ?: return false
        val key = when (action) {
            "answer" -> Notification.EXTRA_ANSWER_INTENT
            "decline" -> Notification.EXTRA_DECLINE_INTENT
            else -> Notification.EXTRA_HANG_UP_INTENT
        }
        @Suppress("DEPRECATION")
        val intent = n.extras.getParcelable<PendingIntent>(key)
            // Declining a call that's on, or hanging up one that rings, is the same button there.
            ?: if (action != "answer") @Suppress("DEPRECATION") n.extras.getParcelable<PendingIntent>(
                if (action == "decline") Notification.EXTRA_HANG_UP_INTENT else Notification.EXTRA_DECLINE_INTENT,
            ) else null
        val words = when (action) {
            "answer" -> Regex("answer|accept|pick up", RegexOption.IGNORE_CASE)
            else -> Regex("decline|reject|hang ?up|end|dismiss", RegexOption.IGNORE_CASE)
        }
        val target = intent ?: n.actions.orEmpty().firstOrNull { words.containsMatchIn(it.title?.toString().orEmpty()) }?.actionIntent ?: return false
        // Answering opens the app's call screen: let it come up from the background, as a tap would.
        val options = if (Build.VERSION.SDK_INT >= 34) {
            android.app.ActivityOptions.makeBasic()
                .setPendingIntentBackgroundActivityStartMode(android.app.ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED).toBundle()
        } else null
        return runCatching { target.send(context, 0, null, null, null, null, options); true }.getOrElse { VLog.d(TAG, "call button gone"); false }
    }

    private const val CALL_STYLE = "android.app.Notification\$CallStyle"
    /** The phone's own call screens, which show phone calls (those come with their number). */
    private val PHONE_APPS = setOf("com.android.server.telecom", "com.samsung.android.incallui", "com.android.incallui", "com.google.android.dialer", "com.samsung.android.dialer", "com.android.dialer")

    private fun convert(context: Context, sbn: StatusBarNotification): PhoneNotification? {
        val n = sbn.notification
        if (sbn.packageName == context.packageName) return null
        if (n.flags and Notification.FLAG_GROUP_SUMMARY != 0) return null
        if (n.flags and Notification.FLAG_FOREGROUND_SERVICE != 0) return null
        if (n.flags and Notification.FLAG_LOCAL_ONLY != 0) return null
        if (!sbn.isClearable && n.flags and Notification.FLAG_ONGOING_EVENT != 0) return null
        if (n.extras.containsKey(Notification.EXTRA_MEDIA_SESSION)) return null
        if (n.category == Notification.CATEGORY_TRANSPORT || n.category == Notification.CATEGORY_PROGRESS) return null

        val extras = n.extras
        val title = (extras.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE) ?: extras.getCharSequence(Notification.EXTRA_TITLE))?.toString().orEmpty()
        val text = (lastMessage(extras) ?: extras.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: extras.getCharSequence(Notification.EXTRA_TEXT))?.toString().orEmpty()
        if (title.isBlank() && text.isBlank()) return null

        // Android 11+ hides other packages from us; the notification carries its app's info.
        val appInfo = if (Build.VERSION.SDK_INT >= 33) extras.getParcelable(EXTRA_APP_INFO, ApplicationInfo::class.java)
        else @Suppress("DEPRECATION") extras.getParcelable(EXTRA_APP_INFO)
        val appName = appLabel(context, sbn.packageName, appInfo)
        val filters = NotificationFilters.load(context)
        filters.posted(context, sbn.packageName, appName, sbn.postTime)
        val mode = filters.mode(context, sbn.packageName)
        if (mode == NotifMode.OFF) return null
        val hidden = mode == NotifMode.PRIVATE

        val key = sbn.key
        var replyId: String? = null
        val plainActions = mutableListOf<Notification.Action>()
        for (action in n.actions.orEmpty()) {
            val input = action.remoteInputs?.firstOrNull { it.allowFreeFormInput }
            if (input != null && replyId == null) {
                replyId = replyIdsByKey.getOrPut(key) { UUID.randomUUID().toString() }
                replies[replyId] = Reply(key, action, input)
            } else if (input == null && !action.title.isNullOrBlank()) {
                plainActions += action
            }
        }
        actionsByKey[key] = plainActions

        val icon = iconFor(context, sbn.packageName, appInfo)
        val update = !seen.add(key)
        // Text hidden: the PC learns only that the app has something (no words, so no replies or
        // actions either, whose labels can give it away).
        return PhoneNotification(
            id = key,
            appName = appName,
            title = if (hidden) appName else title,
            text = if (hidden) "New notification" else text,
            time = sbn.postTime,
            clearable = sbn.isClearable,
            actions = if (hidden) emptyList() else plainActions.map { it.title.toString() },
            replyId = if (hidden) null else replyId,
            icon = icon?.first,
            iconHash = icon?.second,
            silent = update,
            texts = isTextingApp(context, sbn.packageName),
        )
    }

    /** The phone's texting app (the default SMS app, or a known one): its notifications follow the texts lock. */
    fun isTextingApp(context: Context, pkg: String): Boolean =
        pkg in TEXTING_APPS || runCatching { android.provider.Telephony.Sms.getDefaultSmsPackage(context) }.getOrNull() == pkg

    private val TEXTING_APPS = setOf("com.samsung.android.messaging", "com.google.android.apps.messaging", "com.android.messaging")

    private fun lastMessage(extras: Bundle): CharSequence? {
        @Suppress("DEPRECATION")
        val messages = extras.getParcelableArray(Notification.EXTRA_MESSAGES) ?: return null
        val last = messages.lastOrNull() as? Bundle ?: return null
        val sender = last.getCharSequence("sender")
        val text = last.getCharSequence("text") ?: return null
        val group = extras.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE) != null
        return if (group && sender != null) "$sender: $text" else text
    }

    private val labels = ConcurrentHashMap<String, String>()

    private fun appLabel(context: Context, pkg: String, info: ApplicationInfo?) = labels.getOrPut(pkg) {
        val pm = context.packageManager
        runCatching { pm.getApplicationLabel(info ?: pm.getApplicationInfo(pkg, 0)).toString() }.getOrDefault(pkg)
    }

    /** App icon as a 96px PNG plus its MD5, the cache key KDE Connect expects. */
    private fun iconFor(context: Context, pkg: String, info: ApplicationInfo?): Pair<ByteArray, String>? = icons[pkg] ?: runCatching {
        val pm = context.packageManager
        val drawable = if (info != null) pm.getApplicationIcon(info) else pm.getApplicationIcon(pkg)
        val size = 96
        val bitmap = createBitmap(size, size, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, size, size)
        drawable.draw(Canvas(bitmap))
        val png = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        bitmap.recycle()
        val hash = MessageDigest.getInstance("MD5").digest(png).joinToString("") { "%02x".format(it) }
        (png to hash).also { icons[pkg] = it }
    }.getOrNull()
}
