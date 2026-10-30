package dev.vyoma.connect.designsystem

import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.window.Dialog
import androidx.compose.material3.ProvideTextStyle
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.horizontalDrag
import androidx.compose.foundation.gestures.awaitHorizontalTouchSlopOrCancellation
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.util.lerp
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.remember
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import kotlin.math.abs
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.IntSize
import kotlin.math.roundToInt
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.runtime.snapshotFlow
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.runtime.mutableStateOf
import kotlinx.coroutines.launch
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * One UI's signature layout: a tall, airy title area that keeps content in thumb reach,
 * collapsing into a compact bar as you scroll. Collapse is driven by graphicsLayer reads
 * of scroll state, so scrolling never recomposes the header.
 */
@Composable
fun CollapsingHeaderScaffold(
    title: String,
    subtitle: String? = null,
    modifier: Modifier = Modifier,
    navigation: (@Composable () -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit = {},
    hero: (@Composable () -> Unit)? = null,
    listState: LazyListState = rememberLazyListState(),
    /** Applied to the list itself, e.g. [glassListGroup] to draw behind its items. */
    listModifier: Modifier = Modifier,
    content: LazyListScope.() -> Unit,
) {
    val screenHeight = LocalConfiguration.current.screenHeightDp.dp
    val headerHeight = (screenHeight * 0.30f).coerceIn(180.dp, 300.dp)
    val density = LocalDensity.current
    val headerPx = with(density) { headerHeight.toPx() }
    val distance = headerPx * 0.6f
    val collapse by remember {
        derivedStateOf {
            if (listState.firstVisibleItemIndex > 0) 1f
            else (listState.firstVisibleItemScrollOffset / distance).coerceIn(0f, 1f)
        }
    }
    // One UI never rests half-collapsed: when a scroll ends inside the header, settle open or
    // closed, whichever is nearer.
    LaunchedEffect(listState, headerPx) {
        val d = distance.toInt()
        snapshotFlow { listState.isScrollInProgress }.collect { scrolling ->
            if (scrolling || listState.firstVisibleItemIndex != 0) return@collect
            val offset = listState.firstVisibleItemScrollOffset
            if (offset in 1 until d) {
                // Its own job: a finger landing mid-snap cancels just the snap. Run inline, that
                // cancellation ended this whole watcher and nothing snapped again.
                launch { listState.animateScrollBy((if (offset < d / 2) -offset else d - offset).toFloat(), VyomaMotion.page()) }
            }
        }
    }
    // A page that overflows by less than the collapse would stop with the title half faded (and
    // the snap above couldn't close it): room at the end lets any page that scrolls at all close
    // fully. Measured from the real items only, so the room never feeds back into itself.
    var room by remember { mutableIntStateOf(0) }
    LaunchedEffect(listState, distance) {
        snapshotFlow {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull { it.key != ROOM_KEY } ?: return@snapshotFlow null
            if (listState.firstVisibleItemIndex != 0 || last.index != info.totalItemsCount - 2) return@snapshotFlow null
            val end = last.offset + last.size + listState.firstVisibleItemScrollOffset + info.afterContentPadding
            val overflow = end - info.viewportEndOffset
            if (overflow <= 0) 0 else (distance.roundToInt() - overflow).coerceAtLeast(0)
        }.collect { if (it != null) room = it }
    }

    // The title is one piece of text that travels: it rests in the hero, and as the page scrolls it
    // rises and shrinks into the bar, where a bubble of glass forms around it like water.
    val heroStyle = if (hero != null) MaterialTheme.typography.headlineLarge else MaterialTheme.typography.displayMedium
    val shrink = MaterialTheme.typography.titleMedium.fontSize.value / heroStyle.fontSize.value
    val statusTop = WindowInsets.statusBars.getTop(density)
    val anchor = remember { CoordinatesHolder() }
    // The hero title's centre at rest, in the scaffold's coordinates (NaN until placed).
    var restY by remember { mutableFloatStateOf(Float.NaN) }
    var titleSize by remember { mutableStateOf(IntSize.Zero) }
    val bubbleW = remember { Animatable(0f) }
    val bubbleH = remember { Animatable(0f) }
    // Leaving, the bubble pops: it keeps its size around the title and swells a little as it
    // fades (shrinking into a dot behind full-size text looked wrong).
    val bubblePop = remember { Animatable(0f) }
    LaunchedEffect(Unit) {
        snapshotFlow { collapse > 0.72f }.collect { on ->
            if (on) {
                // Forming, the drop swells to full height first and then spreads sideways with a
                // wobble; after a pop it starts again from a drop.
                if (bubblePop.value > 0f) {
                    bubblePop.snapTo(0f)
                    bubbleW.snapTo(0f)
                    bubbleH.snapTo(0f)
                }
                launch { bubbleH.animateTo(1f, VyomaMotion.leading()) }
                launch { bubbleW.animateTo(1f, VyomaMotion.jelly()) }
            } else if (bubbleH.value > 0f) {
                launch { bubblePop.animateTo(1f, tween(220, easing = FastOutSlowInEasing)) }
            }
        }
    }

    val backdrop = rememberBackdrop()
    val bottomInset = LocalBottomBarInset.current
    // Content color for everything on the canvas, so bar icons and titles are legible.
    CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onBackground) { Box(modifier.fillMaxSize().onPlaced { anchor.coordinates = it }) {
        // The backdrop records the canvas too: blurring only the cards would let sharp text show through.
        Box(Modifier.fillMaxSize().backdropSource(backdrop).aurora()) { LazyColumn(
            state = listState,
            contentPadding = PaddingValues(
                start = 16.dp, end = 16.dp,
                bottom = 24.dp + bottomInset + WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding(),
            ),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.fillMaxSize().then(listModifier),
        ) {
            item(key = "header", contentType = "header") {
                Column(
                    Modifier
                        .fillMaxWidth()
                        .height(headerHeight)
                        .graphicsLayer {
                            // Gone by three quarters, before the subtitle can pass under the title's bubble.
                            val c = (listState.firstVisibleItemScrollOffset / (distance * 0.75f)).coerceIn(0f, 1f)
                            alpha = 1f - c
                            compositingStrategy = CompositingStrategy.ModulateAlpha
                        }
                        .padding(horizontal = 12.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Spacer(Modifier.height(32.dp))
                    if (hero != null) {
                        hero()
                        Spacer(Modifier.height(14.dp))
                    }
                    // Holds the title's place; the travelling copy above the list is what shows.
                    Text(
                        title,
                        style = heroStyle,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier
                            .graphicsLayer { alpha = 0f }
                            .onGloballyPositioned { c ->
                                val root = anchor.coordinates ?: return@onGloballyPositioned
                                if (listState.firstVisibleItemIndex != 0 || !root.isAttached) return@onGloballyPositioned
                                // Back to where it sits unscrolled.
                                val y = root.localPositionOf(c, Offset.Zero).y + c.size.height / 2f +
                                    listState.firstVisibleItemScrollOffset
                                if (restY.isNaN() || abs(y - restY) > 0.5f) restY = y
                            },
                    )
                    if (subtitle != null) {
                        Spacer(Modifier.height(6.dp))
                        Text(subtitle, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            content()
            item(key = ROOM_KEY, contentType = ROOM_KEY) {
                Spacer(Modifier.height(with(density) { room.toDp() }))
            }
        } }

        // The compact bar floats over the list in the same material as the tab bar: back and
        // actions are liquid-glass buttons, and the title's bubble forms in the middle. Instead of a
        // hard frosted strip, the status bar area fades softly to the canvas so content slips under it.
        val edgeColor = MaterialTheme.colorScheme.background
        Box(Modifier.fillMaxWidth()) {
            Box(
                Modifier.matchParentSize().drawBehind {
                    val a = collapse
                    if (a < 0.01f) return@drawBehind
                    drawRect(
                        Brush.verticalGradient(
                            0f to edgeColor.copy(alpha = a),
                            0.45f to edgeColor.copy(alpha = 0.85f * a),
                            1f to Color.Transparent,
                        ),
                    )
                },
            )
            Box(
                Modifier
                    .fillMaxWidth()
                    .windowInsetsPadding(WindowInsets.statusBars)
                    .height(BAR_HEIGHT)
                    .padding(horizontal = 12.dp),
            ) {
                if (navigation != null) {
                    Box(
                        Modifier.align(Alignment.CenterStart).size(48.dp).glassChrome(backdrop, CircleShape, blur = 8.dp, bevel = 16.dp, interactive = true),
                        contentAlignment = Alignment.Center,
                    ) { CompositionLocalProvider(LocalOnChrome provides true) { navigation() } }
                }
                // The title's bubble: sized here (layout only, while it forms), empty; the title
                // itself is drawn over it.
                Box(
                    Modifier
                        .align(Alignment.Center)
                        .layout { m, c ->
                            val full = (titleSize.width * shrink + 36.dp.toPx()).coerceAtMost(c.maxWidth - 120.dp.toPx())
                            val h = (44.dp.toPx() * bubbleH.value).coerceAtLeast(0f)
                            val w = maxOf(h, full * bubbleW.value).coerceAtLeast(0f)
                            val p = m.measure(Constraints.fixed(w.roundToInt(), h.roundToInt()))
                            layout(p.width, p.height) { p.place(0, 0) }
                        }
                        .graphicsLayer {
                            val pop = bubblePop.value
                            alpha = 1f - pop
                            scaleX = 1f + 0.18f * pop
                            scaleY = 1f + 0.12f * pop
                        }
                        .drawWithContent { if (bubblePop.value < 1f) drawContent() }
                        .glassChrome(backdrop, CircleShape, blur = 8.dp, bevel = 16.dp),
                )
                Row(
                    Modifier.align(Alignment.CenterEnd).height(48.dp).glassChrome(backdrop, CircleShape, blur = 8.dp, bevel = 16.dp, interactive = true),
                    verticalAlignment = Alignment.CenterVertically,
                ) { CompositionLocalProvider(LocalOnChrome provides true) { actions() } }
            }
        }

        // The travelling title: centred like both its homes, so only height and size change.
        Text(
            title,
            style = heroStyle,
            color = MaterialTheme.colorScheme.onBackground,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier
                .align(Alignment.TopCenter)
                .padding(horizontal = 24.dp)
                .onSizeChanged { titleSize = it }
                .clearAndSetSemantics {}
                .graphicsLayer {
                    val rest = restY
                    if (rest.isNaN()) {
                        alpha = 0f
                        return@graphicsLayer
                    }
                    // It rides with the page until it reaches the bar (by 90% of the collapse at
                    // the latest), then stays there; it shrinks over the same trip.
                    val inBar = statusTop + BAR_HEIGHT.toPx() / 2f
                    val trip = (rest - inBar).coerceAtLeast(1f)
                    val travelled = (collapse * maxOf(distance, trip / 0.9f)).coerceAtMost(trip)
                    translationY = rest - travelled - size.height / 2f
                    val q = travelled / trip
                    val k = lerp(1f, shrink, q * q * (3f - 2f * q))
                    scaleX = k
                    scaleY = k
                },
        )
    } }
}

private const val ROOM_KEY = "collapse-room"
private val BAR_HEIGHT = 64.dp

/** Plain holder: layout coordinates read in callbacks, never state. */
private class CoordinatesHolder {
    var coordinates: LayoutCoordinates? = null
}

/** Grouped rounded card: the basic One UI container. */
@Composable
fun SectionCard(
    modifier: Modifier = Modifier,
    contentPadding: PaddingValues = PaddingValues(vertical = 6.dp),
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.large).padding(contentPadding), content = content)
}

/**
 * Group heading above a [SectionCard]. Drawn a few dp low so it sits with the card it names
 * rather than midway between groups, without changing the list's even spacing.
 */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.padding(start = 20.dp, top = 12.dp).offset(y = 5.dp),
    )
}

@Composable
fun ListRow(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    icon: ImageVector? = null,
    iconTint: Color = MaterialTheme.colorScheme.primary,
    showDivider: Boolean = false,
    trailing: (@Composable () -> Unit)? = null,
    onClick: (() -> Unit)? = null,
) {
    Column(modifier) {
        Row(
            Modifier
                .fillMaxWidth()
                .then(if (onClick != null) Modifier.pressable(pressedScale = 0.985f, onClick = onClick) else Modifier)
                .padding(horizontal = 20.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (icon != null) {
                IconBadge(icon, iconTint)
                Spacer(Modifier.width(16.dp))
            }
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface)
                if (subtitle != null) {
                    Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            if (trailing != null) {
                Spacer(Modifier.width(12.dp))
                trailing()
            }
        }
        if (showDivider) {
            HorizontalDivider(
                Modifier.padding(start = if (icon != null) 70.dp else 20.dp, end = 20.dp),
                color = MaterialTheme.colorScheme.outlineVariant,
            )
        }
    }
}

/** iOS / One UI settings icon: a white glyph on a filled, softly rounded square of [tint], its edge catching the light. */
@Composable
fun IconBadge(icon: ImageVector, tint: Color, size: Dp = 34.dp) {
    Box(
        Modifier
            .size(size)
            .specularEdge(RoundedCornerShape(size * 0.28f), 0.8.dp)
            .clip(RoundedCornerShape(size * 0.28f))
            .background(Brush.verticalGradient(listOf(tint.copy(alpha = 0.88f).compositeOver(Color.White), tint))),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = Color.White, modifier = Modifier.size(size * 0.6f))
    }
}

enum class OrbState { CONNECTED, PENDING, OFFLINE }

/**
 * Live connection indicator: a soft glow that breathes while connected, pulses fast
 * while pairing, and rests gray offline.
 */
@Composable
fun ConnectionOrb(state: OrbState, modifier: Modifier = Modifier, size: Dp = 14.dp) {
    val dark = MaterialTheme.colorScheme.background.red < 0.5f
    val color by animateColorAsState(
        when (state) {
            OrbState.CONNECTED -> Status.connected(dark)
            OrbState.PENDING -> Status.pending(dark)
            OrbState.OFFLINE -> Status.offline(dark)
        },
        label = "orb",
    )
    // Breathes a few times when it connects, then rests: an endless pulse kept the whole screen
    // (glass blur included) redrawing at 120 Hz while idle. Pairing keeps pulsing; it's brief.
    val pulse = remember { Animatable(0f) }
    LaunchedEffect(state) {
        val beats = when (state) {
            OrbState.CONNECTED -> 3
            OrbState.PENDING -> Int.MAX_VALUE
            OrbState.OFFLINE -> 0
        }
        repeat(beats) {
            pulse.snapTo(0f)
            pulse.animateTo(1f, tween(if (state == OrbState.PENDING) 700 else 2200, easing = LinearEasing))
        }
        pulse.snapTo(1f)
    }
    Canvas(modifier.size(size * 2.4f)) {
        val r = size.toPx() / 2
        val c = Offset(this.size.width / 2, this.size.height / 2)
        val p = pulse.value
        if (p < 1f) {
            drawCircle(color.copy(alpha = 0.35f * (1f - p)), radius = r * (1f + p * 1.4f), center = c)
        }
        drawCircle(
            Brush.radialGradient(listOf(color.copy(alpha = 0.95f), color), center = c - Offset(r * 0.3f, r * 0.3f), radius = r * 1.4f),
            radius = r,
            center = c,
        )
    }
}

@Composable
fun Pill(text: String, modifier: Modifier = Modifier, icon: ImageVector? = null, tint: Color = MaterialTheme.colorScheme.primary) {
    Row(
        modifier
            .clip(CircleShape)
            .background(tint.copy(alpha = 0.12f))
            .padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, tint = tint, modifier = Modifier.size(14.dp))
            Spacer(Modifier.width(5.dp))
        }
        Text(text, style = MaterialTheme.typography.labelMedium, color = tint)
    }
}

