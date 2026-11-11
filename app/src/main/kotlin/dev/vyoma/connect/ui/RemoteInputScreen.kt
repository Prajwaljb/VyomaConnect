package dev.vyoma.connect.ui

import dev.vyoma.connect.protocol.plugin.builtin.PcFeature
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import androidx.compose.ui.platform.LocalContext

import androidx.compose.material.icons.rounded.AdsClick
import androidx.compose.material.icons.rounded.Slideshow
import androidx.compose.material.icons.rounded.SportsEsports

import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.material3.LocalContentColor
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.layout.layout
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.SmoothCornerShape
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.LineBreak
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.glassRaised
import dev.vyoma.connect.designsystem.glassCard
import dev.vyoma.connect.designsystem.aurora
import dev.vyoma.connect.designsystem.Glass
import dev.vyoma.connect.designsystem.OrbState
import dev.vyoma.connect.designsystem.ConnectionOrb
import androidx.compose.material3.Surface
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.ime
import androidx.compose.runtime.derivedStateOf
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.Backspace
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.rounded.KeyboardReturn
import androidx.compose.material.icons.automirrored.rounded.Redo
import androidx.compose.material.icons.automirrored.rounded.Undo
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.SelectAll
import androidx.compose.material.icons.rounded.SwapHoriz
import androidx.compose.material.icons.rounded.UnfoldMore
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import android.view.inputmethod.EditorInfo
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.platform.InterceptPlatformTextInput
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import kotlin.math.exp
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.pressable
import androidx.compose.material.icons.rounded.Check
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import dev.vyoma.connect.designsystem.MenuItem
import dev.vyoma.connect.designsystem.LocalMenu
import dev.vyoma.connect.designsystem.pressScale
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.perform
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalWindowInfo
import kotlinx.coroutines.flow.first
import dev.vyoma.connect.protocol.plugin.builtin.ClipboardPlugin
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.plugin.builtin.MousePadPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PointerChannel
import dev.vyoma.connect.service.BluetoothFallback
import dev.vyoma.connect.service.PointerRadio
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.vyoma.connect.presenter.AirMouseMath
import dev.vyoma.connect.presenter.PointerGyro
import dev.vyoma.connect.protocol.plugin.builtin.characterCount
import dev.vyoma.connect.protocol.plugin.builtin.commonEnds
import dev.vyoma.connect.protocol.plugin.builtin.typeable
import dev.vyoma.connect.protocol.plugin.builtin.typingRuns
import dev.vyoma.connect.protocol.plugin.builtin.SpecialKey
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.min
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * The phone as a precision touchpad and keyboard.
 *
 * One finger moves (with acceleration), tap clicks, two-finger tap right-clicks, three-finger
 * tap middle-clicks, two fingers scroll or pinch to zoom, three fingers swipe for the desktop's
 * overview, desktop and virtual desktops (see [DesktopKind]), and press-and-hold starts a drag. Motion is batched
 * into one packet per ~8 ms so a 120 Hz screen doesn't flood the link.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun RemoteInputScreen(vm: AppViewModel, deviceId: String?, startWithKeyboard: Boolean, onBack: () -> Unit, onPresenter: (String?) -> Unit = {}) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val target = devices.firstOrNull { it.identity.deviceId == deviceId && it.isReachable && it.isPaired }
        ?: devices.firstOrNull { it.isPaired && it.isReachable && !it.isCompanion }
    val pad: MousePadPlugin? = remember(target?.plugins) { target?.let { vm.device(it.identity.deviceId)?.plugin(MousePadPlugin.ID) } }
    // The PC's companion, for desktop gestures that know where the PC is.
    val companionDevice = devices.firstOrNull { it.identity.featuresOf == target?.identity?.deviceId && it.isReachable && it.isPaired }
    val companionPlugin: CompanionPlugin? = remember(companionDevice?.plugins) { companionDevice?.let { vm.device(it.identity.deviceId)?.plugin(CompanionPlugin.ID) } }
    // Where the three-finger ladder is, when only the phone keeps track (no companion, or not Plasma).
    var desktopView by remember { mutableStateOf(DesktopView.WINDOWS) }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current

    // The touchpad's own Bluetooth channel, for Wi-Fi that stalls (Settings > Steadier over Bluetooth). Never
    // on the cable, which beats it. While it's down, everything goes over the link as usual.
    val appContext = androidx.compose.ui.platform.LocalContext.current.applicationContext
    remember { BluetoothFallback.init(appContext) }
    val steadier by BluetoothFallback.steadier.collectAsStateWithLifecycle()
    val onCable = target?.transports?.contains(TransportKind.USB) == true
    val pointer by PointerRadio.current.collectAsStateWithLifecycle()
    val pageLifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(companionPlugin, steadier, onCable) {
        val companion = companionPlugin?.takeIf { it.offers(PcFeature.POINTER) }
        if (companion == null || !steadier || onCable) return@LaunchedEffect
        // While it's on screen (it idles at 10 frames a second, and hidden under the controller that
        // shares the radio with the controller's own frames); kept a while after, resting.
        pageLifecycle.repeatOnLifecycle(androidx.lifecycle.Lifecycle.State.STARTED) {
            PointerRadio.hold(appContext, companion)
        }
    }
    // Accumulated motion, flushed by the pump below.
    val motion = remember { FloatArray(4) } // dx, dy, scrollX, scrollY
    suspend fun flushMotion(p: MousePadPlugin) {
        val (dx, dy, sx, sy) = synchronized(motion) { motion.copyOf().also { motion.fill(0f) } }
        val radio = pointer
        if (dx != 0f || dy != 0f) { if (radio?.move(dx, dy) != true) p.move(dx, dy) }
        if (sx != 0f || sy != 0f) { if (radio?.scroll(sx, sy) != true) p.scroll(sx, sy) }
    }
    val flushLifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(pad) {
        val p = pad ?: return@LaunchedEffect
        // Not while hidden (under the gamepad, say): this wakes the main thread 125 times a second.
        flushLifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                delay(8)
                flushMotion(p)
            }
        }
    }
    // Air mouse: aim the phone and the pointer follows (the presenter's gyro, in pixels). The pad still
    // clicks, drags and scrolls meanwhile. Only while the page is on screen; off again when it's left.
    var airMouse by rememberSaveable { mutableStateOf(false) }
    val gyro = remember { PointerGyro(appContext, AirMouseMath::step) { dx, dy -> synchronized(motion) { motion[0] += dx; motion[1] += dy } } }
    LaunchedEffect(airMouse, pad) {
        if (!airMouse || pad == null) return@LaunchedEffect
        flushLifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            gyro.start()
            try { kotlinx.coroutines.awaitCancellation() } finally { gyro.stop() }
        }
    }
    // The phone keyboard types on the PC through the typing bar.
    var keyboard by rememberSaveable { mutableStateOf(startWithKeyboard) }
    var keyboardSession by remember { mutableIntStateOf(0) }
    // From the inset itself: WindowInsets.isImeVisible stays false on One UI even with Gboard up,
    // which left the key unable to close the keyboard. derivedStateOf: recompose on show/hide only.
    val imeInsets = WindowInsets.ime
    val density = LocalDensity.current
    val imeVisible by remember(imeInsets, density) { derivedStateOf { imeInsets.getBottom(density) > 0 } }
    val ime = LocalSoftwareKeyboardController.current
    val focusManager = LocalFocusManager.current
    // The system can hide the keyboard by itself (back gesture, the IME's own hide key). Close the
    // bar with it, so the key's highlight always matches what's on screen.
    var imeShown by remember { mutableStateOf(false) }
    LaunchedEffect(keyboard, keyboardSession) { imeShown = false }
    LaunchedEffect(imeVisible) {
        if (imeVisible) imeShown = true
        else if (keyboard && imeShown) keyboard = false
    }
    fun toggleKeyboard() {
        if (keyboard && imeVisible) {
            ime?.hide()
            focusManager.clearFocus()
            keyboard = false
        } else {
            keyboard = true
            keyboardSession++
        }
    }
    // Bumped when something other than the typing bar may move the PC's caret (a key, a click), so
    // the bar starts afresh instead of replaying edits against a caret that's moved.
    var caretMoved by remember { mutableIntStateOf(0) }
    fun act(block: suspend MousePadPlugin.() -> Unit) {
        if (keyboard) caretMoved++
        pad?.let { scope.launch { it.block() } }
    }
    // Clicks take the same road as the moves before them, or one could land before the cursor got there.
    fun clickButton(button: Int, overLink: suspend MousePadPlugin.() -> Unit) {
        if (keyboard) caretMoved++
        val p = pad ?: return
        scope.launch {
            flushMotion(p)
            if (pointer?.click(button) != true) p.overLink()
        }
    }
    // A drag lets go the way it took hold. If that channel has died, the PC lets go by itself.
    var dragVia by remember { mutableStateOf<PointerChannel?>(null) }
    fun grab() {
        val p = pad ?: return
        val radio = pointer
        scope.launch {
            flushMotion(p)
            if (radio?.button(0, true) == true) dragVia = radio else { dragVia = null; p.press() }
        }
    }
    fun letGo() {
        val p = pad ?: return
        val radio = dragVia
        dragVia = null
        scope.launch {
            flushMotion(p)
            if (radio == null) p.release() else radio.button(0, false)
        }
    }
    // One at a time: a paste waits on the PC, and what's typed next mustn't overtake it.
    val typingQueue = remember { kotlinx.coroutines.channels.Channel<suspend () -> Unit>(kotlinx.coroutines.channels.Channel.UNLIMITED) }
    LaunchedEffect(typingQueue) { for (job in typingQueue) runCatching { job() } }
    fun typing(block: suspend MousePadPlugin.() -> Unit) { pad?.let { p -> typingQueue.trySend { p.block() } } }
    // What a gesture just did ("Overview", "Next desktop"): shown for a moment in the header, in
    // place of the PC's name, instead of a toast over the pad (user 2026-11-04).
    var gestureNote by remember { mutableStateOf<String?>(null) }
    var noteJob by remember { mutableStateOf<kotlinx.coroutines.Job?>(null) }
    fun note(text: String) {
        gestureNote = text
        noteJob?.cancel()
        noteJob = scope.launch { delay(1_400); gestureNote = null }
    }

    /** Keys for what has keys; the rest by clipboard, since neither kdeconnectd nor KWin can type it. */
    fun sendTyped(text: String) = typing {
        for ((run, keyed) in typingRuns(text)) {
            if (keyed) { type(run); continue }
            target?.let { vm.device(it.identity.deviceId)?.plugin<ClipboardPlugin>(ClipboardPlugin.ID) }?.ignoreIncomingFor(PASTE_ECHO_MS)
            if (companionPlugin?.takeIf { it.offers(PcFeature.PASTE) }?.preparePaste(run) == true) {
                chord("v", ctrl = true)
                delay(PASTE_SETTLE_MS) // The app fetches the text on its own time; a key sent now could land first.
            } else {
                // It never arrived, so the field and the PC differ: start the field afresh.
                caretMoved++
                note("Couldn't type that on the PC")
            }
        }
    }

    // Surface, not a bare background: it supplies onBackground as the content color for the bar.
    Surface(Modifier.fillMaxSize().aurora(), color = Color.Transparent, contentColor = MaterialTheme.colorScheme.onBackground) { Column(
        Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).imePadding(),
    ) {
        Row(Modifier.fillMaxWidth().height(64.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            // Same round glass back button as every other page.
            // The whole disc dips as one piece, like the header capsules; no glyph bouncing inside a still ring.
            Box(
                Modifier.size(48.dp).pressable(glass = true, onClick = onBack).glassRaised(CircleShape),
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                // Named for how it was opened: the Keyboard tile shouldn't land on a screen called Touchpad.
                Text(if (startWithKeyboard) "Keyboard" else "Touchpad", style = MaterialTheme.typography.titleLarge)
                androidx.compose.animation.AnimatedContent(
                    gestureNote,
                    transitionSpec = {
                        (androidx.compose.animation.fadeIn(androidx.compose.animation.core.tween(160)) togetherWith
                            androidx.compose.animation.fadeOut(androidx.compose.animation.core.tween(160)))
                    },
                    label = "subtitle",
                ) { shown ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        if (shown != null) {
                            Text(shown, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
                        } else {
                            ConnectionOrb(if (target != null) OrbState.CONNECTED else OrbState.OFFLINE, size = 7.dp)
                            Text(target?.identity?.deviceName ?: "No PC connected", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                        }
                    }
                }
            }
            // Air mouse: a switch, so filled accent while on (never mistaken for a one-shot action).
            if (gyro.available && pad != null) {
                Box(
                    Modifier
                        .size(48.dp)
                        .pressable(glass = true) {
                            airMouse = !airMouse
                            haptics.perform(if (airMouse) Haptic.ToggleOn else Haptic.ToggleOff)
                            note(if (airMouse) "Aim the phone to move the pointer" else "Air mouse off")
                        }
                        .semantics { stateDescription = if (airMouse) "On" else "Off" }
                        .glassRaised(CircleShape, Glass.accent.takeIf { airMouse }),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.Rounded.AdsClick, "Air mouse", tint = if (airMouse) Color.White else LocalContentColor.current) }
                Spacer(Modifier.width(8.dp))
            }
            // Slides: keys and a pointer, so it works with plain KDE Connect too.
            Box(
                Modifier.size(48.dp).pressable(glass = true) { onPresenter(target?.identity?.deviceId) }.glassRaised(CircleShape),
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Rounded.Slideshow, "Presenter") }
            Spacer(Modifier.width(8.dp))
            // The phone as a game controller: the other way to drive the PC by hand (needs the companion).
            if (target != null && companionPlugin?.offers(PcFeature.GAMEPAD) == true) {
                val context = LocalContext.current
                Box(
                    Modifier.size(48.dp).pressable(glass = true) { GamepadActivity.open(context, target.identity.deviceId) }.glassRaised(CircleShape),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.Rounded.SportsEsports, "Game controller") }
            }
        }

        // ---- touch surface ----
        val dots = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.06f)
        // Light pools under the finger and flows after it (eased toward it each frame), lighting the
        // dots it passes. Drawn only while touched or fading: nothing runs at rest.
        val accent = MaterialTheme.colorScheme.primary
        // One pool of light per finger: each flows after its own finger and fades when it lifts.
        val fingers = remember { HashMap<Long, Offset>() }
        val pools = remember { HashMap<Long, LightPool>() }
        var touching by remember { mutableStateOf(false) }
        var frame by remember { mutableIntStateOf(0) }
        LaunchedEffect(touching) {
            var last = withFrameNanos { it }
            while (touching || pools.isNotEmpty()) {
                withFrameNanos { now ->
                    val dt = ((now - last) / 1e9f).coerceIn(0f, 0.05f)
                    last = now
                    for ((id, at) in fingers) pools.getOrPut(id) { LightPool(at) }
                    val gone = ArrayList<Long>()
                    for ((id, pool) in pools) {
                        val f = fingers[id]
                        if (f != null) pool.at += (f - pool.at) * (1f - exp(-dt * 14f))
                        pool.glow += ((if (f != null) 1f else 0f) - pool.glow) * (1f - exp(-dt * if (f != null) 16f else 5f))
                        if (f == null && pool.glow < 0.01f) gone += id
                    }
                    gone.forEach { pools.remove(it) }
                    frame++
                }
            }
        }
        val kind = remember { mutableStateOf(vm.desktopKind()) }
        Box(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .glassCard(SmoothCornerShape(32.dp))
                .pointerInput(pad, companionPlugin) {
                    val slop = viewConfiguration.touchSlop
                    awaitEachGesture {
                        try {
                            val down = awaitFirstDown(requireUnconsumed = false)
                            val start = down.uptimeMillis
                            fingers.clear()
                            fingers[down.id.value] = down.position
                            touching = true
                            var maxPointers = 1
                            var travelled = 0f
                            var dragging = false
                            var ended = false
                            // Two fingers: scroll or pinch, decided by whichever moves first, then kept.
                            var twoMode = 0 // 0 undecided, 1 scroll, 2 pinch
                            var spreadBase = -1f
                            // Three or more: one swipe per gesture, its direction from the fingers' travel.
                            var swipe = Offset.Zero
                            var swiped = false
                            val pinchSlop = 28.dp.toPx()
                            val swipeReach = 56.dp.toPx()
                            while (!ended) {
                                val event = withTimeoutOrNull(if (!dragging && travelled < slop && maxPointers == 1) 420L else Long.MAX_VALUE) { awaitPointerEvent() }
                                if (event == null) {
                                    // Held still: grab for drag-and-drop.
                                    if (travelled < slop && maxPointers == 1) {
                                        dragging = true
                                        haptics.perform(Haptic.Hold)
                                        grab()
                                    }
                                    continue
                                }
                                val pressed = event.changes.filter { it.pressed }
                                event.changes.forEach { c -> if (c.pressed) fingers[c.id.value] = c.position else fingers.remove(c.id.value) }
                                maxPointers = maxOf(maxPointers, pressed.size)
                                if (pressed.size == 1 && maxPointers == 1 || dragging && pressed.isNotEmpty()) {
                                    val d = pressed.first().positionChange()
                                    travelled += hypot(d.x, d.y)
                                    if (travelled > slop || dragging) {
                                        val a = accelerate(d)
                                        synchronized(motion) { motion[0] += a.x; motion[1] += a.y }
                                    }
                                } else if (pressed.size == 2 && maxPointers == 2) {
                                    val d = pressed.map { it.positionChange() }.reduce { acc, o -> acc + o } / 2f
                                    travelled += hypot(d.x, d.y)
                                    val spread = (pressed[0].position - pressed[1].position).getDistance()
                                    if (spreadBase < 0f) spreadBase = (pressed[0].previousPosition - pressed[1].previousPosition).getDistance()
                                    if (twoMode == 0) {
                                        if (abs(spread - spreadBase) > pinchSlop) twoMode = 2 else if (travelled > slop) twoMode = 1
                                    }
                                    if (twoMode == 1) synchronized(motion) { motion[2] += d.x * SCROLL_SCALE; motion[3] += d.y * SCROLL_SCALE }
                                    if (twoMode == 2 && spreadBase > 0f) {
                                        // A zoom step per quarter more (or less) spread, like a ratchet.
                                        val ratio = spread / spreadBase
                                        if (ratio > 1.25f || ratio < 0.8f) {
                                            val zoomIn = ratio > 1f
                                            spreadBase = spread
                                            haptics.perform(Haptic.Detent)
                                            val k = kind.value
                                            act { k.zoom(this, zoomIn) }
                                        }
                                    }
                                } else if (pressed.size >= 3) {
                                    val d = pressed.map { it.positionChange() }.reduce { acc, o -> acc + o } / pressed.size.toFloat()
                                    travelled += hypot(d.x, d.y)
                                    swipe += d
                                    if (!swiped && swipe.getDistance() > swipeReach) {
                                        swiped = true
                                        haptics.perform(Haptic.Edge)
                                        val k = kind.value
                                        if (abs(swipe.y) > abs(swipe.x)) {
                                            // Up and down step along the desktop's ladder (desktop, windows,
                                            // overview, grid), one level per swipe, as its own gestures do.
                                            val up = swipe.y < 0f
                                            val companion = companionPlugin?.takeIf { it.offers(PcFeature.DESKTOP_STEP) }
                                            scope.launch {
                                                val where = if (companion != null) {
                                                    // The PC asks its desktop where it is (KWin, or GNOME's shell), so
                                                    // it's right even after something changed on the PC itself.
                                                    companion.desktopStep(up).getOrNull()?.let(DesktopView::of)
                                                } else null
                                                val view = where ?: pad?.let { k.step(it, up, desktopView) } ?: desktopView
                                                desktopView = view
                                                note(k.viewName(view))
                                            }
                                        } else {
                                            // The desktops follow the fingers: swiping left brings the next one in from the right.
                                            val next = swipe.x < 0f
                                            // The PC's own switch where it has one (any desktop), else the shortcut.
                                            val companion = companionPlugin
                                            act { if (companion?.desktopSwitch(next) != true) k.switchDesktop(this, right = next) }
                                            note(if (next) "Next desktop" else "Previous desktop")
                                        }
                                    }
                                }
                                event.changes.forEach { it.consume() }
                                ended = event.changes.none { it.pressed }
                                if (ended) {
                                    touching = false
                                    fingers.clear()
                                    val duration = event.changes.first().uptimeMillis - start
                                    when {
                                        dragging -> {
                                            haptics.perform(Haptic.Detent)
                                            letGo()
                                        }
                                        travelled < slop && duration < 260 && twoMode != 2 && !swiped -> {
                                            haptics.perform(Haptic.Tap)
                                            when (maxPointers) {
                                                1 -> clickButton(0) { click() }
                                                2 -> clickButton(1) { rightClick() }
                                                else -> clickButton(2) { middleClick() }
                                            }
                                        }
                                    }
                                }
                            }
                        } finally {
                            // A gesture taken away (the page closing) ends the glow too.
                            touching = false
                            fingers.clear()
                        }
                    }
                },
        ) {
            Canvas(Modifier.fillMaxSize().clip(SmoothCornerShape(32.dp))) {
                val step = 28.dp.toPx()
                frame // redraws each frame the light moves
                val reach = 110.dp.toPx()
                val lights = pools.values.filter { it.glow > 0.01f }
                for (l in lights) {
                    drawCircle(
                        Brush.radialGradient(listOf(accent.copy(alpha = 0.22f * l.glow), accent.copy(alpha = 0.07f * l.glow), Color.Transparent), l.at, reach),
                        radius = reach, center = l.at,
                    )
                }
                var y = step
                while (y < size.height) {
                    var x = step
                    while (x < size.width) {
                        val c = Offset(x, y)
                        // Dots near the light catch it: brighter and a touch bigger.
                        var lit = 0f
                        for (l in lights) lit = maxOf(lit, (1f - (c - l.at).getDistance() / reach).coerceIn(0f, 1f) * l.glow)
                        if (lit > 0f) drawCircle(accent.copy(alpha = 0.06f + 0.5f * lit), radius = 1.6f + 1.4f * lit, center = c)
                        else drawCircle(dots, radius = 1.6f, center = c)
                        x += step
                    }
                    y += step
                }
            }
            if (pad == null) {
                Text("Connect a PC to use the touchpad", Modifier.align(Alignment.Center), color = MaterialTheme.colorScheme.onSurfaceVariant)
            } else {
                Column(Modifier.align(Alignment.BottomCenter).padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    if (airMouse) Text(
                        "Aim the phone to move the pointer · tap to click · hold to drag",
                        style = MaterialTheme.typography.labelSmall.copy(lineBreak = LineBreak.Heading, textAlign = TextAlign.Center),
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                    ) else Text(
                        "Tap to click · two fingers to scroll, pinch or right-click · three up or down for windows, desktop and ${kind.value.overviewName().lowercase()}, sideways between desktops · hold to drag",
                        // Balanced lines, centred: no lone "drag" on a line of its own.
                        style = MaterialTheme.typography.labelSmall.copy(lineBreak = LineBreak.Heading, textAlign = TextAlign.Center),
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                    )
                    // Which desktop's shortcuts the gestures send.
                    val menus = LocalMenu.current
                    var pill by remember { mutableStateOf(androidx.compose.ui.geometry.Rect.Zero) }
                    Text(
                        "Gestures for ${kind.value.label}",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier
                            .padding(top = 6.dp)
                            .onGloballyPositioned { pill = it.boundsInRoot() }
                            .pressable(onClick = {
                                menus.show(pill, DesktopKind.entries.map { k ->
                                    MenuItem(k.label, icon = if (k == kind.value) Icons.Rounded.Check else null) {
                                        kind.value = k
                                        vm.setDesktopKind(k)
                                    }
                                }, title = "Gestures for")
                            })
                            .padding(horizontal = 10.dp, vertical = 6.dp),
                    )
                }
            }
        }

        // ---- mouse buttons: the middle one is also a scroll strip ----
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).height(52.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            MouseButton("Left", Modifier.weight(2f)) { clickButton(0) { click() } }
            // A true circle in its share, like the keys (an icon-only capsule broke the shape rule).
            Box(Modifier.weight(1f).fillMaxHeight(), contentAlignment = Alignment.Center) {
                ScrollStrip(Modifier.circle(), onScroll = { dy -> synchronized(motion) { motion[3] += dy * SCROLL_SCALE } }) { act { middleClick() } }
            }
            MouseButton("Right", Modifier.weight(2f)) { clickButton(1) { rightClick() } }
        }

        // ---- keys: two fixed rows, an arrow cluster, and an Fn layer for the rarer ones ----
        var fn by rememberSaveable { mutableStateOf(false) }
        Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 10.dp, bottom = 10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(Modifier.fillMaxWidth().height(KEY_HEIGHT), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                if (!fn) {
                    Key("Esc") { act { key(SpecialKey.ESCAPE) } }
                    Key("Tab") { act { key(SpecialKey.TAB) } }
                    Key(icon = Icons.Rounded.ContentCopy, desc = "Copy") { act { chord("c", ctrl = true) } }
                    Key(icon = Icons.Rounded.ContentPaste, desc = "Paste") { act { chord("v", ctrl = true) } }
                } else {
                    Key("F5") { act { key(SpecialKey.F5) } }
                    Key("F11") { act { key(SpecialKey.F11) } }
                    Key("PrtSc") { act { key(SpecialKey.PRINT_SCREEN) } }
                    Key("Del") { act { key(SpecialKey.DELETE) } }
                }
                ClusterGap()
                if (!fn) {
                    Key(icon = Icons.AutoMirrored.Rounded.Backspace, desc = "Backspace", onRepeat = { n -> act { key(SpecialKey.BACKSPACE, ctrl = n >= WORD_AFTER) } }) { act { key(SpecialKey.BACKSPACE) } }
                    Key(icon = Icons.Rounded.KeyboardArrowUp, desc = "Up", onRepeat = { act { key(SpecialKey.UP) } }) { act { key(SpecialKey.UP) } }
                    Key(icon = Icons.AutoMirrored.Rounded.KeyboardReturn, desc = "Enter", accent = true) { act { key(SpecialKey.ENTER) } }
                } else {
                    Key("Home") { act { key(SpecialKey.HOME) } }
                    Key("PgUp") { act { key(SpecialKey.PAGE_UP) } }
                    Key("End") { act { key(SpecialKey.END) } }
                }
            }
            Row(Modifier.fillMaxWidth().height(KEY_HEIGHT), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Key("Fn", selected = fn) { fn = !fn }
                Key(icon = Icons.Rounded.Keyboard, desc = if (keyboard) "Hide keyboard" else "Type on PC", selected = keyboard, onClick = ::toggleKeyboard)
                if (!fn) {
                    Key(icon = Icons.Rounded.SwapHoriz, desc = "Alt+Tab") { act { key(SpecialKey.TAB, alt = true) } }
                    Key(icon = Icons.AutoMirrored.Rounded.Undo, desc = "Undo") { act { chord("z", ctrl = true) } }
                } else {
                    Key(icon = Icons.AutoMirrored.Rounded.Redo, desc = "Redo") { act { chord("z", ctrl = true, shift = true) } }
                    Key(icon = Icons.Rounded.SelectAll, desc = "Select all") { act { chord("a", ctrl = true) } }
                }
                ClusterGap()
                Key(icon = Icons.AutoMirrored.Rounded.KeyboardArrowLeft, desc = "Left", onRepeat = { n -> act { key(SpecialKey.LEFT, ctrl = n >= WORD_AFTER) } }) { act { key(SpecialKey.LEFT) } }
                if (!fn) Key(icon = Icons.Rounded.KeyboardArrowDown, desc = "Down", onRepeat = { act { key(SpecialKey.DOWN) } }) { act { key(SpecialKey.DOWN) } }
                else Key("PgDn") { act { key(SpecialKey.PAGE_DOWN) } }
                Key(icon = Icons.AutoMirrored.Rounded.KeyboardArrowRight, desc = "Right", onRepeat = { n -> act { key(SpecialKey.RIGHT, ctrl = n >= WORD_AFTER) } }) { act { key(SpecialKey.RIGHT) } }
            }
        }

        if (keyboard) key(keyboardSession) {
            TypingBar(
                caretMoved,
                canPaste = companionPlugin?.offers(PcFeature.PASTE) == true,
                onText = ::sendTyped,
                onKey = { k -> typing { key(k) } },
                onRefused = { note("Emoji need Vyoma on your PC") },
            )
        }
        Spacer(Modifier.navigationBarsPadding())
    } }
}

