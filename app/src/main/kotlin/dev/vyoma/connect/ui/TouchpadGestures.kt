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

    /**
     * Three fingers up or down, one step along the desktop's own ladder, as its touchpad gestures go:
     * desktop → windows → overview → grid (apps on GNOME) going up, and back down again. Each
     * shortcut toggles, so which one to press depends on where the PC is: without the companion
     * (which asks KWin) that's [from], remembered here. Returns where it ends up.
     */
    suspend fun step(p: MousePadPlugin, up: Boolean, from: DesktopView): DesktopView {
        val v = DesktopView.entries
        val to = if (up) v.getOrElse(from.ordinal + 1) { from } else v.getOrElse(from.ordinal - 1) { from }
        // At the top or bottom already: pressing anything would toggle back out.
        if (to == from) return from
        val target = when {
            to == DesktopView.GRID && this in setOf(WINDOWS, MACOS) -> return from // no grid there
            to == DesktopView.DESKTOP && this == GNOME -> return from              // no desktop view by default
            else -> to
        }
        when (this) {
            PLASMA -> when (if (up) target else from) {
                DesktopView.WINDOWS, DesktopView.DESKTOP -> p.chord("d", meta = true) // Peek at desktop (toggles)
                DesktopView.OVERVIEW -> p.chord("w", meta = true)                     // Overview (toggles; from Grid it steps back)
                DesktopView.GRID -> if (up) p.chord("g", meta = true) else p.chord("w", meta = true)
            }
            GNOME -> if (up) {
                if (target == DesktopView.OVERVIEW) p.chord("s", meta = true) else p.chord("a", meta = true) // Activities, then the app grid
            } else p.key(SpecialKey.ESCAPE)                                                                    // one level back
            WINDOWS -> when {
                up && target == DesktopView.WINDOWS -> p.chord("d", meta = true)       // Show desktop (toggles)
                up -> p.key(SpecialKey.TAB, meta = true)                               // Task view
                from == DesktopView.OVERVIEW -> p.key(SpecialKey.ESCAPE)
                else -> p.chord("d", meta = true)
            }
            MACOS -> when {
                up && target == DesktopView.WINDOWS -> p.key(SpecialKey.F11)           // Show desktop (toggles)
                up -> p.key(SpecialKey.UP, ctrl = true)                                // Mission Control
                from == DesktopView.OVERVIEW -> p.key(SpecialKey.UP, ctrl = true)
                else -> p.key(SpecialKey.F11)
            }
        }
        return target
    }

    fun viewName(v: DesktopView) = when (v) {
        DesktopView.DESKTOP -> "Desktop"
        DesktopView.WINDOWS -> "Windows"
        DesktopView.OVERVIEW -> overviewName()
        DesktopView.GRID -> if (this == GNOME) "Apps" else "Grid"
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
}

/** Where the PC's desktop is, bottom to top of the three-finger ladder. */
enum class DesktopView { DESKTOP, WINDOWS, OVERVIEW, GRID;
    companion object {
        /** As the companion names them (vyoma.desktop.stepped). */
        fun of(name: String) = entries.firstOrNull { it.name.equals(name, ignoreCase = true) }
    }
}
