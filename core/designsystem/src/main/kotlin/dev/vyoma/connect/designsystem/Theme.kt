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
    val amoled: Boolean = false,
)

// One UI-like neutral surfaces: soft gray canvas, bright cards that float on it.
private val LightCanvas = Color(0xFFF3F4F6)
private val LightCard = Color(0xFFFFFFFF)
private val DarkCanvas = Color(0xFF0E0F11)
private val DarkCard = Color(0xFF1C1C1E)

@Composable
fun VyomaTheme(settings: ThemeSettings = ThemeSettings(), content: @Composable () -> Unit) {
    val dark = when (settings.darkMode) {
        DarkMode.SYSTEM -> isSystemInDarkTheme()
        DarkMode.LIGHT -> false
        DarkMode.DARK -> true
    }
    val context = LocalContext.current
    val target = remember(settings, dark) {
        val accentSource: ColorScheme? = if (settings.palette == Palette.DYNAMIC && Build.VERSION.SDK_INT >= 31) {
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        } else {
            null
        }
        buildScheme(settings, dark, accentSource)
    }
    MaterialTheme(
        colorScheme = target.animated(),
        typography = VyomaTypography,
        shapes = VyomaShapes,
        content = content,
    )
}

private fun buildScheme(settings: ThemeSettings, dark: Boolean, dynamic: ColorScheme?): ColorScheme {
    val accent = dynamic?.primary ?: if (dark) settings.palette.dark else settings.palette.light
    val canvas = if (dark) { if (settings.amoled) Color.Black else DarkCanvas } else LightCanvas
    val card = if (dark) DarkCard else LightCard
    val onSurface = if (dark) Color(0xFFF2F3F5) else Color(0xFF111316)
    val muted = if (dark) Color(0xFF9EA3AB) else Color(0xFF5F6670)
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
    medium = RoundedCornerShape(20.dp),
    large = RoundedCornerShape(26.dp),
    extraLarge = RoundedCornerShape(32.dp),
)