/**
 * A real text field for the phone keyboard, so Gboard keeps autocorrect, suggestions, swipe and
 * voice typing. The field mirrors the text being typed on the PC, and every change is replayed
 * there: the PC caret is moved to the edit with arrow keys, removed characters become
 * backspaces and new ones are typed. Caret moves alone (Gboard's space-bar swipe, tapping in the
 * field) become arrow presses, so the PC cursor follows. A leading sentinel space lets us see
 * backspace when the field looks empty. Send presses Enter and starts afresh.
 */
@Composable
private fun TypingBar(caretMoved: Int, canPaste: Boolean, onText: (String) -> Unit, onKey: (SpecialKey) -> Unit, onRefused: () -> Unit) {
    val sentinel = " "
    val fresh = TextFieldValue(sentinel, TextRange(sentinel.length))
    var value by remember { mutableStateOf(fresh) }
    // Where the PC's caret is, in this field's coordinates. Starts after the sentinel.
    var pcCaret by remember { mutableIntStateOf(sentinel.length) }
    val focus = remember { FocusRequester() }
    val ime = LocalSoftwareKeyboardController.current
    val window = LocalWindowInfo.current
    LaunchedEffect(Unit) {
        // Android drops a show request made before the window has focus (e.g. opening straight
        // into the keyboard from a tile), so wait for it.
        snapshotFlow { window.isWindowFocused }.first { it }
        focus.requestFocus()
        ime?.show()
    }
    LaunchedEffect(caretMoved) {
        if (caretMoved > 0) {
            value = fresh
            pcCaret = sentinel.length
        }
    }
    // The field never holds what the PC didn't get, so its text and the PC's stay the same.
    LaunchedEffect(canPaste) {
        if (!canPaste && !value.text.all(::typeable)) {
            value = fresh
            pcCaret = sentinel.length
        }
    }
    fun moveTo(text: String, target: Int) {
        val steps = characterCount(text.substring(minOf(pcCaret, target), maxOf(pcCaret, target)))
        repeat(steps) { onKey(if (target > pcCaret) SpecialKey.RIGHT else SpecialKey.LEFT) }
        pcCaret = target
    }
    fun typed(text: String) = text.split('\n').forEachIndexed { i, part ->
        if (i > 0) onKey(SpecialKey.ENTER)
        if (part.isNotEmpty()) onText(part)
    }
    NoKeyboardLearning {
        BasicTextField(
            value = value,
            onValueChange = { incoming ->
                var new = incoming
                if (!canPaste && !new.text.all(::typeable)) {
                    val before = new.text.take(new.selection.start).count(::typeable)
                    new = TextFieldValue(new.text.filter(::typeable), TextRange(before))
                    onRefused()
                }
                val old = value.text
                val cur = new.text
                if (old != cur) {
                    // The edit replaced old[prefix, old.length - suffix) with cur[prefix, cur.length - suffix).
                    val (prefix, suffix) = commonEnds(old, cur)
                    val end = old.length - suffix
                    moveTo(old, end)
                    repeat(characterCount(old.substring(prefix, end))) { onKey(SpecialKey.BACKSPACE) }
                    typed(cur.substring(prefix, cur.length - suffix))
                    pcCaret = cur.length - suffix
                }
                // Backspace at the very start ate the sentinel (and so text before ours on the PC).
                if (!new.text.startsWith(sentinel)) {
                    val shift = sentinel.length
                    new = new.copy(
                        text = sentinel + new.text,
                        selection = TextRange(new.selection.start + shift, new.selection.end + shift),
                        composition = new.composition?.let { TextRange(it.start + shift, it.end + shift) },
                    )
                    pcCaret += shift
                }
                // Keep the caret off the sentinel, then follow it on the PC (only a plain caret, not a range).
                if (new.selection.collapsed && new.selection.start < sentinel.length) new = new.copy(selection = TextRange(sentinel.length))
                if (new.selection.collapsed) moveTo(new.text, new.selection.start)
                value = when {
                    // Long runs reset at a word break when typing at the end, so the field stays light.
                    new.text.length > 400 && new.text.endsWith(' ') && new.selection.start == new.text.length -> fresh.also { pcCaret = sentinel.length }
                    else -> new
                }
            },
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Text,
                capitalization = KeyboardCapitalization.Sentences,
                autoCorrectEnabled = true,
                imeAction = ImeAction.Send,
            ),
            keyboardActions = KeyboardActions(onSend = {
                onKey(SpecialKey.ENTER)
                value = fresh
                pcCaret = sentinel.length
            }),
            singleLine = true,
            textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
            cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 6.dp)
                .focusRequester(focus),
            decorationBox = { field ->
                Box(
                    Modifier
                        .fillMaxWidth()
                        .glassRaised(SmoothCornerShape(20.dp))
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                ) {
                    if (value.text == sentinel) {
                        Text("Type on your PC…", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    field()
                }
            },
        )
    }
}

