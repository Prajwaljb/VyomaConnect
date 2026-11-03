package dev.vyoma.connect.designsystem

import androidx.compose.ui.layout.boundsInRoot

import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.runtime.withFrameNanos
import kotlinx.coroutines.Job
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.Icons
import androidx.compose.ui.text.TextStyle
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
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.layoutId
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
import androidx.compose.ui.text.font.FontWeight
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
    /** The hero's share of the screen: ~30% keeps content in thumb reach. The same on every page (One UI). */
    heroFraction: Float = 0.30f,
    content: LazyListScope.() -> Unit,
) {
    val screenHeight = LocalConfiguration.current.screenHeightDp.dp
    // Wide windows (landscape, tablets, foldables) keep a phone-width column in the middle: stretched
    // edge to edge, square cards grew taller than the screen.
    val side = ((LocalConfiguration.current.screenWidthDp.dp - MaxContentWidth) / 2).coerceAtLeast(16.dp)
    val headerHeight = (screenHeight * heroFraction).coerceIn(180.dp, 300.dp)
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
    // A page whose content all shows above the tab bar doesn't scroll at all: scrolling only to
    // collapse the title read as broken ("everything fits, why does it scroll?").
    var fits by remember { mutableStateOf(false) }
    // It must clear the tab bar (or the nav bar) by a real gap: ending flush against it read as cut
    // off, a page that wouldn't scroll to show its last row.
    val barClearance = with(density) { (24.dp - 12.dp).roundToPx() }
    LaunchedEffect(listState, distance) {
        snapshotFlow {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull { it.key != ROOM_KEY }
            // Scrolled, or the last real item is off screen: the page overflows, so it scrolls. (This
            // used to leave the last answer standing: a page that first loaded short, like a folder
            // still loading or Home before the PC appeared, stayed unscrollable once it grew.)
            if (last == null || listState.firstVisibleItemIndex != 0 || last.index != info.totalItemsCount - 2) {
                return@snapshotFlow Pair(false, null)
            }
            val end = last.offset + last.size + listState.firstVisibleItemScrollOffset
            // A gap above where the floating tab bar (and the nav bar) begins: the list's end padding less part of its own gap.
            val visibleEnd = info.viewportEndOffset - (info.afterContentPadding - barClearance)
            val overflow = end + info.afterContentPadding - info.viewportEndOffset
            Pair(end <= visibleEnd, if (overflow <= 0) 0 else (distance.roundToInt() - overflow).coerceAtLeast(0))
        }.collect { (f, r) ->
            fits = f
            if (f) room = 0 else if (r != null) room = r
        }
    }
    // Settles back to the top if the content shrank to fit while scrolled.
    LaunchedEffect(fits) { if (fits && listState.firstVisibleItemScrollOffset > 0) listState.animateScrollToItem(0) }

    // The title is one piece of text that travels: it rests in the hero, and as the page scrolls it
    // rises and shrinks into the bar, where a bubble of glass forms around it like water.
    // One style everywhere: a device name read smaller on its details page than on Home.
    // A long title (a rule's name) shrinks to fit one line, as iOS large titles do, rather than being
    // cut short; its copy in the bar then reads in full (it's the hero text scaled down, so a cut made
    // at hero size showed there too).
    val baseHero = MaterialTheme.typography.displayMedium
    val measurer = androidx.compose.ui.text.rememberTextMeasurer()
    val titleRoom = with(density) { (androidx.compose.ui.platform.LocalConfiguration.current.screenWidthDp.dp - 48.dp).roundToPx() }
    val heroStyle = remember(title, titleRoom, baseHero) {
        val w = measurer.measure(title, baseHero, maxLines = 1, softWrap = false).size.width
        if (w <= titleRoom) baseHero else baseHero.copy(fontSize = baseHero.fontSize * (titleRoom * 0.98f / w).coerceAtLeast(0.6f))
    }
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
    val haptics = LocalHapticFeedback.current
    // A list flung into either end lands with a soft tick (only while scrolling, so a page that
    // opens at the top stays quiet).
    LaunchedEffect(listState) {
        snapshotFlow { listState.canScrollBackward to listState.canScrollForward }.collect { (back, fwd) ->
            if (listState.isScrollInProgress && (!back || !fwd) && listState.layoutInfo.totalItemsCount > 0) haptics.perform(Haptic.Detent)
        }
    }
    val barCentre = statusTop + with(density) { BAR_HEIGHT.toPx() } / 2f
    // How far the travelling title sits below its place in the bar: 0 once docked.
    fun titleDrop(): Float {
        val rest = restY
        if (rest.isNaN()) return if (collapse < 1f) Float.MAX_VALUE else 0f
        val trip = (rest - barCentre).coerceAtLeast(1f)
        return trip - (collapse * maxOf(distance, trip / 0.9f)).coerceAtMost(trip)
    }
    val dockSlop = with(density) { 6.dp.toPx() }
    LaunchedEffect(Unit) {
        // The bubble belongs to the title: it forms as the title docks and bursts the moment it
        // leaves, so a fast fling never leaves an empty bubble fading behind the text.
        var first = true
        snapshotFlow { titleDrop() < dockSlop }.collect { on ->
            // Felt as it forms and as it bursts, not when a page opens already scrolled.
            if (!first && (on || bubbleH.value > 0f)) haptics.perform(Haptic.Bubble)
            first = false
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
                launch { bubblePop.animateTo(1f, tween(140, easing = FastOutSlowInEasing)) }
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
            userScrollEnabled = !fits,
            contentPadding = PaddingValues(
                start = side, end = side,
                bottom = 24.dp + bottomInset + WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding(),
            ),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.fillMaxSize().then(listModifier),
        ) {
            item(key = "header", contentType = "header") {
                // The title sits at the same height on every page (a fixed anchor, not centred with
                // whatever is around it: a subtitle lifted it 13 dp and the device glyph pushed it
                // down 46 dp). The glyph hangs above it and the subtitle below.
                Layout(
                    content = {
                        if (hero != null) Box(Modifier.layoutId(HERO_GLYPH)) { hero() }
                        // Holds the title's place, and shows it while the page rests at the top: drawn by the
                        // list, it stretches with everything else when the page is pulled down (the
                        // travelling copy sits outside the list, so it stayed put while the rest stretched).
                        Text(
                            title,
                            style = heroStyle,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier
                                .graphicsLayer { alpha = if (listState.atTop()) 1f else 0f }
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
                            Text(
                                subtitle, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.layoutId(HERO_SUBTITLE),
                            )
                        }
                    },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(headerHeight)
                        .graphicsLayer {
                            // Gone by three quarters, before the subtitle can pass under the title's bubble.
                            val c = (listState.firstVisibleItemScrollOffset / (distance * 0.75f)).coerceIn(0f, 1f)
                            alpha = 1f - c
                            compositingStrategy = CompositingStrategy.ModulateAlpha
                        }
                        .padding(horizontal = 12.dp),
                ) { measurables, c ->
                    val loose = c.copy(minWidth = 0, minHeight = 0)
                    val glyph = measurables.firstOrNull { it.layoutId == HERO_GLYPH }?.measure(loose)
                    val text = measurables.first { it.layoutId == null }.measure(loose)
                    val sub = measurables.firstOrNull { it.layoutId == HERO_SUBTITLE }?.measure(loose)
                    layout(c.maxWidth, c.maxHeight) {
                        val titleTop = (c.maxHeight * TITLE_ANCHOR - text.height / 2f).roundToInt()
                        glyph?.place((c.maxWidth - glyph.width) / 2, titleTop - 14.dp.roundToPx() - glyph.height)
                        text.place((c.maxWidth - text.width) / 2, titleTop)
                        sub?.place((c.maxWidth - sub.width) / 2, titleTop + text.height + 6.dp.roundToPx())
                    }
                }
            }
            content()
            item(key = ROOM_KEY, contentType = ROOM_KEY) {
                Spacer(Modifier.height(with(density) { room.toDp() }))
            }
        } }

        // The compact bar floats over the list in the same material as the tab bar: back and
        // actions are liquid-glass buttons, and the title's bubble forms in the middle. No strip and
        // no shade: content slides under clear glass.
        // Under the status bar, content blurs and fades away (iOS 26's soft edge) so the clock never
        // sits on raw cards; it reaches through the bar row and fades out below it (the user asked
        // for it longer and stronger than the first 12 dp).
        // Progressive: a light blur reaches 48 dp below the status bar, a heavy one thickens it under
        // the icons themselves (the user asked for it stronger again).
        val statusDp = with(density) { statusTop.toDp() }
        Box(Modifier.fillMaxWidth().height(statusDp + 48.dp).scrollEdge(backdrop, blur = 14.dp, tint = 0.4f) { !listState.atTop() })
        Box(Modifier.fillMaxWidth().height(statusDp + 18.dp).scrollEdge(backdrop, blur = 32.dp, tint = 0.35f) { !listState.atTop() })
        Box(Modifier.fillMaxWidth()) {
            // No shade behind the bar: the user wants the glass bubbles to show what passes under them
            // (a dark scroll-edge fade hid the refraction; removed 2026-10-31).
            Box(
                Modifier
                    .fillMaxWidth()
                    .windowInsetsPadding(WindowInsets.statusBars)
                    .height(BAR_HEIGHT)
                    // The page's gutter: the bubbles line up with the cards' edges.
                    .padding(horizontal = side),
            ) {
                if (navigation != null) {
                    Box(
                        Modifier.align(Alignment.CenterStart).size(CHROME_SIZE).glassChrome(backdrop, CircleShape, blur = 1.5.dp, bevel = 16.dp, interactive = true),
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
                            val h = (CHROME_SIZE.toPx() * bubbleH.value).coerceAtLeast(0f)
                            val w = maxOf(h, full * bubbleW.value).coerceAtLeast(0f)
                            val p = m.measure(Constraints.fixed(w.roundToInt(), h.roundToInt()))
                            layout(p.width, p.height) { p.place(0, 0) }
                        }
                        .graphicsLayer {
                            val pop = bubblePop.value
                            // Rides down with the leaving title and is gone within 16 dp of it.
                            val drop = titleDrop().coerceAtMost(size.height * 2f)
                            val away = ((drop - dockSlop) / 16.dp.toPx()).coerceIn(0f, 1f)
                            alpha = (1f - pop) * (1f - away)
                            translationY = drop
                            scaleX = 1f + 0.18f * maxOf(pop, away)
                            scaleY = 1f + 0.12f * maxOf(pop, away)
                        }
                        .drawWithContent { if (bubblePop.value < 1f) drawContent() }
                        .glassChrome(backdrop, CircleShape, blur = 1.5.dp, bevel = 16.dp),
                )
                Row(
                    Modifier.align(Alignment.CenterEnd).height(CHROME_SIZE).glassChrome(backdrop, CircleShape, blur = 1.5.dp, bevel = 16.dp, interactive = true),
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
                    val inBar = statusTop + BAR_HEIGHT.toPx() / 2f
                    if (rest.isNaN()) {
                        // Its resting place is only measured while the header is on screen. A page
                        // that comes back scrolled past it (from a rotation, say, leaving the PC
                        // screen) is fully collapsed, and there the title simply sits in the bar.
                        if (collapse < 1f) {
                            alpha = 0f
                            return@graphicsLayer
                        }
                        alpha = 1f
                        translationY = inBar - size.height / 2f
                        scaleX = shrink
                        scaleY = shrink
                        return@graphicsLayer
                    }
                    // At rest the list's own copy shows (see the placeholder); this one takes over the
                    // moment the page moves, at the same spot and size.
                    alpha = if (listState.atTop()) 0f else 1f
                    // It rides with the page until it reaches the bar (by 90% of the collapse at
                    // the latest), then stays there; it shrinks over the same trip.
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

/** Scrolled to the very top (pulling past it only stretches). */
private fun LazyListState.atTop() = firstVisibleItemIndex == 0 && firstVisibleItemScrollOffset == 0

private const val ROOM_KEY = "collapse-room"
private const val HERO_GLYPH = "hero-glyph"
private const val HERO_SUBTITLE = "hero-subtitle"
/** Where the hero title's centre sits, as a share of the hero's height, on every page. */
private const val TITLE_ANCHOR = 0.63f
private val BAR_HEIGHT = 64.dp
/** Every bubble in the bar (back, title, actions) is one height: a smaller title bubble looked like a different part. */
private val CHROME_SIZE = 48.dp

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
    /** 20 dp in a card; 0 where the container already insets (a dialog's text column). */
    horizontalPadding: Dp = 20.dp,
    subtitleMaxLines: Int = Int.MAX_VALUE,
    /** E.g. monospace for a command or a fingerprint. */
    subtitleStyle: TextStyle? = null,
    /** The row's one haptic: a plain tap, or e.g. Select when it picks one of several. */
    haptic: Haptic? = Haptic.Tap,
    /** False: shown quiet (40%) and untappable, e.g. an action that needs the PC while it's away. */
    enabled: Boolean = true,
    /**
     * At rest (a rule switched off): badge and words fade back while the row stays tappable and its
     * trailing control stays bright; the badge gives a small bounce as it changes.
     */
    resting: Boolean = false,
    onClick: (() -> Unit)? = null,
) {
    val rest by animateFloatAsState(if (resting) 1f else 0f, VyomaMotion.effects(), label = "rest")
    val bounce = remember { Animatable(1f) }
    var seen by remember { mutableStateOf(resting) }
    LaunchedEffect(resting) {
        if (seen == resting) return@LaunchedEffect
        seen = resting
        bounce.animateTo(0.8f, VyomaMotion.leading())
        bounce.animateTo(1f, VyomaMotion.jelly())
    }
    val quiet = Modifier.graphicsLayer { alpha = 1f - 0.5f * rest; compositingStrategy = CompositingStrategy.ModulateAlpha }
    // Where the row is, so a dialog it opens grows out of it (see MenuController.anchor).
    val menus = LocalMenu.current
    val placed = remember { arrayOfNulls<androidx.compose.ui.layout.LayoutCoordinates>(1) }
    Column(modifier) {
        Row(
            Modifier
                .fillMaxWidth()
                .onPlaced { placed[0] = it }
                .then(if (!enabled) Modifier.graphicsLayer { alpha = 0.4f; compositingStrategy = CompositingStrategy.ModulateAlpha } else Modifier)
                .then(if (onClick != null && enabled) Modifier.pressable(haptic = haptic, onClick = {
                    placed[0]?.takeIf { it.isAttached }?.let { menus.anchor(it.boundsInRoot()) }
                    onClick()
                }) else Modifier)
                .padding(horizontal = horizontalPadding, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (icon != null) {
                Box(quiet.graphicsLayer { scaleX = bounce.value; scaleY = bounce.value }) { IconBadge(icon, iconTint) }
                Spacer(Modifier.width(16.dp))
            }
            Column(Modifier.weight(1f).then(quiet)) {
                Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface)
                if (subtitle != null) {
                    Text(
                        subtitle,
                        style = subtitleStyle ?: MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = subtitleMaxLines,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (trailing != null) {
                Spacer(Modifier.width(12.dp))
                trailing()
            }
        }
        if (showDivider) {
            HorizontalDivider(
                Modifier.padding(start = horizontalPadding + if (icon != null) 50.dp else 0.dp, end = horizontalPadding),
                color = MaterialTheme.colorScheme.outlineVariant,
            )
        }
    }
}

/**
 * A row's trailing affordance: an optional state ("On", "3 PCs") and a chevron, so every row that
 * opens something looks like it does.
 */
@Composable
fun RowChevron(state: String? = null) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        if (state != null) {
            // Regular weight: a value at the title's size but bolder outweighed the row's own name.
            Text(state, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Normal), color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
            Spacer(Modifier.width(2.dp))
        }
        Icon(
            Icons.AutoMirrored.Rounded.KeyboardArrowRight, null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
            // The glyph's side bearing set chevrons 7 dp inside the edge switches and pills end on.
            modifier = Modifier.size(20.dp).offset(x = 6.dp),
        )
    }
}

/** iOS / One UI settings icon: a white glyph on a filled, softly rounded square of [tint], its edge catching the light. */
@Composable
fun IconBadge(icon: ImageVector, tint: Color, size: Dp = 34.dp) {
    Box(
        Modifier
            .size(size)
            .specularEdge(SmoothCornerShape(size * 0.28f), 0.8.dp, lit = Color.White.copy(alpha = 0.35f), base = Color.White.copy(alpha = 0.08f))
            .clip(SmoothCornerShape(size * 0.28f))
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
fun VyomaSwitch(checked: Boolean, onCheckedChange: (Boolean) -> Unit, modifier: Modifier = Modifier, contentDescription: String? = null) {
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val position = remember { Animatable(if (checked) 1f else 0f) }
    var held by remember { mutableStateOf(false) }
    val current by rememberUpdatedState(checked)
    val change by rememberUpdatedState(onCheckedChange)
    LaunchedEffect(checked) { if (!held) position.animateTo(if (checked) 1f else 0f, VyomaMotion.release()) }
    val swell = animateFloatAsState(if (held) 1f else 0f, if (held) VyomaMotion.spatial() else VyomaMotion.release(), label = "swell")
    val track by animateColorAsState(
        // The one filled accent (as on chips, pills and keys), not the pastel accent.
        if (checked) Glass.accent else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.16f),
        VyomaMotion.effects(), label = "track",
    )
    val card = MaterialTheme.colorScheme.surfaceContainerLow
    val padDp = 2.dp
    // iOS 26's switch: a wide, flat track (63 x 28 pt) and a capsule knob ~1.55 times as wide as tall
    // (a round knob was the iOS 7 one); held, it lifts into a bigger drop.
    fun DrawScope.thumb(): Rect {
        val pad = padDp.toPx()
        val d = size.height - 2 * pad
        val rest = d * KNOB_ASPECT
        val k = swell.value.coerceAtLeast(0f)
        val raw = position.value
        val p = raw.coerceIn(0f, 1f)
        // Overshoot past either end squashes the thumb against the wall instead of leaving the track.
        val squash = abs(raw - p) * d * 1.5f
        val h = d * (1f + 0.4f * k)
        val w = rest * (1f + 0.35f * k) - squash
        val cx = pad + rest / 2 + (size.width - 2 * pad - rest) * p
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
            .size(60.dp, 28.dp)
            .semantics {
                role = Role.Switch
                contentDescription?.let { this.contentDescription = it }
                toggleableState = ToggleableState(checked)
                onClick { settle(!checked); true }
            }
            .pointerInput(Unit) {
                val travel = size.width - 2 * padDp.toPx() - (size.height - 2 * padDp.toPx()) * KNOB_ASPECT
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
                        val tapped = c != null && !c.pressed && !c.isConsumed
                        // The tap is the switch's: a row it sits on (one that opens something) mustn't take it too.
                        if (tapped) c!!.consume()
                        settle(if (tapped) !current else current)
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
            prominent -> Glass.accent
            else -> accent.copy(alpha = 0.14f)
        },
        VyomaMotion.effects(), label = "pillBg",
    )
    val fg = when {
        !enabled -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
        prominent -> Color.White
        else -> accent
    }
    Box(
        modifier
            // 40 dp to look at; Compose already widens the touch target to 48 dp.
            .then(if (large) Modifier.fillMaxWidth().height(54.dp) else Modifier.height(44.dp))
            .pressable(enabled = enabled, haptic = haptic, glass = true, onClick = onClick)
            // Prominent pills are lit accent glass; the rest are raised glass with an accent label.
            .glassRaised(CircleShape, bg.takeIf { prominent && enabled })
            .padding(horizontal = 20.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = if (large) MaterialTheme.typography.titleSmall else MaterialTheme.typography.labelLarge, color = fg, maxLines = 1)
    }
}

/**
 * iOS 26 segmented control with the tab bar's liquid drop (same physics, user's call 2026-11-03):
 * the thumb slides between options as a gel (leading edge first, trailing edge catching up).
 * Touched, it lifts into a clear drop of liquid glass bigger than the track that magnifies the label
 * under it, follows the finger smoothly (a tick per option, a fine grain between), stretches along its
 * motion like a water droplet and wobbles back to round when it stops; let go, it settles on the
 * option under the thumb. Drawn, not laid out.
 */
@Composable
fun SegmentedControl(options: List<String>, selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier) {
    var width by remember { mutableStateOf(0) }
    val seg = if (options.isEmpty()) 0f else width.toFloat() / options.size
    val left = remember { Animatable(-1f) }
    val right = remember { Animatable(-1f) }
    val lift = remember { Animatable(0f) }
    var hover by remember { mutableIntStateOf(-1) }
    var pulse by remember { mutableStateOf<Job?>(null) }
    // The held drop's elastic stretch (+ longer, - taller) and its rate, as on the tab bar.
    var squash by remember { mutableFloatStateOf(0f) }
    var squashV by remember { mutableFloatStateOf(0f) }
    var elastic by remember { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val current by rememberUpdatedState(selected)
    val pick by rememberUpdatedState(onSelect)
    fun glide(toLeft: Float) {
        if (seg == 0f) return
        if (left.value < 0f) {
            scope.launch { left.snapTo(toLeft); right.snapTo(toLeft + seg) }
            return
        }
        // Already headed there: restarting would swap which edge leads mid-flight.
        if (toLeft == left.targetValue && toLeft + seg == right.targetValue) return
        val forward = toLeft > left.targetValue
        scope.launch { left.animateTo(toLeft, if (forward) VyomaMotion.trailing() else VyomaMotion.leading()) }
        scope.launch { right.animateTo(toLeft + seg, if (forward) VyomaMotion.leading() else VyomaMotion.trailing()) }
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
                    val speed = if (seg > 0f) abs(left.velocity + right.velocity) / 2f / seg else 0f
                    val target = (speed * 0.09f).coerceAtMost(0.32f)
                    squashV += (DROP_SQUASH_K * (target - squash) - DROP_SQUASH_C * squashV) * dt
                    squash += squashV * dt
                }
            } while (hover >= 0 || lift.value > 0.01f || abs(squash) > 0.002f || abs(squashV) > 0.02f)
            squash = 0f
            squashV = 0f
        }
    }
    LaunchedEffect(selected, seg) { if (hover < 0) glide(selected * seg) }
    val dark = isDarkUi
    val thumb = if (dark) Color(0xFF636366) else Color.White
    val shadow = Color.Black.copy(alpha = if (dark) 0f else 0.1f)
    val card = MaterialTheme.colorScheme.surfaceContainerLow
    val pad = with(LocalDensity.current) { 3.dp.toPx() }
    val overflow = with(LocalDensity.current) { 7.dp.toPx() }
    // The thumb in the control's coordinates: resting inside the padding; lifted, a drop outgrowing
    // the track, drawn in and stretched along its motion while it travels (the tab bar's pill()).
    fun DrawScope.thumbRect(): Rect {
        val l = left.value
        val w = right.value - l
        val stretch = if (seg > 0f) (w / seg - 1f).coerceIn(0f, 1f) else 0f
        val up = lift.value
        val rest = size.height - 2 * pad
        val sq = squash * up.coerceIn(0f, 1f)
        val shrink = 1f - 0.45f * sq.coerceAtLeast(0f)
        val h = (rest + (2 * overflow + 2 * pad) * up) * (1f - 0.14f * stretch) / (1f + sq) * shrink
        val ww = (w + 2 * overflow * up.coerceAtLeast(0f) * 1.6f + rest * 0.06f * up.coerceAtMost(0f)) * (1f + 0.4f * sq) * shrink
        val cx = pad + l + w / 2
        return Rect(Offset(cx - ww / 2, (size.height - h) / 2), Size(ww, h))
    }
    Box(
        modifier
            .fillMaxWidth()
            .height(40.dp)
            .liquidLens(null, strength = { lift.value }, zoom = { 1f + 0.1f * lift.value.coerceIn(0f, 1f) }, underlay = card, style = LensStyle.Lift, rect = { thumbRect() })
            .background(MaterialTheme.colorScheme.onSurface.copy(alpha = if (dark) 0.12f else 0.07f), CircleShape)
            .drawBehind {
                if (left.value < 0f) return@drawBehind
                // Gone early in the lift: magnified inside the growing drop, the thumb read as a haze.
                val a = if (liquidGlassSupported) 1f - (lift.value * 3f).coerceIn(0f, 1f) else 1f - lift.value.coerceIn(0f, 1f)
                if (a < 0.01f) return@drawBehind
                val r = thumbRect()
                val corner = CornerRadius(r.height / 2)
                drawRoundRect(shadow, r.topLeft + Offset(0f, 1.dp.toPx()), r.size, corner, alpha = a)
                drawRoundRect(thumb, r.topLeft, r.size, corner, alpha = a)
            }
            .pointerInput(options.size) {
                val lastIndex = options.lastIndex
                fun indexAt(x: Float) = ((x - pad) / (size.width - 2 * pad) * options.size).toInt().coerceIn(0, lastIndex)
                awaitEachGesture {
                    val down = awaitFirstDown()
                    hover = indexAt(down.position.x)
                    lifted(true)
                    glide(hover * seg)
                    if (hover != current) haptics.perform(Haptic.Select)
                    // The drop follows the finger itself, not just from option to option.
                    fun follow(x: Float) = glide((x - pad - seg / 2).coerceIn(0f, lastIndex * seg))
                    val grainEvery = 6.dp.toPx()
                    var slid = 0f
                    var lastX = down.position.x
                    var lastGrain = 0L
                    fun moved(c: PointerInputChange) {
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
                    // Sideways claims the gesture (the drop follows); a vertical swipe belongs to the page.
                    val drag = awaitHorizontalTouchSlopOrCancellation(down.id) { c, _ ->
                        c.consume()
                        moved(c)
                    }
                    val cancelled = if (drag != null) {
                        !horizontalDrag(drag.id) { c ->
                            c.consume()
                            moved(c)
                        }
                    } else {
                        val c = currentEvent.changes.firstOrNull { it.id == down.id }
                        c == null || c.pressed || c.isConsumed
                    }
                    val i = hover
                    hover = -1
                    lifted(false)
                    when {
                        cancelled -> glide(current * seg)
                        i != current -> { glide(i * seg); pick(i) }
                        else -> glide(i * seg)
                    }
                }
            }
            .padding(3.dp)
            .onSizeChanged { width = it.width },
    ) {
        Row(Modifier.fillMaxSize()) {
            options.forEachIndexed { i, label ->
                val on = i == selected
                // While held, the label under the drop reads as chosen (as the tab bar's do).
                val shown = if (hover >= 0) i == hover else on
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
                        color = if (shown) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                    )
                }
            }
        }
    }
}

/** The held drop's elastic spring (as the tab bar's): stiffness, and damping well under critical so it wobbles. */
private const val DROP_SQUASH_K = 260f
private const val DROP_SQUASH_C = 9f

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

/** Width over height of the switch's knob at rest, as iOS 26's. */
private const val KNOB_ASPECT = 1.55f

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
    if (r.width < 1f) return
    val a = alpha.coerceIn(0f, 1f)
    val corner = CornerRadius(r.height / 2)
    // Apple's knob shadow: soft and a little below, not a hard 1 dp ledge. It stays while the knob is
    // held (Apple's lifted knob casts one too), softer and wider as the drop lifts.
    val lift = 1f - a
    // Away from the light (follows the tilt, as the lifted drop's does).
    val fall = SpecularLight.cast((1.5f + 1.5f * lift).dp.toPx())
    with(SoftShadow) { draw(r, corner.x, Color.Black.copy(alpha = 0.22f - 0.06f * lift), (5f + 5f * lift).dp.toPx(), fall.y, fall.x) }
    if (a < 0.01f) return
    drawRoundRect(Color.White, r.topLeft, r.size, corner, alpha = a)
    drawRoundRect(
        Brush.verticalGradient(listOf(Color.White, Color.Black.copy(alpha = 0.06f)), startY = r.top, endY = r.bottom),
        r.topLeft, r.size, corner, alpha = a, style = Stroke(1.dp.toPx()),
    )
}

/**
 * A dialog in the app's material. Under a [MenuHost] it's liquid glass over the page, like the
 * menus ([DialogOverlay]); elsewhere (small activities with no page to blur) an opaque glass slab
 * in a window of its own. Same shape of API as Material's AlertDialog.
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
    val host = LocalMenu.current
    val body: @Composable () -> Unit = {
        Column(modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, top = 24.dp, bottom = 12.dp)) {
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
    if (host.hosted) {
        val entry = remember { HostedDialog(from = host.takeAnchor()) }
        // The latest content and dismiss, every composition (the dialog's state lives here).
        SideEffect {
            entry.card = body
            entry.onDismiss = onDismissRequest
        }
        DisposableEffect(entry) {
            host.dialogs += entry
            onDispose { entry.leaving = true }
        }
        return
    }
    Dialog(onDismissRequest) {
        Box(Modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.extraLarge, fill = MaterialTheme.colorScheme.surfaceContainerHigh)) { body() }
    }
}

/**
 * A dialog's action: a text button that dips, glows and ticks like every other control (Material's
 * TextButton only rippled, with no haptic). Same shape of API as TextButton; [destructive] colours
 * it red. 48 dp tall.
 */
@Composable
fun DialogButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    destructive: Boolean = false,
    haptic: Haptic? = Haptic.Tap,
    content: @Composable () -> Unit,
) {
    val color = when {
        !enabled -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
        destructive -> MaterialTheme.colorScheme.error
        else -> MaterialTheme.colorScheme.primary
    }
    Box(
        modifier
            .heightIn(min = 48.dp)
            .pressable(enabled = enabled, haptic = haptic, glass = true, onClick = onClick)
            .padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
    ) {
        CompositionLocalProvider(LocalContentColor provides color) {
            ProvideTextStyle(MaterialTheme.typography.labelLarge, content)
        }
    }
}

/** True inside an interactive [glassChrome] button: content shouldn't add its own glass. */
val LocalOnChrome = staticCompositionLocalOf { false }

/** The widest a page's column gets; wider windows centre it. */
val MaxContentWidth = 560.dp

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
        // On liquid chrome the chrome itself swells like a bubble, so the glyph holds still in it.
        modifier.size(size).pressable(enabled = enabled, pressedScale = if (LocalOnChrome.current) 1f else PressAuto, glass = !LocalOnChrome.current, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { content() }
}
