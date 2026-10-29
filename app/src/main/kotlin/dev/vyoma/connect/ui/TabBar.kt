package dev.vyoma.connect.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
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
import androidx.compose.foundation.layout.offset
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import dev.vyoma.connect.designsystem.Glass
import dev.vyoma.connect.designsystem.LocalBottomBarInset
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.backdropSource
import dev.vyoma.connect.designsystem.glassCard
import dev.vyoma.connect.designsystem.glassChrome
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.designsystem.rememberBackdrop

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
                    transitionSpec = { fadeIn(VyomaMotion.effects()) togetherWith fadeOut(VyomaMotion.effects()) },
                    label = "tab",
                ) { t -> holder.SaveableStateProvider(t.name) { content(t) } }
            }
        }

        Row(
            Modifier.align(Alignment.BottomCenter).padding(bottom = navBottom + 10.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            val index = Tab.entries.indexOf(tab)
            val lens by animateDpAsState(ItemWidth * index, VyomaMotion.spatial(), label = "lens")
            Box(Modifier.height(BarHeight).glassChrome(backdrop, CircleShape).padding(4.dp)) {
                // The selection lens glides between tabs with a little overshoot.
                Box(Modifier.offset(x = lens).width(ItemWidth).fillMaxHeight().glassCard(CircleShape, fill = Glass.raised))
                Row {
                    Tab.entries.forEach { t -> TabItem(t, selected = t == tab) { onTab(t) } }
                }
            }
            Box(
                Modifier.size(BarHeight).glassChrome(backdrop, CircleShape).pressable(pressedScale = 0.9f, onClick = onTouchpad),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.TouchApp, "Touchpad", tint = MaterialTheme.colorScheme.onBackground)
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
        Modifier.width(ItemWidth).fillMaxHeight().pressable(pressedScale = 0.9f, onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(tab.icon, null, tint = color, modifier = Modifier.size(24.dp))
        Spacer(Modifier.height(2.dp))
        Text(tab.label, style = MaterialTheme.typography.labelSmall, color = color)
    }
}
