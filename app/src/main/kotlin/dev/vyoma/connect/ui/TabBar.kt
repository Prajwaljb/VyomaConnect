package dev.vyoma.connect.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import dev.vyoma.connect.designsystem.Backdrop
import dev.vyoma.connect.designsystem.Glass
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.LocalBottomBarInset
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.backdropSource
import dev.vyoma.connect.designsystem.glassChrome
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.designsystem.rememberBackdrop
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.drawscope.DrawScope
import dev.vyoma.connect.designsystem.liquidGlassSupported
import dev.vyoma.connect.designsystem.liquidLens
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlin.math.abs
import kotlinx.coroutines.launch

enum class Tab(val label: String, val icon: ImageVector) {
    HOME("Home", Icons.Rounded.Home),
    CLIPBOARD("Clipboard", Icons.Rounded.ContentPaste),
    SETTINGS("Settings", Icons.Rounded.Settings),
}

private val BarHeight = 62.dp
private val ItemWidth = 82.dp

/**
 * The root: tab content under a floating glass tab bar, with the touchpad as its own round button
 * beside it. Tabs stay composed once visited (and the others are warmed up shortly after start),
 * measured but not placed while hidden: a switch only places and draws, never composes a whole
 * screen in one frame. Hidden tabs aren't drawn, so they cost nothing per frame.
 */
@Composable
fun TabsRoot(tab: Tab, onTab: (Tab) -> Unit, onTouchpad: () -> Unit, content: @Composable (Tab) -> Unit) {
    val backdrop = rememberBackdrop()
    val holder = rememberSaveableStateHolder()
    val navBottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val visited = remember { mutableStateListOf(tab) }
    if (tab !in visited) visited.add(tab)
    LaunchedEffect(Unit) {
        delay(800)
        Tab.entries.forEach { if (it !in visited) visited.add(it) }
    }
    // Pages drift a little in the direction you moved along the bar.
    val travel = remember { object { var last = tab; var dir = 1 } }
    if (travel.last != tab) {
        travel.dir = if (tab.ordinal > travel.last.ordinal) 1 else -1
        travel.last = tab
    }
    Box(Modifier.fillMaxSize()) {
        CompositionLocalProvider(LocalBottomBarInset provides BarHeight + 12.dp) {
            Box(Modifier.fillMaxSize().backdropSource(backdrop)) {
                for (t in Tab.entries) {
                    if (t !in visited) continue
                    key(t) {
                        val active = t == tab
                        val shown = remember { Animatable(if (active) 1f else 0f) }
                        LaunchedEffect(active) { shown.animateTo(if (active) 1f else 0f, if (active) VyomaMotion.spatial() else VyomaMotion.effects()) }
                        Box(
                            Modifier.fillMaxSize().layout { measurable, constraints ->
                                val page = measurable.measure(constraints)
                                layout(page.width, page.height) {
                                    if (!active && shown.value < 0.01f) return@layout
                                    page.placeWithLayer(0, 0) {
                                        val p = shown.value
                                        alpha = p.coerceIn(0f, 1f)
                                        translationX = (1f - p) * size.width / 12 * (if (active) travel.dir else -travel.dir)
                                    }
                                }
                            },
                        ) { holder.SaveableStateProvider(t.name) { content(t) } }
                    }
                }
            }
        }

        Row(
            Modifier.align(Alignment.BottomCenter).padding(bottom = navBottom + 10.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TabBar(tab, onTab, backdrop)
            Box(
                Modifier.size(BarHeight).pressable(pressedScale = 0.86f, onClick = onTouchpad).glassChrome(backdrop, CircleShape, blur = 8.dp, bevel = 16.dp),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.TouchApp, "Touchpad", tint = MaterialTheme.colorScheme.onBackground)
            }
        }
    }
}

/**
 * Glass pill with a liquid selection lens. The lens's leading edge leaves first and the trailing
 * edge catches up, so it stretches like a droplet in flight and thins as it does. Under a finger
 * (and briefly on every switch) it lifts into a clear drop of water a little bigger than the bar,
 * magnifying the tab under it. Drag along the bar to scrub between tabs (a tick per tab, like iOS);
 * letting go picks the one under your thumb. All of it runs in the draw phase.
 */
