package dev.vyoma.connect.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import dev.vyoma.connect.protocol.plugin.builtin.SystemVolumePlugin
import kotlinx.coroutines.flow.MutableStateFlow

@Composable
fun DeviceScreen(
    vm: AppViewModel,
    deviceId: String,
    onBack: () -> Unit,
    onTouchpad: (keyboard: Boolean) -> Unit,
    onClipboard: () -> Unit,
    onBrowseFiles: () -> Unit,
) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val state = devices.firstOrNull { it.identity.deviceId == deviceId }
    val context = LocalContext.current
    var confirmUnpair by rememberSaveable { mutableStateOf(false) }
    val companionOnline = devices.any { it.identity.companionOf == deviceId && it.isPaired && it.isReachable }
    val fingerprint = remember(state?.isReachable, state?.isPaired) { vm.fingerprint(deviceId) }

    // Plugin instances change when the link reconnects; key on the loaded set.
    val device = remember(state?.plugins) { vm.device(deviceId) }
    val battery by (device?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote ?: remember { MutableStateFlow(null) }).collectAsState()
    val sinks by (device?.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.sinks ?: remember { MutableStateFlow(emptyList()) }).collectAsState()
    val commands by (device?.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.commands ?: remember { MutableStateFlow(emptyList()) }).collectAsState()
    val locked by (device?.plugin<LockPlugin>(LockPlugin.ID)?.locked ?: remember { MutableStateFlow(null) }).collectAsState()
    val nowPlaying by vm.nowPlaying.collectAsStateWithLifecycle()

    val subtitle = state?.let {
        listOfNotNull(
            transportLabel(it),
            battery?.let { b -> "${b.charge}%${if (b.charging) " · charging" else ""}" },
        ).joinToString(" · ")
    }

    CollapsingHeaderScaffold(
        title = state?.identity?.deviceName ?: "Device",
        subtitle = subtitle,
        navigation = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
        hero = state?.let { { DeviceGlyph(it) } },
    ) {
        if (state == null) return@CollapsingHeaderScaffold
        val live = state.isPaired && state.isReachable

        if (live) {
            item(key = "actions") {
                Staggered(0, Modifier.animateItem()) {
                    PcDashboard(
                        vm, state, DashboardSection.CONTROLS,
                        onOpenDevice = {}, onTouchpad = { onTouchpad(true) }, onClipboard = onClipboard, inDetails = true, onBrowseFiles = onBrowseFiles,
                    )
                }
            }
        }

        val np = nowPlaying?.takeIf { it.deviceId == deviceId && it.player.hasMedia }
        if (live && np != null) {
            item(key = "media") {
                MediaCard(
                    np = np,
                    onPlayPause = vm::playPause,
                    onNext = vm::next,
                    onPrevious = vm::previous,
                    onSeek = vm::seekTo,
                    onVolume = vm::setPlayerVolume,
                    onPickPlayer = vm::pickPlayer,
                    showVolume = false,
                    modifier = Modifier.animateItem(),
                    onContinueHere = if (companionOnline) { { vm.continueOnPhone(deviceId) } } else null,
                )
            }
        }

        if (live && sinks.isNotEmpty()) {
            item(key = "volume-label") { SectionLabel("PC sound", Modifier.animateItem()) }
            item(key = "volume") {
                val main = sinks.firstOrNull { it.isDefault } ?: sinks.first()
                Column(Modifier.animateItem(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    VolumeBar(main, onVolume = { vm.setSinkVolume(deviceId, main.name, it) })
                    if (sinks.size > 1) OutputPicker(sinks, onDefault = { vm.makeSinkDefault(deviceId, it.name) })
                }
            }
        }

        if (live) {
            item(key = "commands-label") { SectionLabel("Commands", Modifier.animateItem()) }
            item(key = "commands") {
                if (commands.isNotEmpty()) {
                    CommandsCard(commands) { vm.runCommand(deviceId, it.key); toast(context, "Running ${it.name}") }
                } else {
                    SectionCard(Modifier.animateItem()) {
                        ListRow(
                            title = "Add PC commands",
                            subtitle = "Shutdown, sleep, scripts… set them up on the PC and run them from here",
                            icon = Icons.Rounded.Terminal,
                            iconTint = SystemColor.GRAY.color,
                            onClick = { vm.openCommandSetup(deviceId) },
                        )
                    }
                }
            }
        }

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
                    title = if (state.isReachable) transportLabel(state) else "Not reachable",
                    subtitle = if (state.isReachable) "${state.plugins.size} features active" else "Open KDE Connect on it, or join the same network",
                    icon = if (TransportKind.USB in state.transports) Icons.Rounded.Usb else Icons.Rounded.Wifi,
                    iconTint = if (state.isReachable) SystemColor.BLUE.color else SystemColor.GRAY.color,
                    showDivider = battery != null || companionOnline,
                    // Checks the link end to end: the PC shows a "Ping!" notification.
                    trailing = if (live) { { PillButton("Ping", prominent = false, onClick = { vm.ping(deviceId) }) } } else null,
                )
                // With the companion, the camera has Find PC's slot in the controls.
                if (companionOnline) {
                    ListRow(
                        title = "Find this PC",
                        subtitle = "Rings it so you can find it",
                        icon = Icons.Rounded.NotificationsActive,
                        iconTint = SystemColor.YELLOW.color,
                        showDivider = battery != null,
                        trailing = { PillButton("Ring", prominent = false, onClick = { vm.ringPc(deviceId) }) },
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

        item(key = "security-label") { SectionLabel("Security", Modifier.animateItem()) }
        item(key = "security") {
            SectionCard {
                ListRow(
                    title = "Certificate fingerprint",
                    subtitle = fingerprint?.chunked(24)?.joinToString("\n") ?: "Unavailable",
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

    if (confirmUnpair) {
        AlertDialog(
            onDismissRequest = { confirmUnpair = false },
            title = { Text("Unpair ${state?.identity?.deviceName}?") },
            text = { Text("You'll need to pair again and compare codes to reconnect.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmUnpair = false
                    vm.unpair(deviceId)
                    onBack()
                }) { Text("Unpair", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmUnpair = false }) { Text("Cancel") } },
        )
    }
}

private fun toast(context: android.content.Context, text: String) = Toast.makeText(context, text, Toast.LENGTH_SHORT).show()
