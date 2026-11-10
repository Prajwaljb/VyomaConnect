package dev.vyoma.connect.platform

import android.content.Context

/**
 * Which PCs may answer, decline and hang up this phone's calls: all, until the owner turns one off on its page.
 * The phone enforces it (the PC is told it can't, so it shows no Answer, and any request from it is refused).
 */
object CallsFromPc {
    private const val PREFS = "calls_from_pc"
    private const val KEY_OFF = "off"

    private lateinit var app: Context

    fun install(context: Context) {
        app = context.applicationContext
    }

    private fun prefs() = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun allowed(peerId: String): Boolean = peerId !in prefs().getStringSet(KEY_OFF, emptySet()).orEmpty()

    fun set(peerId: String, on: Boolean) {
        val off = prefs().getStringSet(KEY_OFF, emptySet()).orEmpty()
        prefs().edit().putStringSet(KEY_OFF, if (on) off - peerId else off + peerId).apply()
    }
}