/**
 * Fields inside keep autocorrect and swiping, but the phone's keyboard doesn't learn from them or
 * sync what they hear: what's typed here goes to the PC, passwords included.
 */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
private fun NoKeyboardLearning(content: @Composable () -> Unit) {
    InterceptPlatformTextInput(
        interceptor = { request, next ->
            next.startInputMethod(
                PlatformTextInputMethodRequest { attributes ->
                    request.createInputConnection(attributes).also {
                        attributes.imeOptions = attributes.imeOptions or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
                    }
                },
            )
        },
        content = content,
    )
}

/** How long the phone ignores the PC's clipboard after a paste: the text itself, then what comes back. */
private const val PASTE_ECHO_MS = 3_000L
private const val PASTE_SETTLE_MS = 120L

@Composable
private fun MouseButton(label: String, modifier: Modifier, onClick: () -> Unit) {
    Box(
        modifier.fillMaxSize().pressable(glass = true, onClick = onClick).glassRaised(CircleShape),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface) }
}

/** A key that shares its row evenly; either a short label or an icon. */
@Composable
private fun RowScope.Key(
    label: String? = null,
    icon: ImageVector? = null,
    desc: String? = null,
    selected: Boolean = false,
    accent: Boolean = false,
    onRepeat: ((Int) -> Unit)? = null,
    onClick: () -> Unit,
) {
    val bg by animateColorAsState(
        when {
            selected -> Glass.accent
            else -> Glass.raised
        },
        VyomaMotion.effects(), label = "key",
    )
    // Only toggles fill (Fn, keyboard); Enter is marked by an accent glyph, so it never reads as "on".
    val fg = when {
        selected -> Color.White
        accent -> MaterialTheme.colorScheme.primary
        else -> MaterialTheme.colorScheme.onSurface
    }
    // Its share of the row, with the key itself a true circle in the middle (stretched to the
    // share, they were near-circles a dp or two off, which reads as a mistake).
    Box(Modifier.weight(1f).fillMaxHeight(), contentAlignment = Alignment.Center) {
        Box(
            Modifier
                .circle()
                .then(
                        // Every key ticks on touch-down, like a keyboard (the repeating ones must, to start
                    // repeating): ticking the others on release put two moments in one cluster.
                    if (onRepeat == null) Modifier.keyDownTick().pressable(haptic = null, glass = true, onClick = onClick)
                    else Modifier.holdToRepeat(onRepeat, onClick),
                )
                .glassRaised(CircleShape, bg.takeIf { selected }),
            contentAlignment = Alignment.Center,
        ) {
            if (icon != null) Icon(icon, desc, Modifier.size(20.dp), tint = fg)
            else Text(label.orEmpty(), style = if (label.orEmpty().length > 3) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge, color = fg, maxLines = 1)
        }
    }
}

