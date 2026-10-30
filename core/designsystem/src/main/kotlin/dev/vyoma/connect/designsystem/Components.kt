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
import kotlin.math.abs
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
    val headerPx = with(LocalDensity.current) { headerHeight.toPx() }
    val collapse by remember {
        derivedStateOf {
            if (listState.firstVisibleItemIndex > 0) 1f
            else (listState.firstVisibleItemScrollOffset / (headerPx * 0.6f)).coerceIn(0f, 1f)
        }
    }
    // One UI never rests half-collapsed: when a scroll ends inside the header, settle open or
    // closed, whichever is nearer. A page that only overflows by a sliver therefore springs back
    // instead of sitting a few pixels down.
    LaunchedEffect(listState, headerPx) {
        val distance = (headerPx * 0.6f).toInt()
        snapshotFlow { listState.isScrollInProgress }.collect { scrolling ->
            if (scrolling || listState.firstVisibleItemIndex != 0) return@collect
            val offset = listState.firstVisibleItemScrollOffset
            if (offset in 1 until distance) {
                listState.animateScrollBy((if (offset < distance / 2) -offset else distance - offset).toFloat(), VyomaMotion.page())
            }
        }
    }

    val backdrop = rememberBackdrop()
    val bottomInset = LocalBottomBarInset.current
    // Content color for everything on the canvas, so bar icons and titles are legible.
    CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onBackground) { Box(modifier.fillMaxSize()) {
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
                            val c = (listState.firstVisibleItemScrollOffset / (headerPx * 0.6f)).coerceIn(0f, 1f)
                            alpha = 1f - c
                            compositingStrategy = CompositingStrategy.ModulateAlpha
                            translationY = listState.firstVisibleItemScrollOffset * 0.35f
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
                    Text(
                        title,
                        style = if (hero != null) MaterialTheme.typography.headlineLarge else MaterialTheme.typography.displayMedium,
                        color = MaterialTheme.colorScheme.onBackground,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (subtitle != null) {
                        Spacer(Modifier.height(6.dp))
                        Text(subtitle, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            content()
        } }

        // The compact bar floats over the list in the same material as the tab bar: back and
        // actions are liquid-glass buttons, and once the hero has scrolled away the title rises in
        // a glass capsule of its own. Instead of a hard frosted strip, the status bar area fades
        // softly to the canvas so content slips under it.
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
                    .height(64.dp)
                    .padding(horizontal = 12.dp),
            ) {
                if (navigation != null) {
                    Box(
                        Modifier.align(Alignment.CenterStart).size(48.dp).glassChrome(backdrop, CircleShape, blur = 8.dp, bevel = 16.dp, interactive = true),
                        contentAlignment = Alignment.Center,
                    ) { CompositionLocalProvider(LocalOnChrome provides true) { navigation() } }
                }
                Box(
                    Modifier
                        .align(Alignment.Center)
                        .padding(horizontal = 60.dp)
                        // Only once the big title is mostly gone, so a small scroll never shows two titles.
                        .graphicsLayer {
                            val t = ((collapse - 0.6f) / 0.4f).coerceIn(0f, 1f)
                            alpha = t
                            compositingStrategy = CompositingStrategy.ModulateAlpha
                            translationY = (1f - t) * 10.dp.toPx()
                            scaleX = 0.9f + 0.1f * t
                            scaleY = 0.9f + 0.1f * t
                        }
                        .drawWithContent { if (collapse > 0.6f) drawContent() }
                        .glassChrome(backdrop, CircleShape, blur = 8.dp, bevel = 16.dp)
                        .padding(horizontal = 18.dp, vertical = 10.dp),
                ) {
                    Text(title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Row(
                    Modifier.align(Alignment.CenterEnd).height(48.dp).glassChrome(backdrop, CircleShape, blur = 8.dp, bevel = 16.dp, interactive = true),
                    verticalAlignment = Alignment.CenterVertically,
                ) { CompositionLocalProvider(LocalOnChrome provides true) { actions() } }
            }
        }
    } }
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

/** iOS / One UI settings icon: a white glyph on a filled, softly rounded square of [tint]. */
@Composable
fun IconBadge(icon: ImageVector, tint: Color, size: Dp = 34.dp) {
    Box(
        Modifier
            .size(size)
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
 * iOS / One UI switch with a liquid thumb: it swells while your finger is on it, then slides
 * across with a little bounce that squashes against the end. Toggling on and off feel different.
 * Everything moves in the draw phase.
 */
@Composable
fun VyomaSwitch(checked: Boolean, onCheckedChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val haptics = LocalHapticFeedback.current
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val position = animateFloatAsState(if (checked) 1f else 0f, VyomaMotion.release(), label = "thumb")
    val swell = animateFloatAsState(if (pressed) 1f else 0f, VyomaMotion.spatial(), label = "swell")
    val track by animateColorAsState(
        if (checked) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.16f),
        VyomaMotion.effects(), label = "track",
    )
    Canvas(
        modifier
            .size(52.dp, 32.dp)
            .toggleable(checked, interaction, indication = null, role = Role.Switch) {
                haptics.perform(if (it) Haptic.ToggleOn else Haptic.ToggleOff)
                onCheckedChange(it)
            },
    ) {
        drawRoundRect(track, cornerRadius = CornerRadius(size.height / 2))
        val pad = 2.5.dp.toPx()
        val d = size.height - 2 * pad
        val raw = position.value
        val p = raw.coerceIn(0f, 1f)
        // Overshoot past either end squashes the thumb against the wall instead of leaving the track.
        val squash = abs(raw - p) * d * 1.5f
        val w = d + d * 0.3f * swell.value - squash
        val x = pad + (size.width - 2 * pad - w) * p
        val r = CornerRadius(d / 2)
        drawRoundRect(Color.Black.copy(alpha = 0.12f), Offset(x, pad + 1.dp.toPx()), Size(w, d), r)
        drawRoundRect(Color.White, Offset(x, pad), Size(w, d), r)
    }
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
 * iOS-style segmented control: a glass track with a raised thumb that slides between options as a
 * gel (leading edge first, trailing edge catching up). Drawn, not laid out, so the slide costs one
 * draw per frame.
 */
@Composable
fun SegmentedControl(options: List<String>, selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier) {
    var width by remember { mutableStateOf(0) }
    val seg = if (options.isEmpty()) 0f else width.toFloat() / options.size
    val left = remember { Animatable(-1f) }
    val right = remember { Animatable(-1f) }
    LaunchedEffect(selected, seg) {
        if (seg == 0f) return@LaunchedEffect
        val l = selected * seg
        if (left.value < 0f) {
            left.snapTo(l); right.snapTo(l + seg)
            return@LaunchedEffect
        }
        val forward = l > left.value
        launch { left.animateTo(l, if (forward) VyomaMotion.trailing() else VyomaMotion.leading()) }
        launch { right.animateTo(l + seg, if (forward) VyomaMotion.leading() else VyomaMotion.trailing()) }
    }
    val dark = isDarkUi
    val thumb = if (dark) Color.White.copy(alpha = 0.2f) else Color.White
    val shadow = Color.Black.copy(alpha = if (dark) 0f else 0.08f)
    Row(
        modifier
            .fillMaxWidth()
            .height(40.dp)
            .clip(CircleShape)
            .background(MaterialTheme.colorScheme.onSurface.copy(alpha = 0.07f))
            .padding(3.dp)
            .onSizeChanged { width = it.width }
            .drawBehind {
                if (left.value < 0f) return@drawBehind
                val r = CornerRadius(size.height / 2)
                val w = right.value - left.value
                drawRoundRect(shadow, Offset(left.value, 1.dp.toPx()), Size(w, size.height), r)
                drawRoundRect(thumb, Offset(left.value, 0f), Size(w, size.height), r)
            },
    ) {
        options.forEachIndexed { i, label ->
            val on = i == selected
            Box(
                Modifier
                    .weight(1f)
                    .fillMaxSize()
                    .pressable(pressedScale = 0.94f, role = Role.Tab, haptic = Haptic.Select) { if (!on) onSelect(i) },
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

/**
 * The liquid bar: a slim capsule with no thumb. Under a finger it swells (springy, like a drop
 * pressed flat) and follows the finger's movement relatively, iOS-style, so a touch never makes
 * it jump; a tap without a drag changes nothing. [value] is read in the draw phase only.
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
    rest: Dp = 6.dp,
    swollen: Dp = 14.dp,
    /** What the slider sits on, so its drop has something to refract around the track. */
    underlay: Color = Color.Transparent,
) {
    val swell = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val change by rememberUpdatedState(onChange)
    val done by rememberUpdatedState(onDone)
    Box(
        modifier
            .fillMaxWidth()
            .height(32.dp)
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
                                if ((n == 0f || n == 1f) && n != v) haptics.perform(Haptic.Edge)
                                v = n
                                change(v)
                            }
                        }
                        if (moved) done()
                        scope.launch { swell.animateTo(0f, VyomaMotion.release()) }
                    }
                },
            )
            // The knob is always there: a frosted bead at the end of the fill that, grabbed, swells
            // and clears into a drop of water magnifying the fill, and frosts over again when let go.
            // At rest it's a plain drawn bead; the lens takes over as it grows.
            .drawWithContent {
                drawContent()
                frostedKnob(sliderKnob(value(), swell.value, swollen), 1f - swell.value)
            }
            .liquidLens(
                null,
                strength = { swell.value },
                zoom = { 1f + 0.18f * swell.value.coerceIn(0f, 1f) },
                underlay = underlay,
                frost = { 1f - swell.value },
                rect = { sliderKnob(value(), swell.value, swollen) },
            )
            .drawBehind {
                val h = lerp(rest.toPx(), swollen.toPx(), swell.value.coerceAtLeast(0f))
                val top = (size.height - h) / 2
                val r = CornerRadius(h / 2)
                drawRoundRect(track, Offset(0f, top), Size(size.width, h), r)
                val w = (value().coerceIn(0f, 1f) * size.width)
                if (w <= 0f) return@drawBehind
                val fw = w.coerceAtLeast(h)
                drawRoundRect(fill, Offset(0f, top), Size(fw, h), r)
                // A gloss along the top of the fill: lit liquid rather than flat paint.
                drawRoundRect(
                    Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.35f), Color.Transparent), startY = top, endY = top + h * 0.6f),
                    Offset(0f, top), Size(fw, h), r,
                )
            },
    )
}

private fun DrawScope.sliderKnob(value: Float, swell: Float, swollen: Dp): Rect {
    val k = swell.coerceAtLeast(0f)
    val dh = lerp(swollen.toPx() + 2.dp.toPx(), size.height * 1.1f, k)
    val dw = dh * lerp(1.2f, 1.5f, k.coerceAtMost(1f))
    val x = (value.coerceIn(0f, 1f) * size.width).coerceIn(dw / 2, size.width - dw / 2)
    return Rect(x - dw / 2, (size.height - dh) / 2, x + dw / 2, (size.height + dh) / 2)
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
    drawRoundRect(Color(0xFFF7F7F9), r.topLeft, r.size, corner, alpha = a)
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
