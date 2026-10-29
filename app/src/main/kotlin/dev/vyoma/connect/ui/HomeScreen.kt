package dev.vyoma.connect.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Tablet
import androidx.compose.material.icons.rounded.Tv
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ConnectionOrb
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.OrbState
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.link.TransportKind

@Composable
fun HomeScreen(vm: AppViewModel, onOpenDevice: (String) -> Unit, onOpenSettings: () -> Unit) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val connected = devices.filter { it.isPaired && it.isReachable }
    val pairing = devices.filter { it.pairState == PairState.REQUESTED_BY_PEER || it.pairState == PairState.REQUESTED_BY_US }
    val available = devices.filter { !it.isPaired && it.isReachable && it !in pairing }
    val offline = devices.filter { it.isPaired && !it.isReachable }

    val subtitle = when {
        connected.size == 1 -> "Connected to ${connected.first().identity.deviceName}"
        connected.size > 1 -> "${connected.size} devices connected"
        else -> "Looking for devices nearby"
    }

    CollapsingHeaderScaffold(
        title = "Vyoma",
        subtitle = subtitle,
        actions = {
            IconButton(onClick = vm::refresh) { Icon(Icons.Rounded.Refresh, "Refresh") }
            IconButton(onClick = onOpenSettings) { Icon(Icons.Rounded.Settings, "Settings") }
        },
    ) {
        items(connected, key = { "c" + it.identity.deviceId }) { state ->
            DeviceHeroCard(
                state = state,
                onClick = { onOpenDevice(state.identity.deviceId) },
                onPing = { vm.ping(state.identity.deviceId) },
                modifier = Modifier.animateItem(),
            )
        }

        if (pairing.isNotEmpty()) {
            item(key = "pairing-label") { SectionLabel("Pairing", Modifier.animateItem()) }
            items(pairing, key = { "p" + it.identity.deviceId }) { state ->
                PairingCard(
                    state = state,
                    onAccept = { vm.acceptPair(state.identity.deviceId) },
                    onReject = { vm.rejectPair(state.identity.deviceId) },
                    modifier = Modifier.animateItem(),
                )
            }
        }

        if (available.isNotEmpty()) {
            item(key = "available-label") { SectionLabel("Available to pair", Modifier.animateItem()) }
            item(key = "available") {
                SectionCard(Modifier.animateItem()) {
                    available.forEachIndexed { i, state ->
                        ListRow(
                            title = state.identity.deviceName,
                            subtitle = transportLabel(state),
                            icon = state.identity.deviceType.icon(),
                            showDivider = i < available.lastIndex,
                            trailing = {
                                FilledTonalButton(onClick = { vm.requestPair(state.identity.deviceId) }) { Text("Pair") }
                            },
                        )
                    }
                }
            }
        }

        if (offline.isNotEmpty()) {
            item(key = "offline-label") { SectionLabel("Paired · not reachable", Modifier.animateItem()) }
            item(key = "offline") {
                SectionCard(Modifier.animateItem()) {
                    offline.forEachIndexed { i, state ->
                        ListRow(
                            title = state.identity.deviceName,
                            subtitle = "Open KDE Connect on it, or join the same network",
                            icon = state.identity.deviceType.icon(),
                            iconTint = MaterialTheme.colorScheme.onSurfaceVariant,
                            showDivider = i < offline.lastIndex,
                            trailing = { ConnectionOrb(OrbState.OFFLINE, size = 10.dp) },
                            onClick = { onOpenDevice(state.identity.deviceId) },
                        )
                    }
                }
            }
        }

        if (connected.isEmpty() && pairing.isEmpty() && available.isEmpty()) {
            item(key = "searching") { SearchingCard(Modifier.animateItem()) }
        }
    }
}

