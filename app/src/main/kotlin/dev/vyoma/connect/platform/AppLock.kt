package dev.vyoma.connect.platform

import android.app.Activity
import android.app.Application
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricManager.Authenticators
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.os.SystemClock
import android.view.WindowManager
import android.widget.Toast
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.vyoma.connect.BuildConfig

/**
 * Someone holding this phone unlocked shouldn't get into the PC with it: unlocking it, its files,
 * its screen, its camera, its power actions, the touchpad, keyboard and presenter, and the clipboard
 * history ask for a fingerprint (or the phone's PIN) first, and so does any command the owner marked
 * (see [commandLocked]). Once is enough for one go at using Vyoma; the go ends when the phone's
 * screen turns off or after [AWAY_MS] away from the app. Asked at the first of those, not when the
 * app opens, so skipping a song right after unlocking the phone takes no second fingerprint. Rules
 * run unattended, so they aren't asked; their triggers were set up here.
 */
object AppLock {
    const val SCREENSHOT = "To take a screenshot of the PC"
    const val CAMERA = "To take a photo with the PC's camera"
    const val TOUCHPAD = "To control the PC"
    const val PRESENTER = "To present on the PC"
    const val CLIPBOARD = "To see your clipboard history"
    private const val PREFS = "app_lock"
    private const val KEY_ENABLED = "enabled"
    private const val KEY_COMMANDS = "commands"
    private const val KEY_UNLOCK_COUNTS = "unlock_counts"
    private const val AUTHENTICATORS = Authenticators.BIOMETRIC_WEAK or Authenticators.DEVICE_CREDENTIAL

    private val session = LockSession(SystemClock::elapsedRealtime)
    private var asking = false

    fun install(app: Application) {
        app.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityStarted(activity: Activity) {
                session.started()
                hideFromRecents(activity)
            }
            override fun onActivityStopped(activity: Activity) = session.stopped()
            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
            override fun onActivityResumed(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = Unit
        })
        app.registerReceiver(object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (intent.action == Intent.ACTION_SCREEN_OFF) session.end()
                else if (unlockCounts(context)) session.phoneUnlocked()
            }
        }, IntentFilter(Intent.ACTION_SCREEN_OFF).apply { addAction(Intent.ACTION_USER_PRESENT) }, Context.RECEIVER_NOT_EXPORTED)
    }

    /** Recents keeps a picture of the last page, which would show it with the lock closed again. */
    private fun hideFromRecents(activity: Activity) {
        val hide = enabled(activity)
        if (Build.VERSION.SDK_INT >= 33) activity.setRecentsScreenshotEnabled(!hide)
        else if (hide) activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        else activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** On unless turned off; the demo is for showing around, so there it starts off. */
    fun enabled(context: Context) = prefs(context).getBoolean(KEY_ENABLED, !BuildConfig.DEMO)

    fun setEnabled(context: Context, on: Boolean) = prefs(context).edit().putBoolean(KEY_ENABLED, on).apply()

    /**
     * Whether unlocking the phone just now is as good as being asked. Off unless chosen: Android
     * doesn't say how it was unlocked, so a trusted place or device counts too, with nobody proving anything.
     */
    fun unlockCounts(context: Context) = prefs(context).getBoolean(KEY_UNLOCK_COUNTS, false)

    fun setUnlockCounts(context: Context, on: Boolean) = prefs(context).edit().putBoolean(KEY_UNLOCK_COUNTS, on).apply()

    private fun commandEntry(pc: String, key: String) = "$pc/$key"

    /** Whether this command of this PC asks first. Chosen per command, so it holds even with the lock above off. */
    fun commandLocked(context: Context, pc: String, key: String) =
        commandEntry(pc, key) in prefs(context).getStringSet(KEY_COMMANDS, emptySet()).orEmpty()

    fun setCommandLocked(context: Context, pc: String, key: String, locked: Boolean) {
        val all = prefs(context).getStringSet(KEY_COMMANDS, emptySet()).orEmpty().toMutableSet()
        if (locked) all += commandEntry(pc, key) else all -= commandEntry(pc, key)
        prefs(context).edit().putStringSet(KEY_COMMANDS, all).apply()
    }

    /** Runs [then] once a marked command may go: the same one pass per go at using Vyoma. */
    fun askForCommand(context: Context, why: String, then: () -> Unit) {
        if (session.open || !canAsk(context)) then() else prompt(context, why, then)
    }

    /** False on a phone with no screen lock: there's nothing to ask for, so nothing is asked. */
    fun canAsk(context: Context): Boolean {
        val answer = context.getSystemService(BiometricManager::class.java).canAuthenticate(AUTHENTICATORS)
        return answer != BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED && answer != BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE
    }

    /** Whether what's protected may show right now. */
    fun isOpen(context: Context) = session.open || !enabled(context) || !canAsk(context)

    /** Runs [then] on the main thread once the person is known to be the owner, asking if needed. */
    fun ask(context: Context, why: String, then: () -> Unit) {
        if (isOpen(context)) then() else prompt(context, why, then)
    }

    /** Like [ask], but always asks: for what would loosen the lock itself. */
    fun confirm(context: Context, why: String, then: () -> Unit) {
        if (canAsk(context)) prompt(context, why, then) else then()
    }

    private fun prompt(context: Context, why: String, then: () -> Unit) {
        if (asking) return
        asking = true
        BiometricPrompt.Builder(context)
            .setTitle("Confirm it's you")
            .setSubtitle(why)
            .setAllowedAuthenticators(AUTHENTICATORS)
            .setConfirmationRequired(false)
            .build()
            .authenticate(CancellationSignal(), context.mainExecutor, object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    asking = false
                    session.passed()
                    then()
                }

                override fun onAuthenticationError(code: Int, message: CharSequence) {
                    asking = false
                    if (code != BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED && code != BiometricPrompt.BIOMETRIC_ERROR_CANCELED) {
                        Toast.makeText(context, message, Toast.LENGTH_LONG).show()
                    }
                }
            })
    }
}

internal const val AWAY_MS = 5 * 60_000L
internal const val UNLOCK_GRACE_MS = 30_000L

/** One go at using the app. Main thread only. */
internal class LockSession(private val now: () -> Long) {
    /** State, so a page showing something only while this is open follows it. */
    var open by mutableStateOf(false)
        private set
    private var visible = 0
    private var leftAt = 0L
    private var unlockedAt: Long? = null

    fun passed() { open = true }
    fun end() {
        open = false
        unlockedAt = null
    }

    /** The phone was unlocked, and that counts: with Vyoma on screen at once, or if it's opened soon. */
    fun phoneUnlocked() {
        unlockedAt = now()
        if (visible > 0) open = true
    }

    fun started() {
        if (visible == 0) {
            if (now() - leftAt >= AWAY_MS) open = false
            if (unlockedAt?.let { now() - it < UNLOCK_GRACE_MS } == true) open = true
        }
        visible++
    }

    fun stopped() {
        visible = (visible - 1).coerceAtLeast(0)
        if (visible == 0) leftAt = now()
    }
}