/**
 * Switch: an accent track with a round white thumb. Tap it, or drag the thumb across (it follows
 * the finger and lands on whichever side it's let go nearer); it bounces into place and squashes
 * against the end. Touched, the thumb lifts into a clear drop of liquid glass, bigger than the
 * track, that magnifies the track under it, and frosts back into the white thumb when let go
 * (the same knob as the sliders). A vertical swipe that starts on it still scrolls the page.
 */
@Composable
fun VyomaSwitch(checked: Boolean, onCheckedChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val position = remember { Animatable(if (checked) 1f else 0f) }
    var held by remember { mutableStateOf(false) }
    val current by rememberUpdatedState(checked)
    val change by rememberUpdatedState(onCheckedChange)
    LaunchedEffect(checked) { if (!held) position.animateTo(if (checked) 1f else 0f, VyomaMotion.release()) }
    val swell = animateFloatAsState(if (held) 1f else 0f, if (held) VyomaMotion.spatial() else VyomaMotion.release(), label = "swell")
    val track by animateColorAsState(
        if (checked) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.16f),
        VyomaMotion.effects(), label = "track",
    )
    val card = MaterialTheme.colorScheme.surfaceContainerLow
    val padDp = 2.5.dp
    fun DrawScope.thumb(): Rect {
        val pad = padDp.toPx()
        val d = size.height - 2 * pad
        val k = swell.value.coerceAtLeast(0f)
        val raw = position.value
        val p = raw.coerceIn(0f, 1f)
        // Overshoot past either end squashes the thumb against the wall instead of leaving the track.
        val squash = abs(raw - p) * d * 1.5f
        val h = d * (1f + 0.4f * k)
        val w = d * (1f + 0.75f * k) - squash
        val cx = pad + d / 2 + (size.width - 2 * pad - d) * p
        return Rect(cx - w / 2, size.height / 2 - h / 2, cx + w / 2, size.height / 2 + h / 2)
    }
    fun settle(on: Boolean) {
        held = false
        if (on != current) {
            haptics.perform(if (on) Haptic.ToggleOn else Haptic.ToggleOff)
            change(on)
        }
        scope.launch { position.animateTo(if (on) 1f else 0f, VyomaMotion.release()) }
    }
    Spacer(
        modifier
            .size(52.dp, 32.dp)
            .semantics {
                role = Role.Switch
                toggleableState = ToggleableState(checked)
                onClick { settle(!checked); true }
            }
            .pointerInput(Unit) {
                val travel = size.width - size.height.toFloat()
                awaitEachGesture {
                    val down = awaitFirstDown()
                    held = true
                    var v = position.value
                    val drag = awaitHorizontalTouchSlopOrCancellation(down.id) { c, over ->
                        c.consume()
                        v = (v + over / travel).coerceIn(0f, 1f)
                        scope.launch { position.snapTo(v) }
                    }
                    if (drag != null) {
                        val done = horizontalDrag(drag.id) { c ->
                            // Read before consuming: a consumed change reports no movement.
                            val dx = c.positionChange().x
                            c.consume()
                            v = (v + dx / travel).coerceIn(0f, 1f)
                            scope.launch { position.snapTo(v) }
                        }
                        settle(if (done) v > 0.5f else current)
                    } else {
                        // Lifted in place: a tap. Taken by the page (a scroll): nothing changes.
                        val c = currentEvent.changes.firstOrNull { it.id == down.id }
                        settle(if (c != null && !c.pressed && !c.isConsumed) !current else current)
                    }
                }
            }
            .drawWithContent {
                drawContent()
                frostedKnob(thumb(), 1f - swell.value)
            }
            .liquidLens(null, strength = { swell.value }, zoom = { 1f + 0.1f * swell.value.coerceIn(0f, 1f) }, underlay = card, style = LensStyle.Lift, frost = { 1f - swell.value }, rect = { thumb() })
            .drawBehind { drawRoundRect(track, cornerRadius = CornerRadius(size.height / 2)) },
    )
}