@Composable
private fun DeviceHeroCard(state: DeviceState, onClick: () -> Unit, onPing: () -> Unit, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier.fillMaxWidth().pressable(pressedScale = 0.975f, onClick = onClick),
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainerLowest,
    ) {
        Column(Modifier.padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    Modifier
                        .size(56.dp)
                        .clip(MaterialTheme.shapes.medium)
                        .background(MaterialTheme.colorScheme.primaryContainer),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(state.identity.deviceType.icon(), null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(30.dp))
                }
                Spacer(Modifier.width(16.dp))
                Column(Modifier.weight(1f)) {
                    Text(state.identity.deviceName, style = MaterialTheme.typography.titleLarge)
                    Spacer(Modifier.height(4.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        ConnectionOrb(OrbState.CONNECTED, size = 8.dp)
                        Text(transportLabel(state), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            Spacer(Modifier.height(16.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                QuickAction(Icons.Rounded.NotificationsActive, "Ping", onPing)
            }
        }
    }
}

@Composable
private fun QuickAction(icon: ImageVector, label: String, onClick: () -> Unit) {
    Column(
        Modifier
            .clip(MaterialTheme.shapes.medium)
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .pressable(onClick = onClick)
            .padding(horizontal = 18.dp, vertical = 12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, null, tint = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.height(4.dp))
        Text(label, style = MaterialTheme.typography.labelMedium)
    }
}

/** Shown inline (and as a notification) so the code can be compared side by side with the PC. */
@Composable
private fun PairingCard(state: DeviceState, onAccept: () -> Unit, onReject: () -> Unit, modifier: Modifier = Modifier) {
    val incoming = state.pairState == PairState.REQUESTED_BY_PEER
    SectionCard(modifier.animateContentSize(), contentPadding = PaddingValues(20.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            ConnectionOrb(OrbState.PENDING, size = 10.dp)
            Spacer(Modifier.width(6.dp))
            Text(
                if (incoming) "${state.identity.deviceName} wants to pair" else "Waiting for ${state.identity.deviceName}…",
                style = MaterialTheme.typography.titleMedium,
            )
        }
        Spacer(Modifier.height(12.dp))
        Text("Make sure this code matches on both screens", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(8.dp))
        Text(
            (state.verificationKey ?: "········").chunked(4).joinToString("  "),
            style = MaterialTheme.typography.displayMedium.copy(fontFamily = FontFamily.Monospace),
            color = MaterialTheme.colorScheme.primary,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth(),
        )
        AnimatedVisibility(incoming, enter = fadeIn(), exit = fadeOut()) {
            Row(Modifier.fillMaxWidth().padding(top = 16.dp), horizontalArrangement = Arrangement.End) {
                TextButton(onClick = onReject) { Text("Decline") }
                Spacer(Modifier.width(8.dp))
                FilledTonalButton(onClick = onAccept) { Text("Pair") }
            }
        }
    }
}

/** Radar sweep while nothing is around, with the two tips that solve 90% of "not found". */
@Composable
private fun SearchingCard(modifier: Modifier = Modifier) {
    val color = MaterialTheme.colorScheme.primary
    val t = rememberInfiniteTransition(label = "radar")
    val wave by t.animateFloat(0f, 1f, infiniteRepeatable(tween(2400, easing = LinearEasing), RepeatMode.Restart), label = "wave")
    SectionCard(modifier, contentPadding = PaddingValues(24.dp)) {
        Box(Modifier.fillMaxWidth().height(140.dp), contentAlignment = Alignment.Center) {
            Canvas(Modifier.size(140.dp)) {
                for (i in 0 until 3) {
                    val p = (wave + i / 3f) % 1f
                    drawCircle(color.copy(alpha = 0.5f * (1f - p)), radius = size.minDimension / 2 * p, style = Stroke(width = 2.dp.toPx()))
                }
                drawCircle(color, radius = 8.dp.toPx())
            }
        }
        Text("Searching your network", style = MaterialTheme.typography.titleMedium, modifier = Modifier.fillMaxWidth(), textAlign = TextAlign.Center)
        Spacer(Modifier.height(6.dp))
        Text(
            "Open KDE Connect or GSConnect on your computer and keep both on the same Wi-Fi, hotspot, or Tailscale network.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

fun DeviceType.icon(): ImageVector = when (this) {
    DeviceType.DESKTOP -> Icons.Rounded.Computer
    DeviceType.LAPTOP -> Icons.Rounded.Laptop
    DeviceType.PHONE -> Icons.Rounded.PhoneAndroid
    DeviceType.TABLET -> Icons.Rounded.Tablet
    DeviceType.TV -> Icons.Rounded.Tv
}

fun transportLabel(state: DeviceState): String = when {
    TransportKind.USB in state.transports -> "Connected via USB"
    TransportKind.LAN in state.transports -> "Connected via Wi-Fi"
    TransportKind.BLUETOOTH in state.transports -> "Connected via Bluetooth"
    else -> "Not reachable"
}
