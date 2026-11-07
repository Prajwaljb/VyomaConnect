package dev.vyoma.connect.ui

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.collectAsState
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import kotlinx.coroutines.flow.MutableStateFlow
import androidx.compose.material.icons.rounded.Bluetooth
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.RemoveCircleOutline
import androidx.compose.material.icons.rounded.SportsEsports
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.material.icons.rounded.Usb
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.Status
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.isDarkUi
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.plugin.builtin.GamepadRoute
import dev.vyoma.connect.protocol.plugin.builtin.TripReport
import dev.vyoma.connect.service.BluetoothFallback
import kotlinx.coroutines.delay

internal enum class CheckKind(val title: String) {
    WIFI("Wi-Fi"),
    USB("USB cable"),
    BLUETOOTH("Bluetooth"),
    COMPANION("Companion"),
    ENCRYPTION("Encryption"),
    CONTROLLER("Controller"),
    TOUCHPAD("Touchpad"),
}

/** GOOD: working right now. QUIET: nothing wrong, just not in use. PROBLEM: needs something done. */
internal enum class CheckLevel { GOOD, QUIET, PROBLEM }

internal data class ConnectionCheck(val kind: CheckKind, val detail: String, val level: CheckLevel)

/** Everything the checks are worked out from. [companion] is null while it isn't connected. */
internal data class LinkFacts(
    val pc: Set<TransportKind>,
    val paired: Boolean,
    val companion: Set<TransportKind>?,
    val bluetooth: BluetoothFallback.Readiness,
    val controllerRoute: GamepadRoute?,
    val controllerTrip: TripReport?,
    val touchpadOpen: Boolean,
    val touchpadTrip: TripReport?,
)

/** What's really connected, because Android's own Bluetooth page leaves out everything but calls and audio. */
internal fun connectionChecks(f: LinkFacts): List<ConnectionCheck> {
    fun users(kind: TransportKind, route: GamepadRoute) = buildList {
        if (kind in f.pc) add("PC link")
        if (f.companion?.contains(kind) == true) add("companion")
        if (kind == TransportKind.BLUETOOTH && f.touchpadOpen) add("touchpad")
        if (f.controllerRoute == route) add("controller")
    }
    fun inUse(users: List<String>) = "In use: " + users.joinToString(", ")
    fun carried(kind: CheckKind, transport: TransportKind, route: GamepadRoute): ConnectionCheck {
        val users = users(transport, route)
        return if (users.isEmpty()) ConnectionCheck(kind, "Not in use right now", CheckLevel.QUIET)
        else ConnectionCheck(kind, inUse(users), CheckLevel.GOOD)
    }
    fun timed(via: String, trip: TripReport?) =
        if (trip == null) "Over $via, timing it now" else "Over $via, ${trip.medianMs} ms typical, ${trip.worstMs} ms at worst"

    val radioUsers = users(TransportKind.BLUETOOTH, GamepadRoute.BLUETOOTH)
    val radio = when {
        radioUsers.isNotEmpty() -> ConnectionCheck(
            CheckKind.BLUETOOTH,
            inUse(radioUsers) + ". Android's Bluetooth page only counts calls and audio, so it can still say Disconnected.",
            CheckLevel.GOOD,
        )
        else -> when (f.bluetooth) {
            BluetoothFallback.Readiness.READY -> ConnectionCheck(CheckKind.BLUETOOTH, "Ready, nothing using it right now", CheckLevel.QUIET)
            BluetoothFallback.Readiness.RADIO_OFF -> ConnectionCheck(CheckKind.BLUETOOTH, "Turned off on this phone", CheckLevel.QUIET)
            BluetoothFallback.Readiness.NO_ADAPTER -> ConnectionCheck(CheckKind.BLUETOOTH, "This phone has no Bluetooth", CheckLevel.QUIET)
            BluetoothFallback.Readiness.NEEDS_PERMISSION -> ConnectionCheck(CheckKind.BLUETOOTH, "Vyoma needs the Nearby devices permission", CheckLevel.PROBLEM)
            BluetoothFallback.Readiness.PC_UNKNOWN -> ConnectionCheck(CheckKind.BLUETOOTH, "Connect once with the companion on your PC first", CheckLevel.PROBLEM)
            BluetoothFallback.Readiness.NOT_PAIRED -> ConnectionCheck(CheckKind.BLUETOOTH, "Pair this phone with your PC in Bluetooth settings", CheckLevel.PROBLEM)
        }
    }
    val companion = f.companion
    return listOf(
        carried(CheckKind.WIFI, TransportKind.LAN, GamepadRoute.WIFI),
        carried(CheckKind.USB, TransportKind.USB, GamepadRoute.USB),
        radio,
        when {
            companion.isNullOrEmpty() -> ConnectionCheck(CheckKind.COMPANION, "Not connected", CheckLevel.QUIET)
            else -> ConnectionCheck(CheckKind.COMPANION, "Connected over " + transportName(companion), CheckLevel.GOOD)
        },
        when {
            f.pc.isEmpty() -> ConnectionCheck(CheckKind.ENCRYPTION, "Nothing connected to check", CheckLevel.QUIET)
            f.paired -> ConnectionCheck(CheckKind.ENCRYPTION, "Encrypted, and the PC's certificate matches", CheckLevel.GOOD)
            else -> ConnectionCheck(CheckKind.ENCRYPTION, "Encrypted, but this PC isn't paired yet", CheckLevel.PROBLEM)
        },
        when (val route = f.controllerRoute) {
            null -> ConnectionCheck(CheckKind.CONTROLLER, "Not open, or only over the main link", CheckLevel.QUIET)
            else -> ConnectionCheck(CheckKind.CONTROLLER, timed(routeName(route), f.controllerTrip), CheckLevel.GOOD)
        },
        if (f.touchpadOpen) ConnectionCheck(CheckKind.TOUCHPAD, timed("Bluetooth", f.touchpadTrip), CheckLevel.GOOD)
        else ConnectionCheck(CheckKind.TOUCHPAD, "No channel of its own open", CheckLevel.QUIET),
    )
}

