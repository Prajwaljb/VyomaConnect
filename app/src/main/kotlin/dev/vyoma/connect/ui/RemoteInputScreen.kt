package dev.vyoma.connect.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.KeyboardHide
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.protocol.plugin.builtin.MousePadPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SpecialKey
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.min

/**
 * The phone as a precision touchpad and keyboard.
 *
 * One finger moves (with acceleration), tap clicks, two-finger tap right-clicks, three-finger
 * tap middle-clicks, two fingers scroll, and press-and-hold starts a drag. Motion is batched
 * into one packet per ~8 ms so a 120 Hz screen doesn't flood the link.
 */
@Composable
fun RemoteInputScreen(vm: AppViewModel, deviceId: String?, startWithKeyboard: Boolean, onBack: () -> Unit) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val target = devices.firstOrNull { it.identity.deviceId == deviceId && it.isReachable && it.isPaired }
        ?: devices.firstOrNull { it.isPaired && it.isReachable }
    val pad: MousePadPlugin? = remember(target?.plugins) { target?.let { vm.device(it.identity.deviceId)?.plugin(MousePadPlugin.ID) } }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current

    // Accumulated motion, flushed by the pump below.
    val motion = remember { FloatArray(4) } // dx, dy, scrollX, scrollY
    LaunchedEffect(pad) {
        val p = pad ?: return@LaunchedEffect
        while (isActive) {
            delay(8)
            val (dx, dy, sx, sy) = synchronized(motion) { motion.copyOf().also { motion.fill(0f) } }
            if (dx != 0f || dy != 0f) p.move(dx, dy)
            if (sx != 0f || sy != 0f) p.scroll(sx, sy)
        }
    }
    fun act(block: suspend MousePadPlugin.() -> Unit) { pad?.let { scope.launch { it.block() } } }

    var keyboard by rememberSaveable { mutableStateOf(startWithKeyboard) }

    Column(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).windowInsetsPadding(WindowInsets.statusBars).imePadding(),
    ) {
        Row(Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") }
            Column(Modifier.weight(1f)) {
                Text("Touchpad", style = MaterialTheme.typography.titleLarge)
                Text(target?.identity?.deviceName ?: "No PC connected", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            IconButton(onClick = { keyboard = !keyboard }) {
                Icon(if (keyboard) Icons.Rounded.KeyboardHide else Icons.Rounded.Keyboard, if (keyboard) "Hide keyboard" else "Keyboard")
            }
        }

        // ---- touch surface ----
        val surface = MaterialTheme.colorScheme.surfaceContainerHigh
        val dots = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.06f)
        Box(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .clip(RoundedCornerShape(32.dp))
                .background(surface)
                .pointerInput(pad) {
                    val slop = viewConfiguration.touchSlop
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        val start = down.uptimeMillis
                        var maxPointers = 1
                        var travelled = 0f
                        var dragging = false
                        var ended = false
                        while (!ended) {
                            val event = withTimeoutOrNull(if (!dragging && travelled < slop && maxPointers == 1) 420L else Long.MAX_VALUE) { awaitPointerEvent() }
                            if (event == null) {
                                // Held still: grab for drag-and-drop.
                                if (travelled < slop && maxPointers == 1) {
                                    dragging = true
                                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                    act { press() }
                                }
                                continue
                            }
                            val pressed = event.changes.filter { it.pressed }
                            maxPointers = maxOf(maxPointers, pressed.size)
                            if (pressed.size == 1 && maxPointers == 1 || dragging && pressed.isNotEmpty()) {
                                val d = pressed.first().positionChange()
                                travelled += hypot(d.x, d.y)
                                if (travelled > slop || dragging) {
                                    val a = accelerate(d)
                                    synchronized(motion) { motion[0] += a.x; motion[1] += a.y }
                                }
                            } else if (pressed.size >= 2) {
                                val d = pressed.map { it.positionChange() }.reduce { acc, o -> acc + o } / pressed.size.toFloat()
                                travelled += hypot(d.x, d.y)
                                if (travelled > slop) synchronized(motion) { motion[2] += -d.x * SCROLL_SCALE; motion[3] += -d.y * SCROLL_SCALE }
                            }
                            event.changes.forEach { it.consume() }
                            ended = event.changes.none { it.pressed }
                            if (ended) {
                                val duration = event.changes.first().uptimeMillis - start
                                when {
                                    dragging -> act { release() }
                                    travelled < slop && duration < 260 -> {
                                        haptics.performHapticFeedback(HapticFeedbackType.ContextClick)
                                        when (maxPointers) {
                                            1 -> act { click() }
                                            2 -> act { rightClick() }
                                            else -> act { middleClick() }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
        ) {
            Canvas(Modifier.fillMaxSize()) {
                val step = 28.dp.toPx()
                var y = step
                while (y < size.height) {
                    var x = step
                    while (x < size.width) {
                        drawCircle(dots, radius = 1.6f, center = Offset(x, y))
                        x += step
                    }
                    y += step
                }
            }
            if (pad == null) {
                Text("Connect a PC to use the touchpad", Modifier.align(Alignment.Center), color = MaterialTheme.colorScheme.onSurfaceVariant)
            } else {
                Text(
                    "Tap to click · two fingers to scroll or right-click · hold to drag",
                    Modifier.align(Alignment.BottomCenter).padding(16.dp),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                )
            }
        }

        // ---- mouse buttons ----
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).height(56.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            MouseButton("Left", Modifier.weight(2f)) { act { click() } }
            MouseButton("Middle", Modifier.weight(1f)) { act { middleClick() } }
            MouseButton("Right", Modifier.weight(2f)) { act { rightClick() } }
        }

        // ---- special keys ----
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 10.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Key("Esc") { act { key(SpecialKey.ESCAPE) } }
            Key("Tab") { act { key(SpecialKey.TAB) } }
            Key("←") { act { key(SpecialKey.LEFT) } }
            Key("↑") { act { key(SpecialKey.UP) } }
            Key("↓") { act { key(SpecialKey.DOWN) } }
            Key("→") { act { key(SpecialKey.RIGHT) } }
            Key("⌫") { act { key(SpecialKey.BACKSPACE) } }
            Key("Enter") { act { key(SpecialKey.ENTER) } }
            Key("Copy") { act { chord("c", ctrl = true) } }
            Key("Paste") { act { chord("v", ctrl = true) } }
            Key("Undo") { act { chord("z", ctrl = true) } }
            Key("Alt+Tab") { act { key(SpecialKey.TAB, alt = true) } }
            Key("F5") { act { key(SpecialKey.F5) } }
            Key("F11") { act { key(SpecialKey.F11) } }
            Key("PrtSc") { act { key(SpecialKey.PRINT_SCREEN) } }
            Key("Del") { act { key(SpecialKey.DELETE) } }
        }

        if (keyboard) KeyboardSink(onText = { t -> act { type(t) } }, onBackspace = { act { key(SpecialKey.BACKSPACE) } }, onEnter = { act { key(SpecialKey.ENTER) } })
        Spacer(Modifier.navigationBarsPadding())
    }
}

/**
 * An invisible text field that turns IME input into keystrokes. It keeps one sentinel char so
 * backspace on an "empty" field is still observable, and works with swipe/voice typing too.
 */
@Composable
private fun KeyboardSink(onText: (String) -> Unit, onBackspace: () -> Unit, onEnter: () -> Unit) {
    val sentinel = " "
    var value by remember { mutableStateOf(TextFieldValue(sentinel, TextRange(1))) }
    val focus = remember { FocusRequester() }
    val ime = LocalSoftwareKeyboardController.current
    LaunchedEffect(Unit) {
        focus.requestFocus()
        ime?.show()
    }
    BasicTextField(
        value = value,
        onValueChange = { new ->
            val text = new.text
            when {
                text.length < sentinel.length -> repeat(sentinel.length - text.length) { onBackspace() }
                text.startsWith(sentinel) && text.length > sentinel.length -> {
                    val typed = text.substring(sentinel.length)
                    typed.split('\n').forEachIndexed { i, part ->
                        if (i > 0) onEnter()
                        if (part.isNotEmpty()) onText(part)
                    }
                }
                text != sentinel -> onText(text)
            }
            value = TextFieldValue(sentinel, TextRange(1))
        },
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Text, imeAction = ImeAction.Send, autoCorrectEnabled = false),
        keyboardActions = KeyboardActions(onSend = { onEnter() }),
        modifier = Modifier.size(1.dp).focusRequester(focus),
        textStyle = MaterialTheme.typography.bodySmall.copy(color = Color.Transparent),
    )
}

@Composable
private fun MouseButton(label: String, modifier: Modifier, onClick: () -> Unit) {
    Box(
        modifier.fillMaxSize().clip(RoundedCornerShape(20.dp)).background(MaterialTheme.colorScheme.surfaceContainerHigh).pressable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant) }
}

@Composable
private fun Key(label: String, onClick: () -> Unit) {
    Text(
        label,
        style = MaterialTheme.typography.labelLarge,
        modifier = Modifier
            .clip(RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .pressable(pressedScale = 0.92f, onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 10.dp),
    )
}

private const val SCROLL_SCALE = 0.35f

/** Gentle pointer acceleration: slow strokes stay precise, flicks cross the screen. */
private fun accelerate(d: Offset): Offset {
    val speed = hypot(d.x, d.y)
    val gain = 0.9f * (1f + min(speed, 48f) / 20f)
    return Offset(d.x * gain, d.y * gain).let { if (abs(it.x) < 0.01f && abs(it.y) < 0.01f) Offset.Zero else it }
}
