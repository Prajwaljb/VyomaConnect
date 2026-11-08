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
import androidx.compose.material.icons.rounded.Bluetooth
import androidx.compose.material.icons.rounded.Usb
import androidx.compose.material.icons.rounded.Sensors
import androidx.compose.material.icons.rounded.Terminal
import android.widget.Toast
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.BatteryStd
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.LinkOff
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
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PcSpecs
import dev.vyoma.connect.data.PcSpecsStore
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Layers
import androidx.compose.material.icons.rounded.DeveloperBoard
import androidx.compose.material.icons.rounded.Gradient
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material.icons.rounded.Monitor
import kotlin.math.roundToInt
import dev.vyoma.connect.protocol.plugin.builtin.PcPower
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
    val companion = devices.firstOrNull { it.identity.featuresOf == deviceId && it.isPaired && it.isReachable }
    val companionOnline = companion != null
    val companionPlugin = remember(companion?.identity?.deviceId, companion?.plugins) {
        companion?.let { vm.device(it.identity.deviceId)?.plugin<CompanionPlugin>(CompanionPlugin.ID) }
    }
    val pcStats by (companionPlugin?.stats ?: remember { MutableStateFlow(null) }).collectAsState()
    val fingerprint = remember(state?.isReachable, state?.isPaired) { vm.fingerprint(deviceId) }
    val specs = PcSpecsStore.all.collectAsStateWithLifecycle().value[deviceId]

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

        // Commands are run from Home; here they're one row (a wall of shell lines led the page before).
        if (live) {
            item(key = "commands") {
                SectionCard(Modifier.animateItem()) {
                    ListRow(
                        title = "Ping",
                        subtitle = "Shows a notification on the PC",
                        icon = Icons.Rounded.Sensors,
                        iconTint = SystemColor.CYAN.color,
                        showDivider = true,
                        onClick = { vm.ping(deviceId); status.show("Pinged: look for it on the PC") },
                    )
                    ListRow(
                        title = "Commands",
                        subtitle = if (commands.isEmpty()) (if (companionOnline) "Scripts: set up on the PC" else "Shutdown, sleep, scripts: set up on the PC") else "Run or edit what the PC offers",
                        icon = Icons.Rounded.Terminal,
                        iconTint = SystemColor.GRAY.color,
                        trailing = { RowChevron(commands.size.takeIf { it > 0 }?.toString()) },
                        onClick = {
                            if (commands.isEmpty()) {
                                vm.openCommandSetup(deviceId)
                                status.show("Opening Commands on the PC", icon = Icons.Rounded.Terminal)
                            } else showCommands = true
                        },
                    )
                }
            }
        }

        // What the PC is, from its companion (kept, so it shows while the PC is away too).
        if (specs != null && state.isPaired) {
            item(key = "specs-label") { SectionLabel("About this PC", Modifier.animateItem()) }
            item(key = "specs") { SpecsCard(specs, Modifier.animateItem()) }
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

/** One row per part, each named and then said in full (a details page: nothing cut short). */
@Composable
private fun SpecsCard(s: PcSpecs, modifier: Modifier = Modifier) {
    val rows = buildList {
        s.model?.let { add(Triple("Model", it, (if (s.displays.any { d -> d.internal }) Icons.Rounded.Laptop else Icons.Rounded.Computer) to SystemColor.BLUE)) }
        // One line each: the system, the desktop it runs, its kernel.
        val system = listOfNotNull(s.os, s.desktop?.let { d -> s.session?.let { "$d ($it)" } ?: d }, s.kernel?.let { "Linux $it" })
        if (system.isNotEmpty()) add(Triple("System", system.joinToString("\n"), Icons.Rounded.Layers to SystemColor.TEAL))
        s.cpu?.let { cpu ->
            val count = when {
                s.cores != null && s.threads != null && s.threads != s.cores -> "${s.cores} cores, ${s.threads} threads"
                s.cores != null -> "${s.cores} cores"
                s.threads != null -> "${s.threads} threads"
                else -> null
            }
            add(Triple("Processor", listOfNotNull(cpu, count).joinToString("\n"), Icons.Rounded.DeveloperBoard to SystemColor.ORANGE))
        }
        if (s.gpus.isNotEmpty()) add(Triple("Graphics", s.gpus.joinToString("\n"), Icons.Rounded.Gradient to SystemColor.PINK))
        s.memTotal?.let { add(Triple("Memory", memorySize(it), Icons.Rounded.Memory to SystemColor.GREEN)) }
        s.diskTotal?.let { total ->
            add(Triple("Storage", s.diskFree?.let { "${diskSize(it)} free of ${diskSize(total)}" } ?: diskSize(total), Icons.Rounded.Storage to SystemColor.GRAY))
        }
        if (s.displays.isNotEmpty()) {
            val many = s.displays.size > 1
            val text = s.displays.joinToString("\n") { d ->
                val size = "${d.width} × ${d.height}"
                if (!many) size else if (d.internal) "Built-in: $size" else "External: $size"
            }
            add(Triple(if (many) "Displays" else "Display", text, Icons.Rounded.Monitor to SystemColor.CYAN))
        }
    }
    SectionCard(modifier) {
        rows.forEachIndexed { i, (title, value, look) ->
            ListRow(title = title, subtitle = value, icon = look.first, iconTint = look.second.color, showDivider = i < rows.lastIndex, haptic = null)
        }
    }
}

/**
 * RAM as it's sold: the kernel and firmware keep some back, so 15.4 GiB on the meter is a 16 GB PC
 * and 30.6 GiB a 32 GB one. The next size RAM comes in.
 */
internal fun memorySize(bytes: Long): String {
    val gib = bytes / (1024.0 * 1024 * 1024)
    if (gib < 1) return "${bytes / (1024 * 1024)} MB"
    val sold = RAM_SIZES.firstOrNull { it >= gib } ?: kotlin.math.ceil(gib).toInt()
    return "$sold GB"
}

private val RAM_SIZES = listOf(1, 2, 3, 4, 6, 8, 12, 16, 18, 24, 32, 36, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024)

/** Disks are sold in thousands: 508 GB, 2 TB. */
internal fun diskSize(bytes: Long): String {
    val gb = bytes / 1e9
    return when {
        gb >= 1000 -> "%.1f TB".format(java.util.Locale.ROOT, gb / 1000).replace(".0 ", " ")
        gb >= 10 -> "${gb.roundToInt()} GB"
        else -> "%.1f GB".format(java.util.Locale.ROOT, gb)
    }
}
