package dev.vyoma.connect.designsystem

import android.os.Build
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp

/** Accent families. Surfaces are always neutral, so the accent is the only color. */
enum class Palette(val label: String, val light: Color, val dark: Color) {
    DYNAMIC("Wallpaper", Color(0xFF3B82F6), Color(0xFF7DB4FF)),
    OCEAN("Ocean", Color(0xFF1A73E8), Color(0xFF8AB8FF)),
    GRAPHITE("Graphite", Color(0xFF46515E), Color(0xFFB7C2CF)),
    EMBER("Ember", Color(0xFFE8572A), Color(0xFFFF9A73)),
    MINT("Mint", Color(0xFF0F9D76), Color(0xFF5ADBB3)),
    SAND("Sand", Color(0xFFB07A2C), Color(0xFFE8BC7A)),
}

enum class DarkMode { SYSTEM, LIGHT, DARK }

@Immutable
data class ThemeSettings(
    val palette: Palette = Palette.DYNAMIC,
    val darkMode: DarkMode = DarkMode.SYSTEM,
)

// One UI-like neutral surfaces: soft gray canvas, bright cards that stand clearly off it. Dark is
// true black (AMOLED): pixels off, glass cards lit against it.
private val LightCanvas = Color(0xFFEDEEF2)
private val LightCard = Color(0xFFFFFFFF)
private val DarkCard = Color(0xFF1C1C1E)

@Composable
fun VyomaTheme(settings: ThemeSettings = ThemeSettings(), content: @Composable () -> Unit) {
    val dark = when (settings.darkMode) {
        DarkMode.SYSTEM -> isSystemInDarkTheme()
        DarkMode.LIGHT -> false
        DarkMode.DARK -> true
    }
    val context = LocalContext.current
    remember(context) { Haptics.attach(context) }
    val target = remember(settings, dark) {
        val accentSource: ColorScheme? = if (settings.palette == Palette.DYNAMIC && Build.VERSION.SDK_INT >= 31) {
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        } else {
            null
        }
        buildScheme(settings, dark, accentSource)
    }
    TrackSpecularLight()
    MaterialTheme(
        colorScheme = target.animated(),
        typography = VyomaTypography,
        shapes = VyomaShapes,
        content = content,
    )
}

private fun buildScheme(settings: ThemeSettings, dark: Boolean, dynamic: ColorScheme?): ColorScheme {
    // Material You's accents are muted; a little more colour at the same lightness reads crisper
    // without changing how legible a label on it is.
    val accent = dynamic?.primary?.saturate(1.35f) ?: if (dark) settings.palette.dark else settings.palette.light
    val canvas = if (dark) Color.Black else LightCanvas
    val card = if (dark) DarkCard else LightCard
    // Secondary text is where contrast was weakest: lifted toward the primary, still clearly second.
    val onSurface = if (dark) Color(0xFFF7F8FA) else Color(0xFF0B0C0E)
    val muted = if (dark) Color(0xFFAEB3BB) else Color(0xFF50565F)
    val container = accent.blend(card, if (dark) 0.78f else 0.86f)
    val onAccent = if (accent.luminance() > 0.45f) Color(0xFF0B0C0E) else Color.White
    val base = if (dark) darkColorScheme() else lightColorScheme()
    return base.copy(
        primary = accent,
        onPrimary = onAccent,
        primaryContainer = container,
        onPrimaryContainer = onSurface,
        secondary = dynamic?.secondary ?: accent,
        onSecondary = onAccent,
        secondaryContainer = container,
        onSecondaryContainer = onSurface,
        tertiary = dynamic?.tertiary ?: Status.connected(dark),
        background = canvas,
        onBackground = onSurface,
        surface = canvas,
        onSurface = onSurface,
        onSurfaceVariant = muted,
        surfaceContainerLowest = card,
        surfaceContainerLow = card,
        surfaceContainer = card,
        surfaceContainerHigh = card.blend(onSurface, 0.06f),
        surfaceContainerHighest = card.blend(onSurface, 0.10f),
        surfaceVariant = card.blend(onSurface, 0.08f),
        outline = muted.copy(alpha = 0.5f),
        outlineVariant = onSurface.copy(alpha = 0.08f),
        error = if (dark) Color(0xFFFF7A6E) else Color(0xFFD93025),
    )
}