/**
 * The one button shape: a pill that squishes and ticks. [prominent] fills it with the accent for
 * the main action; otherwise it's tinted glass. [large] is the full-width onboarding size.
 */
@Composable
fun PillButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    prominent: Boolean = true,
    enabled: Boolean = true,
    large: Boolean = false,
    haptic: Haptic? = Haptic.Tap,
) {
    val accent = MaterialTheme.colorScheme.primary
    val bg by animateColorAsState(
        when {
            !enabled -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f)
            prominent -> accent
            else -> accent.copy(alpha = 0.14f)
        },
        VyomaMotion.effects(), label = "pillBg",
    )
    val fg = when {
        !enabled -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
        prominent -> MaterialTheme.colorScheme.onPrimary
        else -> accent
    }
    Box(
        modifier
            .then(if (large) Modifier.fillMaxWidth().height(54.dp) else Modifier.height(40.dp))
            .pressable(enabled = enabled, pressedScale = if (large) 0.97f else 0.92f, haptic = haptic, glass = true, onClick = onClick)
            // Prominent pills are lit accent glass; the rest are raised glass with an accent label.
            .glassRaised(CircleShape, bg.takeIf { prominent && enabled })
            .padding(horizontal = 20.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = if (large) MaterialTheme.typography.titleSmall else MaterialTheme.typography.labelLarge, color = fg, maxLines = 1)
    }
}

