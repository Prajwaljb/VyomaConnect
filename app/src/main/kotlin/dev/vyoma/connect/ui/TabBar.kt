package dev.vyoma.connect.ui

import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.material.icons.rounded.Slideshow
import androidx.compose.material.icons.rounded.SportsEsports
import dev.vyoma.connect.designsystem.liquidPress
import dev.vyoma.connect.designsystem.MenuItem
import dev.vyoma.connect.designsystem.LocalMenu
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.ui.graphics.Color
import dev.vyoma.connect.designsystem.isDarkUi
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
import androidx.compose.material.icons.rounded.Folder
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
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import dev.vyoma.connect.designsystem.Backdrop
import dev.vyoma.connect.designsystem.Glass
import dev.vyoma.connect.designsystem.LensStyle
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
import dev.vyoma.connect.designsystem.menuOccluder
import dev.vyoma.connect.designsystem.liquidLens
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlin.math.abs
import androidx.compose.animation.core.tween
import androidx.compose.runtime.withFrameNanos
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.ui.draw.drawWithContent
import kotlin.coroutines.cancellation.CancellationException
import androidx.compose.runtime.mutableFloatStateOf
import kotlinx.coroutines.launch

enum class Tab(val label: String, val icon: ImageVector) {
    HOME("Home", Icons.Rounded.Home),
    FILES("Files", Icons.Rounded.Folder),
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
fun TabsRoot(
    tab: Tab, onTab: (Tab) -> Unit, backToHome: Boolean, onTouchpad: () -> Unit,
    onGamepad: () -> Unit = {},
    onPresenter: () -> Unit = {},
    content: @Composable (Tab) -> Unit,
) {
    val backdrop = rememberBackdrop()
    // Back on another tab goes Home, under the finger like a page: the tab slides off over Home
    // (parallaxed and dimmed, as under a pushed page), and finishes or snaps back from wherever you let go.
    val swipe = remember { Animatable(0f) }
    val swipeScope = rememberCoroutineScope()
    val visited = remember { mutableStateListOf(tab) }
    PredictiveBackHandler(enabled = backToHome && tab != Tab.HOME) { events ->
        // Swiping soon after this page came back from a pushed one: Home must be there to show.
        if (Tab.HOME !in visited) visited.add(Tab.HOME)
        try {
            events.collect { swipe.snapTo(it.progress) }
        } catch (e: CancellationException) {
            swipeScope.launch { swipe.animateTo(0f, VyomaMotion.page()) }
            throw e
        }
        swipeScope.launch {
            swipe.animateTo(1f, VyomaMotion.page())
            onTab(Tab.HOME)
            // The tabs switch without their crossfade (see below) while this still reads 1.
            withFrameNanos {}
            withFrameNanos {}
            swipe.snapTo(0f)
        }
    }
    val holder = rememberSaveableStateHolder()
    val navBottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
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
                        LaunchedEffect(active) {
                            if (swipe.value > 0f) shown.snapTo(if (active) 1f else 0f) // a back swipe already moved them
                            else shown.animateTo(if (active) 1f else 0f, if (active) VyomaMotion.spatial() else VyomaMotion.effects())
                        }
                        val peeking = t == Tab.HOME && !active
                        Box(
                            Modifier.fillMaxSize().layout { measurable, constraints ->
                                val page = measurable.measure(constraints)
                                layout(page.width, page.height) {
                                    val back = swipe.value
                                    // While a back swipe runs (and the frame it lands on), only it places the tabs:
                                    // the one leaving slides with the finger and Home waits under it.
                                    if (back > 0f) {
                                        if (t != Tab.HOME && !active) return@layout
                                        page.placeWithLayer(0, 0) {
                                            translationX = when {
                                                t != Tab.HOME -> back * size.width
                                                active -> 0f
                                                else -> -(1f - back) * size.width / 4
                                            }
                                        }
                                        return@layout
                                    }
                                    if (!active && shown.value < 0.01f) return@layout
                                    page.placeWithLayer(0, 0) {
                                        val p = shown.value
                                        alpha = p.coerceIn(0f, 1f)
                                        // Fade each draw instead of the whole page through an offscreen buffer:
                                        // every glass element replays this page, so a buffer would cost one per element.
                                        compositingStrategy = CompositingStrategy.ModulateAlpha
                                        translationX = (1f - p) * size.width / 12 * (if (active) travel.dir else -travel.dir)
                                    }
                                }
                            }.drawWithContent {
                                drawContent()
                                val back = swipe.value
                                if (peeking && back > 0f) drawRect(Color.Black, alpha = 0.4f * (1f - back))
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
            // Tapped: the touchpad (its keyboard is a button inside). Held: the game controller, the presenter.
            val menus = LocalMenu.current
            var bounds by remember { mutableStateOf(androidx.compose.ui.geometry.Rect.Zero) }
            Box(
                Modifier.size(BarHeight).menuOccluder().onGloballyPositioned { bounds = it.boundsInRoot() }
                    .glassChrome(backdrop, CircleShape, blur = 1.5.dp, bevel = 16.dp, interactive = true)
                    .liquidPress(
                        pressedScale = 1f, onLongClickLabel = "More",
                        onLongClick = {
                            menus.show(bounds, listOf(
                                MenuItem("Game controller", Icons.Rounded.SportsEsports, onClick = onGamepad),
                                MenuItem("Presenter", Icons.Rounded.Slideshow, onClick = onPresenter),
                            ), fromRadius = BarHeight / 2)
                        },
                        onClick = onTouchpad,
                    ),
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
 * it lifts into a clear drop of water a little bigger than the bar,
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
    // A held drop is elastic, like a droplet of water (2026-11: "the elasticity of a water
    // droplet", not ripples): it stretches along its motion as it speeds up, and when it stops the
    // stretch carries on past round and wobbles out on a soft spring. [squash] is that stretch
    // (+ longer, - taller), [squashV] its rate; stepped each frame only while the drop is up.
    var squash by remember { mutableFloatStateOf(0f) }
    var squashV by remember { mutableFloatStateOf(0f) }
    var elastic by remember { mutableStateOf<Job?>(null) }

    fun glide(toLeft: Float) {
        // Already headed there: restarting would swap which edge leads (and its spring) mid-flight,
        // which made a tap to the right overshoot while one to the left didn't.
        if (toLeft == left.targetValue && toLeft + itemPx == right.targetValue) return
        val forward = toLeft > left.targetValue
        scope.launch { left.animateTo(toLeft, if (forward) VyomaMotion.trailing() else VyomaMotion.leading()) }
        scope.launch { right.animateTo(toLeft + itemPx, if (forward) VyomaMotion.leading() else VyomaMotion.trailing()) }
    }
    fun lifted(up: Boolean) {
        pulse?.cancel()
        pulse = scope.launch { lift.animateTo(if (up) 1f else 0f, if (up) VyomaMotion.spatial() else VyomaMotion.release()) }
        if (up && elastic?.isActive != true) elastic = scope.launch {
            var last = withFrameNanos { it }
            do {
                withFrameNanos { now ->
                    val dt = ((now - last) / 1e9f).coerceIn(0f, 1f / 30)
                    last = now
                    // How fast the drop travels (bar widths a second) sets how stretched it wants to be.
                    val speed = abs(left.velocity + right.velocity) / 2f / itemPx
                    val target = (speed * 0.09f).coerceAtMost(0.32f)
                    // A soft, underdamped spring: it lags the speed, then overshoots into a wobble.
                    squashV += (SQUASH_K * (target - squash) - SQUASH_C * squashV) * dt
                    squash += squashV * dt
                }
            } while (hover >= 0 || lift.value > 0.01f || abs(squash) > 0.002f || abs(squashV) > 0.02f)
            squash = 0f
            squashV = 0f
        }
    }
    // The drop only lifts under a finger (as on iOS); a programmatic switch just glides.
    LaunchedEffect(tab) {
        if (hover < 0) glide(tab.ordinal * itemPx)
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
        // Pulled along, the droplet draws in: smaller overall the faster it goes (length too,
        // the user asked, 2026-11), still longer than tall; it springs back to full size when it
        // stops, wobbling through round. At rest it's unchanged.
        val sq = squash * up.coerceIn(0f, 1f)
        val shrink = 1f - 0.45f * sq.coerceAtLeast(0f)
        val h = (rest + (2 * overflow + 2 * pad) * up) * (1f - 0.14f * stretch) / (1f + sq) * shrink
        val ww = (w + 2 * overflow * up.coerceAtLeast(0f) * 1.6f + rest * 0.06f * up.coerceAtMost(0f)) * (1f + 0.4f * sq) * shrink
        val cx = pad + l + w / 2
        return Rect(Offset(cx - ww / 2, (size.height - h) / 2), Size(ww, h))
    }

    // iOS 26: the selected tab sits on a plain neutral capsule; its colour is in the icon and label.
    val fill = if (isDarkUi) Color.White.copy(alpha = 0.14f) else Color.Black.copy(alpha = 0.12f)
    Box(
        Modifier
            .height(BarHeight)
            .menuOccluder()
            .liquidLens(backdrop, strength = { lift.value }, zoom = { 1f + 0.1f * lift.value.coerceIn(0f, 1f) }, style = LensStyle.Lift, rect = { pill() })
            .glassChrome(backdrop, CircleShape, blur = 1.5.dp, bevel = 16.dp)
            .drawBehind {
                // The resting pill is gone early in the lift: magnified inside the growing drop, its
                // grey read as a white haze (where drops can be drawn).
                val a = if (liquidGlassSupported) 1f - (lift.value * 3f).coerceIn(0f, 1f) else 1f
                if (a < 0.01f) return@drawBehind
                val r = pill()
                val corner = CornerRadius(r.height / 2)
                drawRoundRect(fill, r.topLeft, r.size, corner, alpha = a)
            }
            .padding(4.dp)
            .pointerInput(itemPx) {
                // iOS 26: the drop lifts the moment a finger lands, jumps to the tab under it, follows
                // the finger along the bar (a tick per tab) and picks whatever it's over on release.
                val last = Tab.entries.lastIndex
                fun indexAt(x: Float) = (x / itemPx).toInt().coerceIn(0, last)
                fun follow(x: Float) = glide((x - itemPx / 2).coerceIn(0f, last * itemPx))
                awaitEachGesture {
                    val down = awaitFirstDown()
                    hover = indexAt(down.position.x)
                    lifted(true)
                    glide(hover * itemPx)
                    if (hover != currentTab.ordinal) haptics.perform(Haptic.Select)
                    var dragged = false
                    var cancelled = false
                    // A swipe texture: a fine, light grain under the thumb as the drop slides,
                    // denser the faster it goes (distance-based), between the tab ticks.
                    val grainEvery = 6.dp.toPx()
                    var slid = 0f
                    var lastX = down.position.x
                    var lastGrain = 0L
                    while (true) {
                        val e = awaitPointerEvent()
                        val c = e.changes.firstOrNull { it.id == down.id }
                        if (c == null) { cancelled = true; break }
                        if (!c.pressed) break
                        if (c.isConsumed) { cancelled = true; break }
                        if (abs(c.position.x - down.position.x) > viewConfiguration.touchSlop) dragged = true
                        if (dragged) {
                            c.consume()
                            follow(c.position.x)
                            slid += abs(c.position.x - lastX)
                            lastX = c.position.x
                            val now = c.uptimeMillis
                            val i = indexAt(c.position.x)
                            if (i != hover) {
                                hover = i
                                haptics.perform(Haptic.Select)
                                slid = 0f
                                lastGrain = now
                            } else if (slid >= grainEvery && now - lastGrain >= 18) {
                                haptics.perform(Haptic.Grain)
                                slid = 0f
                                lastGrain = now
                            }
                        }
                    }
                    val i = hover
                    hover = -1
                    lifted(false)
                    when {
                        cancelled -> glide(currentTab.ordinal * itemPx)
                        i != currentTab.ordinal -> select(Tab.entries[i])
                        else -> glide(i * itemPx)
                    }
                }
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
private fun TabItem(tab: Tab, selected: Boolean, onSelect: () -> Unit) {
    val color by animateColorAsState(
        if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onBackground,
        VyomaMotion.effects(), label = "tabColor",
    )
    Column(
        Modifier.width(ItemWidth).fillMaxHeight().semantics(mergeDescendants = true) {
            role = Role.Tab
            this.selected = selected
            onClick { onSelect(); true }
        },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(tab.icon, null, tint = color, modifier = Modifier.size(24.dp))
        Spacer(Modifier.height(2.dp))
        Text(tab.label, style = MaterialTheme.typography.labelSmall, color = color)
    }
}

/** The held droplet's elastic spring (see TabBar): stiffness, and damping well under critical so it wobbles. */
private const val SQUASH_K = 260f
private const val SQUASH_C = 9f
