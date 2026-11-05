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
import android.os.Bundle
import android.os.CancellationSignal
import android.os.SystemClock
import android.widget.Toast
import dev.vyoma.connect.BuildConfig

/**
 * Someone holding this phone unlocked shouldn't get into the PC with it: unlocking it, its files,
 * its screen, its camera and its power actions ask for a fingerprint (or the phone's PIN) first, and
 * so does any command the owner marked (see [commandLocked]). Once is enough
 * for one go at using Vyoma; the go ends when the phone's screen turns off or after [AWAY_MS] away
 * from the app. Rules run unattended, so they aren't asked; their triggers were set up here.
 */
object AppLock {
    const val SCREENSHOT = "To take a screenshot of the PC"
    const val CAMERA = "To take a photo with the PC's camera"
    private const val PREFS = "app_lock"
    private const val KEY_ENABLED = "enabled"
    private const val KEY_COMMANDS = "commands"
    private const val AUTHENTICATORS = Authenticators.BIOMETRIC_WEAK or Authenticators.DEVICE_CREDENTIAL

    private val session = LockSession(SystemClock::elapsedRealtime)
    private var asking = false

    fun install(app: Application) {
        app.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityStarted(activity: Activity) = session.started()
            override fun onActivityStopped(activity: Activity) = session.stopped()
            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
            override fun onActivityResumed(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = Unit
        })
        app.registerReceiver(object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = session.end()
        }, IntentFilter(Intent.ACTION_SCREEN_OFF), Context.RECEIVER_NOT_EXPORTED)
    }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** On unless turned off; the demo is for showing around, so there it starts off. */
    fun enabled(context: Context) = prefs(context).getBoolean(KEY_ENABLED, !BuildConfig.DEMO)

    fun setEnabled(context: Context, on: Boolean) = prefs(context).edit().putBoolean(KEY_ENABLED, on).apply()

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

/** One go at using the app. Main thread only. */
internal class LockSession(private val now: () -> Long) {
    var open = false
        private set
    private var visible = 0
    private var leftAt = 0L

    fun passed() { open = true }
    fun end() { open = false }

    fun started() {
        if (visible == 0 && now() - leftAt >= AWAY_MS) open = false
        visible++
    }

    fun stopped() {
        visible = (visible - 1).coerceAtLeast(0)
        if (visible == 0) leftAt = now()
    }
}
