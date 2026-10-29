package dev.vyoma.connect.designsystem

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
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
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

@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.padding(start = 20.dp, top = 12.dp, bottom = 0.dp),
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
    val transition = rememberInfiniteTransition(label = "orb")
    val period = if (state == OrbState.PENDING) 700 else 2200
    val pulse by transition.animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(period, easing = LinearEasing), RepeatMode.Restart),
        label = "pulse",
    )
    Canvas(modifier.size(size * 2.4f)) {
        val r = size.toPx() / 2
        val c = Offset(this.size.width / 2, this.size.height / 2)
        if (state != OrbState.OFFLINE) {
            drawCircle(color.copy(alpha = 0.35f * (1f - pulse)), radius = r * (1f + pulse * 1.4f), center = c)
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
