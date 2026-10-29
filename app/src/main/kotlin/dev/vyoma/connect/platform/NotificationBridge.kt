package dev.vyoma.connect.platform

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Bundle
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import androidx.core.graphics.createBitmap
import dev.vyoma.connect.protocol.VLog
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
    override fun onListenerConnected() = NotificationBridge.attach(this)
    override fun onListenerDisconnected() = NotificationBridge.detach(this)
    override fun onNotificationPosted(sbn: StatusBarNotification) = NotificationBridge.posted(this, sbn)
    override fun onNotificationRemoved(sbn: StatusBarNotification) = NotificationBridge.removed(sbn)
}

/** Bridges the listener service to the protocol's [NotificationSource]. */
object NotificationBridge : NotificationSource {
    private const val TAG = "Notifications"
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

        val icon = iconFor(context, sbn.packageName)
        val update = !seen.add(key)
        return PhoneNotification(
            id = key,
            appName = appLabel(context, sbn.packageName),
            title = title,
            text = text,
            time = sbn.postTime,
            clearable = sbn.isClearable,
            actions = plainActions.map { it.title.toString() },
            replyId = replyId,
            icon = icon?.first,
            iconHash = icon?.second,
            silent = update,
        )
    }

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

    private fun appLabel(context: Context, pkg: String) = labels.getOrPut(pkg) {
        runCatching { context.packageManager.getApplicationLabel(context.packageManager.getApplicationInfo(pkg, 0)).toString() }.getOrDefault(pkg)
    }

    /** App icon as a 96px PNG plus its MD5, the cache key KDE Connect expects. */
    private fun iconFor(context: Context, pkg: String): Pair<ByteArray, String>? = icons[pkg] ?: runCatching {
        val drawable = context.packageManager.getApplicationIcon(pkg)
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
