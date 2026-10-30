package dev.vyoma.connect.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
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
import androidx.compose.runtime.mutableIntStateOf
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
 * beside it. Only the selected tab is composed; the others keep their saved state (scroll
 * position included), so switching is instant and idle tabs cost nothing.
 */
@Composable
fun TabsRoot(tab: Tab, onTab: (Tab) -> Unit, onTouchpad: () -> Unit, content: @Composable (Tab) -> Unit) {
    val backdrop = rememberBackdrop()
    val holder = rememberSaveableStateHolder()
    val navBottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    Box(Modifier.fillMaxSize()) {
        CompositionLocalProvider(LocalBottomBarInset provides BarHeight + 12.dp) {
            Box(Modifier.fillMaxSize().backdropSource(backdrop)) {
                AnimatedContent(
                    tab,
                    transitionSpec = {
                        // Pages drift a little in the direction you moved along the bar.
                        val dir = if (targetState.ordinal > initialState.ordinal) 1 else -1
                        (slideInHorizontally(VyomaMotion.spatial()) { dir * it / 12 } + fadeIn(VyomaMotion.effects())) togetherWith
                            (slideOutHorizontally(VyomaMotion.spatial()) { -dir * it / 12 } + fadeOut(VyomaMotion.effects()))
                    },
                    label = "tab",
                ) { t -> holder.SaveableStateProvider(t.name) { content(t) } }
            }
        }

        Row(
            Modifier.align(Alignment.BottomCenter).padding(bottom = navBottom + 10.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TabBar(tab, onTab, backdrop)
            Box(
                Modifier.size(BarHeight).glassChrome(backdrop, CircleShape).pressable(pressedScale = 0.88f, onClick = onTouchpad),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.TouchApp, "Touchpad", tint = MaterialTheme.colorScheme.onBackground)
            }
        }
    }
}

/**
 * Glass pill with a liquid selection lens. The lens's leading edge leaves first and the trailing
 * edge catches up, so it stretches like a droplet in flight and thins as it does. Drag along the
 * bar to scrub between tabs (a tick per tab, like iOS); letting go picks the one under your thumb.
 * The lens is drawn, not laid out, so all of this runs in the draw phase.
 */
@Composable
private fun TabBar(tab: Tab, onTab: (Tab) -> Unit, backdrop: Backdrop) {
    val itemPx = with(LocalDensity.current) { ItemWidth.toPx() }
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val currentTab by rememberUpdatedState(tab)
    val select by rememberUpdatedState(onTab)
    val left = remember { Animatable(tab.ordinal * itemPx) }
    val right = remember { Animatable((tab.ordinal + 1) * itemPx) }
    val lift = remember { Animatable(0f) }
    var hover by remember { mutableIntStateOf(-1) }

    fun glide(toLeft: Float) {
        val forward = toLeft > left.targetValue
        scope.launch { left.animateTo(toLeft, if (forward) VyomaMotion.trailing() else VyomaMotion.leading()) }
        scope.launch { right.animateTo(toLeft + itemPx, if (forward) VyomaMotion.leading() else VyomaMotion.trailing()) }
    }
    LaunchedEffect(tab) { if (hover < 0) glide(tab.ordinal * itemPx) }

    val fill = Glass.raised
    val edge = Glass.edge
    Box(
        Modifier
            .height(BarHeight)
            .glassChrome(backdrop, CircleShape)
            .padding(4.dp)
            .drawBehind {
                val l = left.value
                val w = right.value - l
                // Stretched sideways, it gets a little thinner; lifted under a finger, a little bigger.
                val stretch = (w / itemPx - 1f).coerceIn(0f, 1f)
                val grow = 1f + 0.06f * lift.value
                val h = size.height * (1f - 0.14f * stretch) * grow
                val ww = w * grow
                val topLeft = Offset(l - (ww - w) / 2, (size.height - h) / 2)
                val r = CornerRadius(h / 2)
                drawRoundRect(fill, topLeft, Size(ww, h), r)
                drawRoundRect(edge, topLeft, Size(ww, h), r, style = Stroke(0.75.dp.toPx()))
            }
            .pointerInput(itemPx) {
                val last = Tab.entries.lastIndex
                fun indexAt(x: Float) = (x / itemPx).toInt().coerceIn(0, last)
                fun follow(x: Float) = glide((x - itemPx / 2).coerceIn(0f, last * itemPx))
                detectHorizontalDragGestures(
                    onDragStart = { o ->
                        hover = indexAt(o.x)
                        scope.launch { lift.animateTo(1f, VyomaMotion.spatial()) }
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
                        scope.launch { lift.animateTo(0f, VyomaMotion.release()) }
                        if (i != currentTab.ordinal) select(Tab.entries[i]) else glide(i * itemPx)
                    },
                    onDragCancel = {
                        hover = -1
                        scope.launch { lift.animateTo(0f, VyomaMotion.release()) }
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