/**
 * iOS 26 segmented control: a capsule track with a raised thumb that slides between options as a
 * gel (leading edge first, trailing edge catching up). Touched, the thumb lifts into a clear drop of
 * liquid glass a little bigger than the track that magnifies the label under it, and you can slide
 * it to another option; let go, it settles as the thumb again. Drawn, not laid out.
 */
@Composable
fun SegmentedControl(options: List<String>, selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier) {
    var width by remember { mutableStateOf(0) }
    val seg = if (options.isEmpty()) 0f else width.toFloat() / options.size
    val left = remember { Animatable(-1f) }
    val right = remember { Animatable(-1f) }
    val lift = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val current by rememberUpdatedState(selected)
    val pick by rememberUpdatedState(onSelect)
    fun glide(i: Int) {
        if (seg == 0f) return
        val l = i * seg
        if (left.value < 0f) {
            scope.launch { left.snapTo(l); right.snapTo(l + seg) }
            return
        }
        val forward = l > left.targetValue
        scope.launch { left.animateTo(l, if (forward) VyomaMotion.trailing() else VyomaMotion.leading()) }
        scope.launch { right.animateTo(l + seg, if (forward) VyomaMotion.leading() else VyomaMotion.trailing()) }
    }
    LaunchedEffect(selected, seg) { glide(selected) }
    val dark = isDarkUi
    val thumb = if (dark) Color(0xFF636366) else Color.White
    val shadow = Color.Black.copy(alpha = if (dark) 0f else 0.1f)
    val card = MaterialTheme.colorScheme.surfaceContainerLow
    val pad = with(LocalDensity.current) { 3.dp.toPx() }
    // The thumb in the control's coordinates, swelling past the track while lifted.
    fun DrawScope.thumbRect(): Rect {
        val k = lift.value.coerceAtLeast(0f)
        val grow = 7.dp.toPx() * k
        return Rect(pad + left.value - grow, pad - grow, pad + right.value + grow, size.height - pad + grow)
    }
    Box(
        modifier
            .fillMaxWidth()
            .height(40.dp)
            .liquidLens(null, strength = { lift.value }, zoom = { 1f + 0.12f * lift.value.coerceIn(0f, 1f) }, underlay = card, style = LensStyle.Lift, rect = { thumbRect() })
            .background(MaterialTheme.colorScheme.onSurface.copy(alpha = if (dark) 0.12f else 0.07f), CircleShape)
            .drawBehind {
                if (left.value < 0f) return@drawBehind
                val a = 1f - lift.value.coerceIn(0f, 1f)
                if (a < 0.01f) return@drawBehind
                val r = thumbRect()
                val corner = CornerRadius(r.height / 2)
                drawRoundRect(shadow, r.topLeft + Offset(0f, 1.dp.toPx()), r.size, corner, alpha = a)
                drawRoundRect(thumb, r.topLeft, r.size, corner, alpha = a)
            }
            .pointerInput(options.size) {
                fun indexAt(x: Float) = ((x - pad) / (size.width - 2 * pad) * options.size).toInt().coerceIn(0, options.lastIndex)
                awaitEachGesture {
                    val down = awaitFirstDown()
                    var hover = indexAt(down.position.x)
                    scope.launch { lift.animateTo(1f, VyomaMotion.spatial()) }
                    if (hover != current) { haptics.perform(Haptic.Select); glide(hover) }
                    fun follow(x: Float) {
                        val i = indexAt(x)
                        if (i != hover) { hover = i; haptics.perform(Haptic.Select); glide(i) }
                    }
                    // Sideways claims the gesture (the thumb follows); a vertical swipe belongs to the page.
                    val drag = awaitHorizontalTouchSlopOrCancellation(down.id) { c, _ ->
                        c.consume()
                        follow(c.position.x)
                    }
                    val cancelled = if (drag != null) {
                        !horizontalDrag(drag.id) { c ->
                            c.consume()
                            follow(c.position.x)
                        }
                    } else {
                        val c = currentEvent.changes.firstOrNull { it.id == down.id }
                        c == null || c.pressed || c.isConsumed
                    }
                    scope.launch { lift.animateTo(0f, VyomaMotion.release()) }
                    if (cancelled) glide(current) else if (hover != current) pick(hover) else glide(current)
                }
            }
            .padding(3.dp)
            .onSizeChanged { width = it.width },
    ) {
        Row(Modifier.fillMaxSize()) {
            options.forEachIndexed { i, label ->
                val on = i == selected
                Box(
                    Modifier
                        .weight(1f)
                        .fillMaxSize()
                        .semantics { role = Role.Tab; this.selected = on; onClick { if (!on) onSelect(i); true } },
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        label,
                        style = MaterialTheme.typography.labelLarge,
                        color = if (on) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                    )
                }
            }
        }
    }
}