/** The largest circle that fits the space given. */
private fun Modifier.circle() = layout { measurable, constraints ->
    val d = minOf(constraints.maxWidth, constraints.maxHeight)
    val p = measurable.measure(Constraints.fixed(d, d))
    layout(d, d) { p.place(0, 0) }
}

/**
 * Fires [onClick] on touch, then, while held, [onRepeat] with a growing count: after 350 ms, at
 * an accelerating rate (75 ms down to 30 ms), like a hardware key's auto-repeat. A firmer tick
 * marks the switch to whole words.
 */
/** [Haptic.Key] the moment a finger lands, without taking the gesture from the press handler. */
private fun Modifier.keyDownTick(): Modifier = composed {
    val haptics = LocalHapticFeedback.current
    pointerInput(Unit) {
        awaitEachGesture {
            awaitFirstDown(requireUnconsumed = false)
            haptics.perform(Haptic.Key)
        }
    }
}

private fun Modifier.holdToRepeat(onRepeat: (Int) -> Unit, onClick: () -> Unit): Modifier = composed {
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val repeat by rememberUpdatedState(onRepeat)
    val click by rememberUpdatedState(onClick)
    this
        .pressScale(1f, glass = true)
        // A button to TalkBack too (tap gestures alone expose nothing).
        .semantics { role = Role.Button; onClick { click(); true } }
        .pointerInput(Unit) {
            detectTapGestures(onPress = {
                haptics.perform(Haptic.Key)
                click()
                val repeater = scope.launch {
                    delay(350)
                    var n = 0
                    while (isActive) {
                        if (n == WORD_AFTER) haptics.perform(Haptic.Edge)
                        repeat(n++)
                        delay((75 - n * 3L).coerceAtLeast(30))
                    }
                }
                tryAwaitRelease()
                repeater.cancel()
            })
        }
}