/** Semantic status colors, kept identical across palettes so states read the same everywhere. */
object Status {
    fun connected(dark: Boolean) = if (dark) Color(0xFF4ADE80) else Color(0xFF16A34A)
    fun pending(dark: Boolean) = if (dark) Color(0xFFFBBF24) else Color(0xFFD97706)
    fun offline(dark: Boolean) = if (dark) Color(0xFF6B7280) else Color(0xFF9CA3AF)
}

private fun Color.blend(other: Color, otherWeight: Float) = Color(
    red = red * (1 - otherWeight) + other.red * otherWeight,
    green = green * (1 - otherWeight) + other.green * otherWeight,
    blue = blue * (1 - otherWeight) + other.blue * otherWeight,
    alpha = 1f,
)

/** More (or less) chroma at the same HSL lightness. */
private fun Color.saturate(by: Float): Color = toned(saturation = by)

/**
 * The same hue with its HSL saturation scaled by [saturation] and, if given, its [lightness] set:
 * e.g. a deep shade of the accent for a white glyph to sit on.
 */
fun Color.toned(saturation: Float = 1f, lightness: Float? = null): Color {
    val max = maxOf(red, green, blue)
    val min = minOf(red, green, blue)
    val l = (max + min) / 2f
    if (max == min) return lightness?.let { Color(it, it, it, alpha) } ?: this
    val d = max - min
    val s = if (l > 0.5f) d / (2f - max - min) else d / (max + min)
    val h = when (max) {
        red -> ((green - blue) / d + (if (green < blue) 6f else 0f))
        green -> (blue - red) / d + 2f
        else -> (red - green) / d + 4f
    } / 6f
    val s2 = (s * saturation).coerceAtMost(1f)
    val l2 = lightness ?: l
    val q = if (l2 < 0.5f) l2 * (1f + s2) else l2 + s2 - l2 * s2
    val p = 2f * l2 - q
    fun hue(t0: Float): Float {
        val t = (t0 + 1f) % 1f
        return when {
            t < 1f / 6f -> p + (q - p) * 6f * t
            t < 1f / 2f -> q
            t < 2f / 3f -> p + (q - p) * (2f / 3f - t) * 6f
            else -> p
        }
    }
    return Color(hue(h + 1f / 3f), hue(h), hue(h - 1f / 3f), alpha)
}

private fun Color.luminance() = 0.2126f * red + 0.7152f * green + 0.0722f * blue

/** Theme changes cross-fade instead of snapping. */
@Composable
private fun ColorScheme.animated(): ColorScheme {
    val spec = tween<Color>(durationMillis = 380)
    @Composable fun Color.a() = animateColorAsState(this, spec, label = "scheme").value
    return copy(
        primary = primary.a(),
        onPrimary = onPrimary.a(),
        primaryContainer = primaryContainer.a(),
        secondaryContainer = secondaryContainer.a(),
        background = background.a(),
        surface = surface.a(),
        onSurface = onSurface.a(),
        onBackground = onBackground.a(),
        onSurfaceVariant = onSurfaceVariant.a(),
        surfaceContainerLowest = surfaceContainerLowest.a(),
        surfaceContainerLow = surfaceContainerLow.a(),
        surfaceContainer = surfaceContainer.a(),
        surfaceContainerHigh = surfaceContainerHigh.a(),
    )
}

val VyomaShapes = Shapes(
    extraSmall = RoundedCornerShape(10.dp),
    small = RoundedCornerShape(14.dp),
    // Cards, tiles and sheets: Apple's continuous corners (macOS/iOS 26 tiles), not circular arcs.
    medium = SmoothCornerShape(20.dp),
    large = SmoothCornerShape(26.dp),
    extraLarge = SmoothCornerShape(32.dp),
)