/**
 * Apple's two sliders in one. With a [knob] it's the iOS 26 slider: a slim track and a wide white
 * capsule knob that, touched, lifts into a clear drop of liquid glass magnifying the track and
 * frosts back when let go. Without one it's the Apple Music scrubber: a bare track that thickens
 * under the finger. Either way it follows the finger's movement relatively once it moves
 * sideways (a vertical swipe still scrolls the page; a touch never makes it jump; a tap changes
 * nothing). [value] is read in the draw phase only.
 */
@Composable
fun LiquidSlider(
    value: () -> Float,
    onChange: (Float) -> Unit,
    onDone: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    fill: Color = Color.White,
    track: Color = Color.White.copy(alpha = 0.22f),
    knob: Dp? = 24.dp,
    rest: Dp = 6.dp,
    swollen: Dp = if (knob == null) 14.dp else 7.dp,
    /** What the slider sits on, so the lifted knob has something to refract around the track. */
    underlay: Color = Color.Transparent,
    edgeHaptics: Boolean = true,
) {
    val swell = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val change by rememberUpdatedState(onChange)
    val done by rememberUpdatedState(onDone)
    Box(
        modifier
            .fillMaxWidth()
            .height(if (knob == null) 32.dp else maxOf(32.dp, knob + 8.dp))
            .then(
                if (!enabled) Modifier else Modifier.pointerInput(Unit) {
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        scope.launch { swell.animateTo(1f, VyomaMotion.spatial()) }
                        var v = value()
                        var moved = false
                        val start = awaitHorizontalTouchSlopOrCancellation(down.id) { c, over ->
                            c.consume()
                            moved = true
                            v = (v + over / size.width).coerceIn(0f, 1f)
                            change(v)
                        }
                        if (start != null) {
                            horizontalDrag(start.id) { c ->
                                val dx = c.positionChange().x
                                c.consume()
                                val n = (v + dx / size.width).coerceIn(0f, 1f)
                                if (edgeHaptics && (n == 0f || n == 1f) && n != v) haptics.perform(Haptic.Edge)
                                v = n
                                change(v)
                            }
                        }
                        if (moved) done()
                        scope.launch { swell.animateTo(0f, VyomaMotion.release()) }
                    }
                },
            )
            .then(
                if (knob == null) Modifier else Modifier
                    // At rest a drawn white knob (no layers); held, a lens of the same rect takes over.
                    .drawWithContent {
                        drawContent()
                        frostedKnob(sliderKnob(value(), swell.value, knob), 1f - swell.value)
                    }
                    .liquidLens(
                        null,
                        strength = { swell.value },
                        zoom = { 1f + 0.2f * swell.value.coerceIn(0f, 1f) },
                        underlay = underlay,
                        style = LensStyle.Lift,
                        frost = { 1f - swell.value },
                        rect = { sliderKnob(value(), swell.value, knob) },
                    ),
            )
            .drawBehind {
                val k = swell.value.coerceAtLeast(0f)
                val h = lerp(rest.toPx(), swollen.toPx(), k)
                val top = (size.height - h) / 2
                val r = CornerRadius(h / 2)
                drawRoundRect(track, Offset(0f, top), Size(size.width, h), r)
                val w = (value().coerceIn(0f, 1f) * size.width)
                if (w <= 0f) return@drawBehind
                drawRoundRect(fill, Offset(0f, top), Size(w.coerceAtLeast(h), h), r)
            },
    )
}