/** Separates the arrow cluster from the shortcut keys. */
@Composable
private fun RowScope.ClusterGap() = Spacer(Modifier.width(6.dp))

/** Tap for a middle click; drag up or down to scroll, like a mouse wheel. */
@Composable
private fun ScrollStrip(modifier: Modifier, onScroll: (Float) -> Unit, onClick: () -> Unit) {
    val haptics = LocalHapticFeedback.current
    // The gesture below outlives this composition's lambdas: these hold the PC of the moment.
    val scroll by rememberUpdatedState(onScroll)
    val click by rememberUpdatedState(onClick)
    Box(
        modifier
            .fillMaxSize()
            .pressScale(1f, glass = true)
            .glassRaised(CircleShape)
            .semantics { role = Role.Button; contentDescription = "Middle click; drag to scroll"; onClick { onClick(); true } }
            .pointerInput(Unit) {
                val slop = viewConfiguration.touchSlop
                awaitEachGesture {
                    awaitFirstDown()
                    val notch = 22.dp.toPx()
                    var travelled = 0f
                    var wheel = 0f
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.first()
                        if (!change.pressed) break
                        val d = change.positionChange()
                        travelled += abs(d.y)
                        if (travelled > slop) {
                            scroll(d.y * 1.6f)
                            // A detent per notch, like a mouse wheel.
                            wheel += d.y
                            if (abs(wheel) >= notch) {
                                wheel %= notch
                                haptics.perform(Haptic.Detent)
                            }
                        }
                        change.consume()
                    }
                    if (travelled <= slop) {
                        haptics.perform(Haptic.Tap)
                        click()
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Rounded.UnfoldMore, "Middle click or scroll", tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

private val KEY_HEIGHT = 48.dp

/** Held arrows and backspace switch from characters to whole words after ~1 s of repeats. */
private const val WORD_AFTER = 14
private const val SCROLL_SCALE = 0.35f

/** Light pooled under one finger on the touchpad. */
private class LightPool(var at: Offset, var glow: Float = 0f)

/** Gentle pointer acceleration: slow strokes stay precise, flicks cross the screen. */
private fun accelerate(d: Offset): Offset {
    val speed = hypot(d.x, d.y)
    val gain = 0.9f * (1f + min(speed, 48f) / 20f)
    return Offset(d.x * gain, d.y * gain).let { if (abs(it.x) < 0.01f && abs(it.y) < 0.01f) Offset.Zero else it }
}
