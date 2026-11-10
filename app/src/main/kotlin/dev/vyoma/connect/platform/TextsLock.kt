package dev.vyoma.connect.platform

import android.app.ActivityManager
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.os.SystemClock
import dev.vyoma.connect.BuildConfig
import dev.vyoma.connect.R
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.TextsGuard
import dev.vyoma.connect.service.Notifications
import dev.vyoma.connect.ui.TextsUnlockActivity
import java.util.concurrent.ConcurrentHashMap

/**
 * Texts go to a PC's Messages page only after the owner says so here, with a fingerprint (or the phone's PIN), each
 * time someone opens it there: whoever sits at an unlocked PC shouldn't read this phone's texts and login codes. The
 * PC locks it again when its screen locks, when Messages sits unused for 10 minutes and when the phone goes; this
 * phone refuses texts to it meanwhile, so turning something off on the PC can't get round it. Only Vyoma's PCs can
 * ask, so stock desktops are let be ([TextsGuard]).
 */
object TextsLock : TextsGuard {
    private const val TAG = "TextsLock"
    private const val PREFS = "texts_lock"
    private const val KEY_ON = "on"
    private const val KEY_HELD = "held"
    private const val ASK_FOR_MS = 120_000L
    /** A yes lasts at most this long on the phone's own clock, whatever the PC says (it locks it sooner). */
    private const val GRANT_MS = 60 * 60_000L
    /** Asks closer together than this are one: anything on the PC could otherwise flood the phone with them. */
    private const val ASK_GAP_MS = 3_000L

    private lateinit var app: Context
    /** PC → when its owner said yes (elapsed realtime). */
    private val grants = ConcurrentHashMap<String, Long>()
    private val waiting = ConcurrentHashMap<String, (Boolean) -> Unit>()
    private val askedAt = ConcurrentHashMap<String, Long>()

    fun install(context: Context) {
        app = context.applicationContext
    }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** On unless turned off; the demo is for showing around, so there it starts off. */
    fun enabled(context: Context) = prefs(context).getBoolean(KEY_ON, !BuildConfig.DEMO)

    fun setEnabled(context: Context, on: Boolean) {
        prefs(context).edit().putBoolean(KEY_ON, on).apply()
        if (on) grants.clear()
    }

    override fun open(peerId: String): Boolean {
        if (!enabled(app)) return true
        val at = grants[peerId] ?: return false
        if (SystemClock.elapsedRealtime() - at < GRANT_MS) return true
        grants.remove(peerId)
        return false
    }

    override fun holds(peerId: String, claims: Boolean): Boolean {
        val prefs = prefs(app)
        val held = prefs.getStringSet(KEY_HELD, emptySet()).orEmpty()
        if (claims && peerId !in held) prefs.edit().putStringSet(KEY_HELD, held + peerId).apply()
        return claims || peerId in held
    }

    override fun ask(peerId: String, peerName: String, answer: (Boolean) -> Unit) {
        if (!enabled(app)) {
            answer(true)
            return
        }
        val now = SystemClock.elapsedRealtime()
        val last = askedAt.put(peerId, now)
        if (last != null && now - last < ASK_GAP_MS && waiting.containsKey(peerId)) {
            waiting[peerId] = answer
            return
        }
        waiting[peerId] = answer
        VLog.i(TAG, "a PC asks for the texts")
        // Straight to the prompt only when there's a fingerprint or PIN to ask for: with no screen lock, the
        // prompt would let it through untouched, so then it's the notification's Allow that says yes.
        if (inFront() && AppLock.canAsk(app)) {
            runCatching { app.startActivity(TextsUnlockActivity.intent(app, peerId, peerName)) }
                .onFailure { notifyAsk(peerId, peerName) }
        } else {
            notifyAsk(peerId, peerName)
        }
    }

    override fun close(peerId: String) {
        grants.remove(peerId)
        waiting.remove(peerId)
        app.getSystemService(NotificationManager::class.java).cancel(notificationId(peerId))
    }

    /** The owner answered on this phone (or turned it down). An answer nobody waits for any more is dropped. */
    fun answered(peerId: String, ok: Boolean) {
        app.getSystemService(NotificationManager::class.java).cancel(notificationId(peerId))
        val answer = waiting.remove(peerId) ?: return
        if (ok) grants[peerId] = SystemClock.elapsedRealtime()
        VLog.i(TAG, if (ok) "texts allowed" else "texts not allowed")
        answer(ok)
    }

    private fun inFront(): Boolean {
        val me = ActivityManager.RunningAppProcessInfo()
        ActivityManager.getMyMemoryState(me)
        return me.importance == ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
    }

    private fun notifyAsk(peerId: String, peerName: String) {
        if (!Notifications.allowed(app)) return
        val open = PendingIntent.getActivity(
            app, notificationId(peerId), TextsUnlockActivity.intent(app, peerId, peerName),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val refuse = PendingIntent.getActivity(
            app, notificationId(peerId) + 1, TextsUnlockActivity.intent(app, peerId, peerName, refuse = true),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = androidx.core.app.NotificationCompat.Builder(app, Notifications.CH_PAIRING)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Show your texts on $peerName?")
            .setContentText("Messages was opened there. Tap to confirm it's you.")
            .setContentIntent(open)
            .setAutoCancel(true)
            .setTimeoutAfter(ASK_FOR_MS)
            .setCategory(Notification.CATEGORY_RECOMMENDATION)
            .addAction(0, "Don't allow", refuse)
            // Not from the lock screen: whoever holds the locked phone can't let texts out.
            .addAction(androidx.core.app.NotificationCompat.Action.Builder(0, "Allow", open).setAuthenticationRequired(true).build())
            .setVisibility(androidx.core.app.NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(
                androidx.core.app.NotificationCompat.Builder(app, Notifications.CH_PAIRING)
                    .setSmallIcon(R.drawable.ic_notification)
                    .setContentTitle("A PC asks for your texts")
                    .setContentText("Unlock to answer")
                    .build(),
            )
            .build()
        app.getSystemService(NotificationManager::class.java).notify(notificationId(peerId), n)
    }

    private fun notificationId(peerId: String) = 3000 + (peerId.hashCode() and 0xFFF) * 2
}