/** The knob: a wide capsule centred on the value, kept inside the track; lifted it grows ~45%. */
private fun DrawScope.sliderKnob(value: Float, swell: Float, knob: Dp): Rect {
    val k = swell.coerceAtLeast(0f)
    val h = knob.toPx() * (1f + 0.45f * k)
    val w = h * 1.55f
    val rest = knob.toPx() * 1.55f
    val x = (value.coerceIn(0f, 1f) * size.width).coerceIn(rest / 2, size.width - rest / 2)
    return Rect(x - w / 2, (size.height - h) / 2, x + w / 2, (size.height + h) / 2)
}

/**
 * A slider knob at rest: the frosted look of a [liquidLens] with full frost, drawn as a plain
 * pill, so a resting slider costs no offscreen layers. Faded by [alpha] as the lens takes over.
 */
fun DrawScope.frostedKnob(r: Rect, alpha: Float) {
    val a = alpha.coerceIn(0f, 1f)
    if (a < 0.01f || r.width < 1f) return
    val corner = CornerRadius(r.height / 2)
    drawRoundRect(Color.Black.copy(alpha = 0.14f * a), r.topLeft + Offset(0f, 1.dp.toPx()), r.size, corner)
    drawRoundRect(Color.White, r.topLeft, r.size, corner, alpha = a)
    drawRoundRect(
        Brush.verticalGradient(listOf(Color.White, Color.Black.copy(alpha = 0.06f)), startY = r.top, endY = r.bottom),
        r.topLeft, r.size, corner, alpha = a, style = Stroke(1.dp.toPx()),
    )
}