@Composable
private fun TabBar(tab: Tab, onTab: (Tab) -> Unit, backdrop: Backdrop) {
    val density = LocalDensity.current
    val itemPx = with(density) { ItemWidth.toPx() }
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val currentTab by rememberUpdatedState(tab)
    val select by rememberUpdatedState(onTab)
    val left = remember { Animatable(tab.ordinal * itemPx) }
    val right = remember { Animatable((tab.ordinal + 1) * itemPx) }
    val lift = remember { Animatable(0f) }
    var hover by remember { mutableIntStateOf(-1) }
    var pulse by remember { mutableStateOf<Job?>(null) }

    fun glide(toLeft: Float) {
        val forward = toLeft > left.targetValue
        scope.launch { left.animateTo(toLeft, if (forward) VyomaMotion.trailing() else VyomaMotion.leading()) }
        scope.launch { right.animateTo(toLeft + itemPx, if (forward) VyomaMotion.leading() else VyomaMotion.trailing()) }
    }
    fun lifted(up: Boolean) {
        pulse?.cancel()
        pulse = scope.launch { lift.animateTo(if (up) 1f else 0f, if (up) VyomaMotion.spatial() else VyomaMotion.release()) }
    }
    LaunchedEffect(tab) {
        if (hover >= 0) return@LaunchedEffect
        val far = abs(left.value - tab.ordinal * itemPx) > itemPx / 2
        glide(tab.ordinal * itemPx)
        // A tap lifts the drop for the trip and sets it down as it arrives.
        if (far && liquidGlassSupported) {
            pulse?.cancel()
            pulse = scope.launch {
                lift.animateTo(1f, VyomaMotion.leading())
                lift.animateTo(0f, VyomaMotion.release())
            }
        }
    }

    // The pill in bar coordinates: resting it sits inside the padding; lifted it outgrows the bar.
    val pad = with(density) { 4.dp.toPx() }
    val overflow = with(density) { 7.dp.toPx() }
    fun DrawScope.pill(): Rect {
        val l = left.value
        val w = right.value - l
        val stretch = (w / itemPx - 1f).coerceIn(0f, 1f)
        val up = lift.value
        val rest = size.height - 2 * pad
        val h = (rest + (2 * overflow + 2 * pad) * up) * (1f - 0.14f * stretch)
        val ww = w + 2 * overflow * up.coerceAtLeast(0f) * 1.6f + rest * 0.06f * up.coerceAtMost(0f)
        val cx = pad + l + w / 2
        return Rect(Offset(cx - ww / 2, (size.height - h) / 2), Size(ww, h))
    }

    val fill = Glass.raised
    val edge = Glass.edge
    Box(
        Modifier
            .height(BarHeight)
            .liquidLens(backdrop, strength = { lift.value }, zoom = { 1f + 0.22f * lift.value.coerceIn(0f, 1f) }, rect = { pill() })
            .glassChrome(backdrop, CircleShape, blur = 8.dp, bevel = 16.dp)
            .drawBehind {
                // The resting pill fades as the clear drop takes its place (where drops can be drawn).
                val a = if (liquidGlassSupported) 1f - lift.value.coerceIn(0f, 1f) else 1f
                if (a < 0.01f) return@drawBehind
                val r = pill()
                val corner = CornerRadius(r.height / 2)
                drawRoundRect(fill, r.topLeft, r.size, corner, alpha = a)
                drawRoundRect(edge, r.topLeft, r.size, corner, alpha = a, style = Stroke(0.75.dp.toPx()))
            }
            .padding(4.dp)
            .pointerInput(itemPx) {
                val last = Tab.entries.lastIndex
                fun indexAt(x: Float) = (x / itemPx).toInt().coerceIn(0, last)
                fun follow(x: Float) = glide((x - itemPx / 2).coerceIn(0f, last * itemPx))
                detectHorizontalDragGestures(
                    onDragStart = { o ->
                        hover = indexAt(o.x)
                        lifted(true)
                        follow(o.x)
                    },
                    onHorizontalDrag = { change, _ ->
                        change.consume()
                        follow(change.position.x)
                        val i = indexAt(change.position.x)
                        if (i != hover) {
                            hover = i
                            haptics.perform(Haptic.Select)
                        }
                    },
                    onDragEnd = {
                        val i = hover
                        hover = -1
                        lifted(false)
                        if (i != currentTab.ordinal) select(Tab.entries[i]) else glide(i * itemPx)
                    },
                    onDragCancel = {
                        hover = -1
                        lifted(false)
                        glide(currentTab.ordinal * itemPx)
                    },
                )
            },
    ) {
        Row {
            Tab.entries.forEach { t ->
                TabItem(t, selected = if (hover >= 0) t.ordinal == hover else t == tab) {
                    if (t != tab) onTab(t)
                }
            }
        }
    }
}

@Composable
private fun TabItem(tab: Tab, selected: Boolean, onClick: () -> Unit) {
    val color by animateColorAsState(
        if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onBackground,
        VyomaMotion.effects(), label = "tabColor",
    )
    Column(
        Modifier.width(ItemWidth).fillMaxHeight().pressable(pressedScale = 0.86f, haptic = Haptic.Select, onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(tab.icon, null, tint = color, modifier = Modifier.size(24.dp))
        Spacer(Modifier.height(2.dp))
        Text(tab.label, style = MaterialTheme.typography.labelSmall, color = color)
    }
}
