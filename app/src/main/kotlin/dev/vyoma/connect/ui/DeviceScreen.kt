package dev.vyoma.connect.ui

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.LinkOff
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ConnectionOrb
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.OrbState
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.protocol.device.PairState

@Composable
fun DeviceScreen(vm: AppViewModel, deviceId: String, onBack: () -> Unit) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val state = devices.firstOrNull { it.identity.deviceId == deviceId }
    var confirmUnpair by rememberSaveable { mutableStateOf(false) }
    val fingerprint = remember(state?.isReachable, state?.isPaired) { vm.fingerprint(deviceId) }

    CollapsingHeaderScaffold(
        title = state?.identity?.deviceName ?: "Device",
        subtitle = state?.let(::transportLabel),
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

        if (state.isPaired && state.isReachable) {
            item(key = "actions") {
                SectionCard {
                    ListRow(
                        title = "Ping",
                        subtitle = "Send a quick notification",
                        icon = Icons.Rounded.NotificationsActive,
                        onClick = { vm.ping(deviceId) },
                    )
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
                        trailing = { FilledTonalButton(onClick = { vm.requestPair(deviceId) }) { Text("Pair") } },
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