private fun transportName(kinds: Set<TransportKind>) = when {
    TransportKind.USB in kinds -> "the USB cable"
    TransportKind.LAN in kinds -> "Wi-Fi"
    else -> "Bluetooth"
}

private fun routeName(route: GamepadRoute) = when (route) {
    GamepadRoute.USB -> "the USB cable"
    GamepadRoute.WIFI -> "Wi-Fi"
    GamepadRoute.BLUETOOTH -> "Bluetooth"
}

/** Android doesn't say when the radio is turned off here, so it's looked at every so often while shown. */
@Composable
internal fun rememberBluetoothReadiness(): BluetoothFallback.Readiness {
    val context = LocalContext.current.applicationContext
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var readiness by remember { mutableStateOf(BluetoothFallback.readiness(context)) }
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                readiness = BluetoothFallback.readiness(context)
                delay(READINESS_POLL_MS)
            }
        }
    }
    return readiness
}

@Composable
internal fun ConnectionChecksCard(checks: List<ConnectionCheck>, modifier: Modifier = Modifier) {
    val dark = isDarkUi
    SectionCard(modifier) {
        checks.forEachIndexed { i, check ->
            ListRow(
                title = check.kind.title,
                subtitle = check.detail,
                icon = check.kind.icon(),
                iconTint = if (check.level == CheckLevel.QUIET) SystemColor.GRAY.color else check.kind.tint(),
                showDivider = i < checks.lastIndex,
                trailing = {
                    when (check.level) {
                        CheckLevel.GOOD -> Icon(Icons.Rounded.CheckCircle, "Working", tint = Status.connected(dark))
                        CheckLevel.QUIET -> Icon(Icons.Rounded.RemoveCircleOutline, "Not in use", tint = Status.offline(dark))
                        CheckLevel.PROBLEM -> Icon(Icons.Rounded.ErrorOutline, "Needs attention", tint = Status.pending(dark))
                    }
                },
            )
        }
    }
}

private fun CheckKind.icon(): ImageVector = when (this) {
    CheckKind.WIFI -> Icons.Rounded.Wifi
    CheckKind.USB -> Icons.Rounded.Usb
    CheckKind.BLUETOOTH -> Icons.Rounded.Bluetooth
    CheckKind.COMPANION -> Icons.Rounded.Computer
    CheckKind.ENCRYPTION -> Icons.Rounded.Lock
    CheckKind.CONTROLLER -> Icons.Rounded.SportsEsports
    CheckKind.TOUCHPAD -> Icons.Rounded.TouchApp
}

@Composable
private fun CheckKind.tint() = when (this) {
    CheckKind.WIFI, CheckKind.USB, CheckKind.BLUETOOTH -> SystemColor.BLUE.color
    CheckKind.COMPANION -> SystemColor.GREEN.color
    CheckKind.ENCRYPTION -> SystemColor.TEAL.color
    CheckKind.CONTROLLER -> SystemColor.ORANGE.color
    CheckKind.TOUCHPAD -> SystemColor.TEAL.color
}

private const val READINESS_POLL_MS = 2_000L

/** What's working and what isn't, for each paired PC: where to look when something's wrong. */
@Composable
internal fun ConnectionChecksScreen(vm: AppViewModel, onBack: () -> Unit) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val pcs = devices.filter { it.isPaired && it.identity.companionOf == null }
    val bluetooth = rememberBluetoothReadiness()
    CollapsingHeaderScaffold(
        title = "Connection",
        subtitle = "What's working right now",
        navigation = { BounceIconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
    ) {
        if (pcs.isEmpty()) item(key = "none") {
            Text(
                "Nothing to check yet: pair with a PC first.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 32.dp),
            )
        }
        pcs.forEach { pc ->
            val id = pc.identity.deviceId
            if (pcs.size > 1) item(key = "label-$id") { SectionLabel(pc.identity.deviceName, Modifier.animateItem()) }
            item(key = "checks-$id") {
                val companion = devices.firstOrNull { it.identity.featuresOf == id && it.isPaired && it.isReachable }
                val plugin = remember(companion?.identity?.deviceId, companion?.plugins) {
                    companion?.let { vm.device(it.identity.deviceId)?.plugin<CompanionPlugin>(CompanionPlugin.ID) }
                }
                val route by (plugin?.gamepadRoute ?: remember { MutableStateFlow(null) }).collectAsState()
                val trip by (plugin?.gamepadTrip ?: remember { MutableStateFlow(null) }).collectAsState()
                val pointerOpen by (plugin?.pointerOpen ?: remember { MutableStateFlow(false) }).collectAsState()
                val pointerTrip by (plugin?.pointerTrip ?: remember { MutableStateFlow(null) }).collectAsState()
                ConnectionChecksCard(
                    connectionChecks(
                        LinkFacts(
                            pc = pc.transports,
                            paired = pc.isPaired,
                            companion = companion?.transports,
                            bluetooth = bluetooth,
                            controllerRoute = route,
                            controllerTrip = trip,
                            touchpadOpen = pointerOpen,
                            touchpadTrip = pointerTrip,
                        ),
                    ),
                    Modifier.animateItem(),
                )
            }
        }
    }
}
