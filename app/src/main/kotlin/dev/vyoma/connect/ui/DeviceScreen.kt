package dev.vyoma.connect.ui

import android.widget.Toast
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.BatteryStd
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.LinkOff
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Mouse
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ConnectionOrb
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.OrbState
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.protocol.device.PairState
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
) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val state = devices.firstOrNull { it.identity.deviceId == deviceId }
    val context = LocalContext.current
    var confirmUnpair by rememberSaveable { mutableStateOf(false) }
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
        actions = {
            if (state != null) {
                ConnectionOrb(
                    when {
                        state.pairState == PairState.REQUESTED_BY_PEER || state.pairState == PairState.REQUESTED_BY_US -> OrbState.PENDING
                        state.isReachable -> OrbState.CONNECTED
                        else -> OrbState.OFFLINE
                    },
                )
            }
        },
    ) {
        if (state == null) return@CollapsingHeaderScaffold
        val live = state.isPaired && state.isReachable

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
                    modifier = Modifier.animateItem(),
                )
            }
        }

        if (live) {
            item(key = "actions") {
                SectionCard(Modifier.animateItem(), contentPadding = PaddingValues(14.dp)) {
                    ActionGrid(
                        listOf(
                            Triple(Icons.Rounded.UploadFile, "Send files") { vm.pickFiles(context) },
                            Triple(Icons.Rounded.ContentPaste, "Clipboard") { onClipboard() },
                            Triple(Icons.Rounded.Mouse, "Touchpad") { onTouchpad(false) },
                            Triple(Icons.Rounded.Keyboard, "Keyboard") { onTouchpad(true) },
                            Triple(Icons.AutoMirrored.Rounded.VolumeUp, "Ring PC") { vm.ringPc(deviceId); toast(context, "Ringing ${state.identity.deviceName}") },
                            Triple(Icons.Rounded.Lock, if (locked == true) "Locked" else "Lock") { vm.lockPc(deviceId) },
                            Triple(Icons.Rounded.NotificationsActive, "Ping") { vm.ping(deviceId) },
                        ),
                    )
                }
            }
        }

        if (live && sinks.isNotEmpty()) {
            item(key = "volume-label") { SectionLabel("PC sound", Modifier.animateItem()) }
            item(key = "volume") {
                VolumeCard(
                    sinks,
                    onVolume = { s, v -> vm.setSinkVolume(deviceId, s.name, v) },
                    onMute = { s -> vm.setSinkMuted(deviceId, s.name, !s.muted) },
                    onDefault = { s -> vm.makeSinkDefault(deviceId, s.name) },
                )
            }
        }

        if (live && commands.isNotEmpty()) {
            item(key = "commands-label") { SectionLabel("Commands", Modifier.animateItem()) }
            item(key = "commands") { CommandsCard(commands) { vm.runCommand(deviceId, it.key); toast(context, "Running ${it.name}") } }
        }

        if (!state.isPaired && state.isReachable) {
            item(key = "pair") {
                SectionCard {
                    ListRow(
                        title = "Not paired",
                        subtitle = "Pair to share clipboard, files, media and more",
                        icon = Icons.Rounded.Wifi,
                        trailing = { FilledTonalButton(onClick = { vm.requestPair(deviceId) }) { Text("Pair") } },
                    )
                }
            }
        }

        if (battery != null) {
            item(key = "battery") {
                SectionCard(Modifier.animateItem()) {
                    ListRow(
                        title = "Battery ${battery!!.charge}%",
                        subtitle = if (battery!!.charging) "Charging" else if (battery!!.low) "Low" else "On battery",
                        icon = if (battery!!.charging) Icons.Rounded.BatteryChargingFull else Icons.Rounded.BatteryStd,
                        iconTint = if (battery!!.low) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
                    )
                }
            }
        }

        item(key = "security-label") { SectionLabel("Security") }
        item(key = "security") {
            SectionCard {
                ListRow(
                    title = "Certificate fingerprint",
                    subtitle = fingerprint?.chunked(24)?.joinToString("\n") ?: "Unavailable",
                    icon = Icons.Rounded.Fingerprint,
                    showDivider = state.isPaired,
                )
                if (state.isPaired) {
                    ListRow(
                        title = "Unpair",
                        subtitle = "Forget this device and its key",
                        icon = Icons.Rounded.LinkOff,
                        iconTint = MaterialTheme.colorScheme.error,
                        onClick = { confirmUnpair = true },
                    )
                }
            }
        }
        if (live && commands.isEmpty()) {
            item(key = "commands-hint") {
                TextButton(onClick = { vm.openCommandSetup(deviceId) }, modifier = Modifier.padding(horizontal = 8.dp)) {
                    Text("Add PC commands (opens settings on the PC)")
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
