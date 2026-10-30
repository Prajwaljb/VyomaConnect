package dev.vyoma.connect.designsystem

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
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
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import kotlin.math.abs
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.LaunchedEffect
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
    content: LazyListScope.() -> Unit,
) {
    val listState = rememberLazyListState()
    val screenHeight = LocalConfiguration.current.screenHeightDp.dp
    val headerHeight = (screenHeight * 0.30f).coerceIn(180.dp, 300.dp)
    val headerPx = with(LocalDensity.current) { headerHeight.toPx() }
    val collapse by remember {
        derivedStateOf {
            if (listState.firstVisibleItemIndex > 0) 1f
            else (listState.firstVisibleItemScrollOffset / (headerPx * 0.6f)).coerceIn(0f, 1f)
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
            modifier = Modifier.fillMaxSize(),
        ) {
            item(key = "header", contentType = "header") {
                Column(
                    Modifier
                        .fillMaxWidth()
                        .height(headerHeight)
                        .graphicsLayer {
                            val c = (listState.firstVisibleItemScrollOffset / (headerPx * 0.6f)).coerceIn(0f, 1f)
                            alpha = 1f - c
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

        // Compact bar: always present for navigation/actions. Once the hero has scrolled away it
        // turns into frosted glass over the list, and its title fades in.
        val frosted by remember { derivedStateOf { collapse > 0.98f } }
        Box(Modifier.fillMaxWidth()) {
            // Only composed while collapsed, so the expanded page never pays for the blur.
            if (frosted) Box(Modifier.matchParentSize().glassChrome(backdrop, RectangleShape, edge = false))
            Row(
                Modifier
                    .fillMaxWidth()
                    .windowInsetsPadding(WindowInsets.statusBars)
                    .height(56.dp)
                    .padding(horizontal = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (navigation != null) navigation() else Spacer(Modifier.width(8.dp))
                Text(
                    title,
                    style = MaterialTheme.typography.titleLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier
                        .weight(1f)
                        .padding(horizontal = 8.dp)
                        .graphicsLayer {
                            alpha = collapse
                            translationY = (1f - collapse) * 12.dp.toPx()
                        },
                )
                actions()
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
    // Offline rests: no frame clock running for a gray dot.
    val pulse: State<Float> = if (state == OrbState.OFFLINE) remember { mutableFloatStateOf(0f) } else {
        rememberInfiniteTransition(label = "orb").animateFloat(
            initialValue = 0f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(tween(if (state == OrbState.PENDING) 700 else 2200, easing = LinearEasing), RepeatMode.Restart),
            label = "pulse",
        )
    }
    Canvas(modifier.size(size * 2.4f)) {
        val r = size.toPx() / 2
        val c = Offset(this.size.width / 2, this.size.height / 2)
        if (state != OrbState.OFFLINE) {
            val p = pulse.value
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
            .pressable(enabled = enabled, pressedScale = if (large) 0.97f else 0.92f, haptic = haptic, onClick = onClick)
            .clip(CircleShape)
            .background(bg)
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
