package dev.vyoma.connect.mirror

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.accessibilityservice.GestureDescription.StrokeDescription
import android.content.ComponentName
import android.content.Context
import android.graphics.Path
import android.graphics.PointF
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.view.ViewConfiguration
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin.MirrorInput
import kotlin.math.hypot

/**
 * What lets the PC's mouse and keyboard work the phone while it's mirrored: Android only lets an
 * accessibility service touch the screen for an app (no root, no adb). The user switches it on once
 * in Settings > Accessibility; it does nothing but what the PC's mirror window sends.
 */
class MirrorAccessibility : AccessibilityService() {
    override fun onServiceConnected() { instance = this }
    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
    override fun onInterrupt() {}
    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    companion object {
        @Volatile var instance: MirrorAccessibility? = null
            private set

        /** Switched on in Settings (it may still be starting). */
        fun enabled(context: Context): Boolean {
            val me = ComponentName(context, MirrorAccessibility::class.java).flattenToString()
            val on = Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES).orEmpty()
            return on.split(':').any { it.equals(me, ignoreCase = true) }
        }
    }
}

/**
 * Turns the mirror window's inputs into touches and typing. Main thread only (gesture callbacks
 * come there too).
 *
 * A press becomes a tap unless the pointer moves past the touch slop or stays down
 * [HOLD_MS] (a long press): then it's a live stroke, sent in short continued pieces so the phone
 * follows the drag while it happens. Android runs one gesture at a time, so each piece waits for
 * the one before; points that arrive meanwhile join the next piece. The wheel is a short drag that
 * stops still before lifting, so it scrolls by that much without flinging.
 */
class MirrorControl(private val context: Context) {
    private val main = Handler(Looper.getMainLooper())
    private val slop = ViewConfiguration.get(context).scaledTouchSlop.toFloat()

    private var held = false
    private var start = PointF()
    private var downAt = 0L
    /** The live stroke's last piece sent (null: none yet, a tap so far). */
    private var live: StrokeDescription? = null
    private var lastEnd = PointF()
    private var lastSent = 0L
    private val pending = ArrayList<PointF>()
    private var ending = false
    private var busy = false
    private var scrollBy = PointF()
    private var scrollAt = PointF()

    private val holdCheck = Runnable { if (held && live == null && !busy) begin() }

