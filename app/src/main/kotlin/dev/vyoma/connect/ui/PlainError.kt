package dev.vyoma.connect.ui

/**
 * An error the PC (or a library) sent, as something to show: kept when it already reads as a plain
 * sentence ("The laptop's lid is closed"), null when it's an internal error chain ("pactl: exit
 * status 1", "org.freedesktop..."), so the caller says it in its own plain words instead. The raw
 * text still goes to the log.
 */
internal fun plainReason(raw: String?): String? {
    val text = raw?.trim()?.takeIf { it.isNotEmpty() } ?: return null
    if (!text.first().isUpperCase() || text.length > 140) return null
    val internal = listOf("os error", "exit status", "::", "errno", "Exception", "org.", "0x", "{", "dbus", "D-Bus", "gst", "GStreamer", "pipeline", "EOF", "ECONN", "failed to", "Error:")
    return text.takeUnless { t -> internal.any { t.contains(it, ignoreCase = it.first().isLowerCase()) } }
}