/**
 * A dialog in the app's material: an opaque glass slab (dialogs have no backdrop to blur) with
 * the same lit rim and depth as the cards. Same shape of API as Material's AlertDialog.
 */
@Composable
fun GlassDialog(
    onDismissRequest: () -> Unit,
    confirmButton: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    dismissButton: (@Composable () -> Unit)? = null,
    title: (@Composable () -> Unit)? = null,
    text: (@Composable () -> Unit)? = null,
) {
    Dialog(onDismissRequest) {
        Column(
            modifier
                .fillMaxWidth()
                .glassCard(MaterialTheme.shapes.extraLarge, fill = MaterialTheme.colorScheme.surfaceContainerHigh)
                .padding(start = 24.dp, end = 24.dp, top = 24.dp, bottom = 12.dp),
        ) {
            CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurface) {
                if (title != null) {
                    ProvideTextStyle(MaterialTheme.typography.headlineSmall) { title() }
                    Spacer(Modifier.height(14.dp))
                }
                if (text != null) {
                    CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) {
                        ProvideTextStyle(MaterialTheme.typography.bodyMedium) { text() }
                    }
                    Spacer(Modifier.height(16.dp))
                }
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
                    dismissButton?.invoke()
                    Spacer(Modifier.width(4.dp))
                    confirmButton()
                }
            }
        }
    }
}

/** True inside an interactive [glassChrome] button: content shouldn't add its own glass. */
val LocalOnChrome = staticCompositionLocalOf { false }

/**
 * An icon button that squishes and bounces like everything else (Material's IconButton only
 * ripples). 48 dp target.
 */
@Composable
fun BounceIconButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    size: Dp = 48.dp,
    content: @Composable () -> Unit,
) {
    Box(
        // On liquid chrome the chrome itself flexes, so only the glyph squishes here.
        modifier.size(size).pressable(enabled = enabled, pressedScale = 0.8f, glass = !LocalOnChrome.current, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { content() }
}