    suspend fun perform(i: MirrorInput) = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Main) { handle(i) }

    private fun handle(i: MirrorInput) {
        val svc = MirrorAccessibility.instance ?: return
        when (i.kind) {
            "back" -> svc.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
            "home" -> svc.performGlobalAction(AccessibilityService.GLOBAL_ACTION_HOME)
            "recents" -> svc.performGlobalAction(AccessibilityService.GLOBAL_ACTION_RECENTS)
            "down" -> {
                reset()
                held = true
                start = point(i)
                downAt = SystemClock.uptimeMillis()
                main.postDelayed(holdCheck, HOLD_MS)
            }
            "move" -> {
                if (!held) return
                val p = point(i)
                if (live == null) {
                    if (hypot(p.x - start.x, p.y - start.y) < slop) return
                    main.removeCallbacks(holdCheck)
                    pending.add(p)
                    if (!busy) begin()
                } else {
                    pending.add(p)
                    pump()
                }
            }
            "up" -> {
                if (!held) return
                held = false
                main.removeCallbacks(holdCheck)
                if (live == null) {
                    // Never went live (a tap, or a quick flick while another gesture ran): all at once.
                    pending.add(point(i))
                    if (hypot(pending.last().x - start.x, pending.last().y - start.y) < slop) tap(svc) else flick(svc)
                    pending.clear()
                } else {
                    pending.add(point(i))
                    ending = true
                    pump()
                }
            }
            "scroll" -> {
                if (held || live != null) return
                val h = MirrorPointSize.height(context)
                scrollAt = point(i)
                // Wheel down shows what's further down: the finger moves up.
                scrollBy = PointF(scrollBy.x - i.dx * h * SCROLL_STEP, scrollBy.y - i.dy * h * SCROLL_STEP)
                if (!busy) scroll()
            }
            "text" -> i.text?.let { type(svc, it) }
            "key" -> when (i.key) {
                "backspace" -> erase(svc, forward = false)
                "delete" -> erase(svc, forward = true)
                "enter" -> enter(svc)
                "left" -> moveCursor(svc, -1)
                "right" -> moveCursor(svc, 1)
            }
        }
    }

    private fun point(i: MirrorInput): PointF = PhoneMirror.screenPoint(context, i.x, i.y).let { (x, y) -> PointF(x, y) }

    private fun reset() {
        main.removeCallbacks(holdCheck)
        held = false
        live = null
        pending.clear()
        ending = false
    }

    private fun tap(svc: AccessibilityService) {
        val held = (SystemClock.uptimeMillis() - downAt).coerceIn(1, HOLD_MS)
        val path = Path().apply { moveTo(start.x, start.y) }
        dispatch(svc, StrokeDescription(path, 0, held))
    }

    private fun flick(svc: AccessibilityService) {
        val ms = (SystemClock.uptimeMillis() - downAt).coerceIn(PIECE_MIN_MS, HOLD_MS)
        val path = Path().apply { moveTo(start.x, start.y); pending.forEach { lineTo(it.x, it.y) } }
        dispatch(svc, StrokeDescription(path, 0, ms))
    }

    /** The live stroke's first piece: down at the start, through whatever moves came already. */
    private fun begin() {
        val svc = MirrorAccessibility.instance ?: return
        val path = Path().apply { moveTo(start.x, start.y); pending.forEach { lineTo(it.x, it.y) } }
        lastEnd = pending.lastOrNull() ?: start
        pending.clear()
        val stroke = StrokeDescription(path, 0, PIECE_MIN_MS, true)
        live = stroke
        lastSent = SystemClock.uptimeMillis()
        dispatch(svc, stroke)
    }

    /** The next piece of the live stroke, once the last one is done. */
    private fun pump() {
        val prev = live ?: return
        if (busy || (pending.isEmpty() && !ending)) return
        val svc = MirrorAccessibility.instance ?: return reset()
        val path = Path().apply { moveTo(lastEnd.x, lastEnd.y); pending.forEach { lineTo(it.x, it.y) } }
        lastEnd = pending.lastOrNull() ?: lastEnd
        pending.clear()
        val now = SystemClock.uptimeMillis()
        val ms = (now - lastSent).coerceIn(PIECE_MIN_MS, PIECE_MAX_MS)
        lastSent = now
        val last = ending
        val stroke = runCatching { prev.continueStroke(path, 0, ms, !last) }.getOrElse { reset(); return }
        live = if (last) null else stroke
        if (last) ending = false
        dispatch(svc, stroke)
    }

    private fun scroll() {
        val svc = MirrorAccessibility.instance ?: return
        if (scrollBy.x == 0f && scrollBy.y == 0f) return
        val (w, h) = MirrorPointSize.width(context) to MirrorPointSize.height(context)
        val from = scrollAt
        val to = PointF((from.x + scrollBy.x).coerceIn(1f, w - 2f), (from.y + scrollBy.y).coerceIn(1f, h - 2f))
        scrollBy = PointF()
        val move = StrokeDescription(Path().apply { moveTo(from.x, from.y); lineTo(to.x, to.y) }, 0, SCROLL_MS, true)
        // Then still for a moment before lifting: no fling, it stops where the wheel said.
        val rest = move.continueStroke(Path().apply { moveTo(to.x, to.y) }, 0, REST_MS, false)
        dispatch(svc, move) { dispatch(svc, rest) { scroll() } }
    }

    private fun dispatch(svc: AccessibilityService, stroke: StrokeDescription, then: (() -> Unit)? = null) {
        busy = true
        val ok = svc.dispatchGesture(GestureDescription.Builder().addStroke(stroke).build(), object : AccessibilityService.GestureResultCallback() {
            override fun onCompleted(g: GestureDescription) {
                busy = false
                if (then != null) then()
                else if (live != null || ending) pump()
                else if (held && live == null && pending.isNotEmpty()) begin()
            }
            override fun onCancelled(g: GestureDescription) {
                busy = false
                VLog.d(TAG, "gesture cancelled")
                live = null
                pending.clear()
                ending = false
            }
        }, main)
        if (!ok) {
            busy = false
            live = null
        }
    }

    // ---- typing: into the focused text field, through the accessibility node ----

    private fun field(svc: AccessibilityService): AccessibilityNodeInfo? =
        svc.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.takeIf { it.isEditable }

    private class Field(val text: String, val from: Int, val to: Int)

    private fun read(node: AccessibilityNodeInfo): Field {
        val text = if (node.isShowingHintText) "" else node.text?.toString().orEmpty()
        var a = node.textSelectionStart
        var b = node.textSelectionEnd
        if (a < 0 || b < 0 || a > text.length || b > text.length) { a = text.length; b = text.length }
        return Field(text, minOf(a, b), maxOf(a, b))
    }

    private fun write(node: AccessibilityNodeInfo, text: String, cursor: Int) {
        node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text)
        })
        select(node, cursor)
    }

    private fun select(node: AccessibilityNodeInfo, at: Int) {
        node.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, at)
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, at)
        })
    }

    private fun type(svc: AccessibilityService, typed: String) {
        val node = field(svc) ?: return
        val f = read(node)
        write(node, f.text.substring(0, f.from) + typed + f.text.substring(f.to), f.from + typed.length)
    }

    private fun erase(svc: AccessibilityService, forward: Boolean) {
        val node = field(svc) ?: return
        val f = read(node)
        val (a, b) = when {
            f.from != f.to -> f.from to f.to
            forward && f.to < f.text.length -> f.from to f.text.offsetByCodePoints(f.to, 1)
            !forward && f.from > 0 -> f.text.offsetByCodePoints(f.from, -1) to f.to
            else -> return
        }
        write(node, f.text.substring(0, a) + f.text.substring(b), a)
    }

    private fun moveCursor(svc: AccessibilityService, by: Int) {
        val node = field(svc) ?: return
        val f = read(node)
        val at = when {
            f.from != f.to -> if (by < 0) f.from else f.to
            by < 0 && f.from > 0 -> f.text.offsetByCodePoints(f.from, -1)
            by > 0 && f.to < f.text.length -> f.text.offsetByCodePoints(f.to, 1)
            else -> return
        }
        select(node, at)
    }

    /** Enter: the field's own action (send, search, next) if it has one, else a new line. */
    private fun enter(svc: AccessibilityService) {
        val node = field(svc) ?: return
        val ime = AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER
        if (node.actionList.any { it.id == ime.id }) node.performAction(ime.id)
        else if (node.isMultiLine) type(svc, "\n")
    }

    private companion object {
        const val TAG = "MirrorControl"
        /** Held this long without moving: a long press (Android's is 400-500 ms; ours starts it). */
        const val HOLD_MS = 300L
        const val PIECE_MIN_MS = 8L
        const val PIECE_MAX_MS = 60L
        /** One wheel click scrolls this share of the screen's height. */
        const val SCROLL_STEP = 0.08f
        const val SCROLL_MS = 90L
        const val REST_MS = 60L
    }
}

/** The phone screen's size right now (it turns), for gestures. */
private object MirrorPointSize {
    fun width(context: Context) = PhoneMirror.screenPoint(context, 1f, 0f).first + 1f
    fun height(context: Context) = PhoneMirror.screenPoint(context, 0f, 1f).second + 1f
}
