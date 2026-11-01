package dev.vyoma.connect.ui

import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.RowChevron

import dev.vyoma.connect.designsystem.DialogButton
import dev.vyoma.connect.designsystem.VyomaMotion
import androidx.compose.animation.animateContentSize
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.GlassDialog
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.SystemColor
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import dev.vyoma.connect.protocol.link.TransportKind
import androidx.compose.material.icons.rounded.Usb
import androidx.compose.material.icons.rounded.Terminal
import android.widget.Toast
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.BatteryStd
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.LinkOff
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import dev.vyoma.connect.protocol.plugin.builtin.RunCommandPlugin
import kotlinx.coroutines.flow.MutableStateFlow

@Composable
fun DeviceScreen(
    vm: AppViewModel,
    deviceId: String,
    onBack: () -> Unit,
) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val state = devices.firstOrNull { it.identity.deviceId == deviceId }
    val context = LocalContext.current
    var confirmUnpair by rememberSaveable { mutableStateOf(false) }
    var showCommands by rememberSaveable { mutableStateOf(false) }
    val status = LocalStatus.current
    val companionOnline = devices.any { it.identity.companionOf == deviceId && it.isPaired && it.isReachable }
    val fingerprint = remember(state?.isReachable, state?.isPaired) { vm.fingerprint(deviceId) }

    // Plugin instances change when the link reconnects; key on the loaded set.
    val device = remember(state?.plugins) { vm.device(deviceId) }
    val battery by (device?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote ?: remember { MutableStateFlow(null) }).collectAsState()
    val commands by (device?.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.commands ?: remember { MutableStateFlow(emptyList()) }).collectAsState()

    // How it's connected; the battery row below owns the charge (each fact said once).
    val subtitle = state?.let { transportLabel(it) }

    CollapsingHeaderScaffold(
        title = state?.identity?.deviceName ?: "Device",
        subtitle = subtitle,
        navigation = { BounceIconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
        hero = state?.let { { DeviceGlyph(it) } },
    ) {
        if (state == null) return@CollapsingHeaderScaffold
        val live = state.isPaired && state.isReachable

        if (!state.isPaired && state.isReachable) {
            item(key = "pair") {
                SectionCard {
                    ListRow(
                        title = "Not paired",
                        subtitle = "Pair to share clipboard, files, media and more",
                        icon = Icons.Rounded.Wifi,
                        iconTint = SystemColor.BLUE.color,
                        trailing = { PillButton("Pair", onClick = { vm.requestPair(deviceId) }) },
                    )
                }
            }
        }

        item(key = "info-label") { SectionLabel("Connection", Modifier.animateItem()) }
        item(key = "info") {
            SectionCard(Modifier.animateItem()) {
                ListRow(
                    title = if (state.isReachable) "Link" else "Not reachable",
                    subtitle = if (state.isReachable) "${state.plugins.size} features active" else "Open KDE Connect on it, or join the same network",
                    icon = if (TransportKind.USB in state.transports) Icons.Rounded.Usb else Icons.Rounded.Wifi,
                    iconTint = if (state.isReachable) SystemColor.BLUE.color else SystemColor.GRAY.color,
                    showDivider = battery != null || companionOnline,
                    // Checks the link end to end: the PC shows a "Ping!" notification.
                    trailing = if (live) { { PillButton("Ping", prominent = false, onClick = { vm.ping(deviceId); status.show("Pinged: look for it on the PC") }) } } else null,
                )
                // With the companion, the camera has Find PC's slot in the controls.
                if (companionOnline) {
                    ListRow(
                        title = "Find this PC",
                        subtitle = "Rings it so you can find it",
                        icon = Icons.Rounded.NotificationsActive,
                        iconTint = SystemColor.YELLOW.color,
                        showDivider = battery != null,
                        trailing = { PillButton("Ring", prominent = false, onClick = { vm.ringPc(deviceId); status.show("Ringing the PC", icon = Icons.Rounded.NotificationsActive) }) },
                    )
                }
                battery?.let { b ->
                    ListRow(
                        title = "Battery ${b.charge}%",
                        subtitle = if (b.charging) "Charging" else if (b.low) "Low" else "On battery",
                        icon = if (b.charging) Icons.Rounded.BatteryChargingFull else Icons.Rounded.BatteryStd,
                        iconTint = if (b.low) SystemColor.RED.color else SystemColor.GREEN.color,
                    )
                }
            }
        }

        // Commands are run from Home; here they're one row (a wall of shell lines led the page before).
        if (live) {
            item(key = "commands") {
                SectionCard(Modifier.animateItem()) {
                    ListRow(
                        title = "Commands",
                        subtitle = if (commands.isEmpty()) "Shutdown, sleep, scripts: set up on the PC" else "Run or edit what the PC offers",
                        icon = Icons.Rounded.Terminal,
                        iconTint = SystemColor.GRAY.color,
                        trailing = { RowChevron(commands.size.takeIf { it > 0 }?.toString()) },
                        onClick = { if (commands.isEmpty()) vm.openCommandSetup(deviceId) else showCommands = true },
                    )
                }
            }
        }

        item(key = "security-label") { SectionLabel("Security", Modifier.animateItem()) }
        item(key = "security") {
            SectionCard {
                // One line until asked for: the full fingerprint is for comparing with the PC's.
                var fullPrint by rememberSaveable { mutableStateOf(false) }
                ListRow(
                    modifier = Modifier.animateContentSize(VyomaMotion.spatial()),
                    title = "Certificate fingerprint",
                    subtitle = when {
                        fingerprint == null -> "Unavailable"
                        fullPrint -> fingerprint.chunked(24).joinToString("\n")
                        else -> fingerprint.take(11) + "…" + fingerprint.takeLast(8)
                    },
                    onClick = if (fingerprint != null) { { fullPrint = !fullPrint } } else null,
                    // Monospace so the hex pairs line up in columns, easy to compare with the PC's.
                    subtitleStyle = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                    icon = Icons.Rounded.Fingerprint,
                    iconTint = SystemColor.TEAL.color,
                    showDivider = state.isPaired,
                )
                if (state.isPaired) {
                    ListRow(
                        title = "Unpair",
                        subtitle = "Forget this device and its key",
                        icon = Icons.Rounded.LinkOff,
                        iconTint = SystemColor.RED.color,
                        onClick = { confirmUnpair = true },
                    )
                }
            }
        }
    }

    if (showCommands) CommandsSheet(vm, deviceId) { showCommands = false }
    if (confirmUnpair) {
        GlassDialog(
            onDismissRequest = { confirmUnpair = false },
            title = { Text("Unpair ${state?.identity?.deviceName}?") },
            text = { Text("You'll need to pair again and compare codes to reconnect.") },
            confirmButton = {
                DialogButton(destructive = true, onClick = {
                    confirmUnpair = false
                    vm.unpair(deviceId)
                    onBack()
                }) { Text("Unpair") }
            },
            dismissButton = { DialogButton(onClick = { confirmUnpair = false }) { Text("Cancel") } },
        )
    }
}
