package dev.vyoma.connect.ui

import dev.vyoma.connect.protocol.plugin.builtin.MousePadPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SpecialKey

/**
 * Which desktop the PC runs, for the touchpad's multi-finger gestures: each is sent as that
 * desktop's own default shortcut (the KDE Connect mousepad can press keys, not system gestures).
 * The PC doesn't say which it is, so the touchpad lets you pick; Plasma is the default.
 */
enum class DesktopKind(val label: String) {
    PLASMA("Plasma"), GNOME("GNOME"), WINDOWS("Windows"), MACOS("macOS");

    /** Three fingers up: every window at a glance. */
    suspend fun overview(p: MousePadPlugin) = when (this) {
        PLASMA -> p.chord("w", meta = true)                 // Overview
        GNOME -> p.chord("s", meta = true)                  // Activities overview
        WINDOWS -> p.key(SpecialKey.TAB, meta = true)       // Task view
        MACOS -> p.key(SpecialKey.UP, ctrl = true)          // Mission Control
    }

    /** Three fingers down: the desktop (windows out of the way). */
    suspend fun desktop(p: MousePadPlugin) = when (this) {
        PLASMA -> p.chord("d", meta = true)                 // Peek at desktop
        GNOME -> p.chord("a", meta = true)                  // App grid (GNOME has no show-desktop key by default)
        WINDOWS -> p.chord("d", meta = true)                // Show desktop
        MACOS -> p.key(SpecialKey.F11)                      // Show desktop
    }

    /** Three fingers sideways: the next or previous virtual desktop, following the fingers. */
    suspend fun switchDesktop(p: MousePadPlugin, right: Boolean) {
        val k = if (right) SpecialKey.RIGHT else SpecialKey.LEFT
        when (this) {
            PLASMA -> p.key(k, ctrl = true, meta = true)
            GNOME -> p.key(k, ctrl = true, alt = true)
            WINDOWS -> p.key(k, ctrl = true, meta = true)
            MACOS -> p.key(k, ctrl = true)
        }
    }

    /** Pinch: zoom the app under the pointer one step. */
    suspend fun zoom(p: MousePadPlugin, zoomIn: Boolean) {
        val c = if (zoomIn) "=" else "-"
        if (this == MACOS) p.chord(c, meta = true) else p.chord(c, ctrl = true)
    }

    fun overviewName() = when (this) { PLASMA -> "Overview"; GNOME -> "Activities"; WINDOWS -> "Task view"; MACOS -> "Mission Control" }
    fun desktopName() = when (this) { GNOME -> "Apps"; else -> "Desktop" }
}
