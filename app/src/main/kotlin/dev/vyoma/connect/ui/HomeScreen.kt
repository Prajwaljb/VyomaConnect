package dev.vyoma.connect.ui

import dev.vyoma.connect.service.LastSeen
import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.SmoothCornerShape
import androidx.compose.material.icons.rounded.Terminal
import kotlinx.coroutines.flow.MutableStateFlow
import dev.vyoma.connect.protocol.plugin.builtin.RunCommandPlugin
import dev.vyoma.connect.protocol.plugin.builtin.RemoteCommand
import dev.vyoma.connect.protocol.plugin.builtin.CaptureState
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.graphics.graphicsLayer
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.perform
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import dev.vyoma.connect.designsystem.GlassDialog
import dev.vyoma.connect.designsystem.DialogButton
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.rounded.AutoAwesome
import androidx.compose.material.icons.rounded.Lan
import androidx.compose.material.icons.rounded.Usb

import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.path
import dev.vyoma.connect.designsystem.fadingHorizontalScroll
import androidx.compose.material.icons.rounded.Info
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.glassRaised
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.Glass
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
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.ui.platform.LocalContext
import dev.vyoma.connect.platform.AppLock
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Tablet
import androidx.compose.material.icons.rounded.Tv
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.flow.flowOf
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ConnectionOrb
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.OrbState
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.pressGroup
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.link.TransportKind

@Composable
fun HomeScreen(
    vm: AppViewModel,
    onOpenDevice: (String) -> Unit,
    onTouchpad: (String) -> Unit,
    onClipboard: () -> Unit,
    onBrowseFiles: (String) -> Unit,
    onFiles: () -> Unit = {},
    onCaptures: () -> Unit = {},
    onPresenter: (String) -> Unit = {},
) {
    if (FirstRun.assembling) LaunchedEffect(Unit) {
        kotlinx.coroutines.delay(3000)
        FirstRun.assembling = false
    }
    val allDevices by vm.devices.collectAsStateWithLifecycle()
    // Companions are part of their PC, never a separate computer.
    val devices = allDevices.filter { it.identity.companionOf == null }
    val companionOnline = allDevices.any { it.identity.featuresOf != null && it.isPaired && it.isReachable }
    val nowPlaying by vm.nowPlaying.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val status = LocalStatus.current
    val connected = devices.filter { it.isPaired && it.isReachable }
    val pairing = devices.filter { it.pairState == PairState.REQUESTED_BY_PEER || it.pairState == PairState.REQUESTED_BY_US }
    val available = devices.filter { !it.isPaired && it.isReachable && it !in pairing }
    val offline = devices.filter { it.isPaired && !it.isReachable }

    var selected by rememberSaveable { mutableStateOf<String?>(null) }
    // A paired PC that's away keeps its Home (dimmed, last known state), so the commonest real state,
    // a sleeping laptop, isn't a different screen.
    val primary = connected.firstOrNull { it.identity.deviceId == selected } ?: connected.firstOrNull() ?: offline.firstOrNull()
    val live = primary?.isReachable == true
    val commands by remember(primary?.identity?.deviceId, primary?.plugins) {
        primary?.let { vm.device(it.identity.deviceId)?.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.commands } ?: MutableStateFlow(emptyList())
    }.collectAsStateWithLifecycle()
    val companionPlugin = remember(companionOnline, primary?.identity?.deviceId, allDevices) { primary?.let { vm.companionOf(it.identity.deviceId) } }
    val shot by (companionPlugin?.screenshot?.state ?: remember { MutableStateFlow(CaptureState.Idle) }).collectAsStateWithLifecycle()
    val photo by (companionPlugin?.photo?.state ?: remember { MutableStateFlow(CaptureState.Idle) }).collectAsStateWithLifecycle()
    RefreshCaptures((shot as? CaptureState.Done)?.at to (photo as? CaptureState.Done)?.at)
    val seen = remember(primary?.identity?.deviceId, live) { primary?.takeIf { !live }?.let { LastSeen.of(context, it.identity.deviceId) } }

    CollapsingHeaderScaffold(
        title = primary?.identity?.deviceName ?: "Vyoma",
        subtitle = when {
            primary == null -> "Looking for devices nearby"
            live -> transportLabel(primary)
            seen != null -> "Away · seen ${ago(seen).toString().replaceFirstChar { it.lowercase() }}"
            else -> "Away"
        },
        // The PC's name is the hero: a glyph over it only restated it every day. Its details are the
        // header button.
        actions = {
            if (primary != null) {
                BounceIconButton(onClick = { onOpenDevice(primary.identity.deviceId) }) { Icon(InfoGlyph, "PC details") }
            } else {
                BounceIconButton(onClick = vm::refresh) { Icon(Icons.Rounded.Refresh, "Refresh") }
            }
        },
    ) {
        if (primary != null) {
            val id = primary.identity.deviceId
            if (connected.size > 1) {
                item(key = "switcher") {
                    Row(Modifier.animateItem().pressGroup().fadingHorizontalScroll(rememberScrollState(), bleed = 16.dp).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        connected.forEach { d ->
                            val on = d.identity.deviceId == id
                            Text(
                                d.identity.deviceName,
                                style = MaterialTheme.typography.labelLarge,
                                color = if (on) Color.White else MaterialTheme.colorScheme.onSurface,
                                // Press first so the glass itself squishes, like every other chip.
                                modifier = Modifier.pressable(haptic = Haptic.Select, glass = true) { selected = d.identity.deviceId }
                                    .glassRaised(CircleShape, if (on) Glass.accent else null)
                                    .padding(horizontal = 14.dp, vertical = 8.dp),
                            )
                        }
                    }
                }
            }
            if (!live) item(key = "away") { AwayCard(vm, Modifier.animateItem()) }
            if (live && companionOnline.not() && vm.companionPromo()) {
                item(key = "promo") { CompanionPromoCard(vm, primary?.identity?.deviceId, onDismiss = vm::dismissCompanionPromo, modifier = Modifier.animateItem()) }
            }
            item(key = "status") {
                Staggered(0, Modifier.animateItem()) {
                    PcDashboard(vm, primary, DashboardSection.STATUS, { onTouchpad(id) }, onClipboard)
                }
            }
            // What's playing, then the sound (order by frequency: state, what you adjust, actions).
            val np = nowPlaying?.takeIf { it.deviceId == id && it.player.hasMedia }
            if (live && np != null) item(key = "media") {
                // Only when the companion says this exact video can continue here (a web page,
                // not mpv playing a local file).
                val companionUp = allDevices.any { it.identity.featuresOf == id && it.isPaired && it.isReachable }
                val stats by remember(companionUp, id) { vm.companionOf(id)?.stats ?: flowOf(null) }.collectAsStateWithLifecycle(null)
                val offered = stats?.handoff?.let { sameVideo(it, np.player.title) } == true
                Staggered(1, Modifier.animateItem()) {
                    MediaCard(
                        np = np,
                        onPlayPause = vm::playPause,
                        onNext = vm::next,
                        onPrevious = vm::previous,
                        onSeek = vm::seekTo,
                        onVolume = vm::setPlayerVolume,
                        onPickPlayer = vm::pickPlayer,
                        showVolume = false,
                        onContinueHere = if (offered) { { vm.continueOnPhone(id) } } else null,
                    )
                }
            }
            // Spacing groups Home (no labels): what the PC is doing, then what you control.
            if (live) item(key = "volume") {
                Staggered(2, Modifier.animateItem().padding(top = 8.dp)) {
                    PcDashboard(vm, primary, DashboardSection.VOLUME, { onTouchpad(id) }, onClipboard)
                }
            }
            // The rest of what you'd adjust on the PC: its screens (each app's sound is the volume card's mixer).
            if (live) companionPlugin?.let { plugin ->
                item(key = "levels") { Staggered(3, Modifier.animateItem()) { LevelsCard(plugin) } }
            }
            item(key = "controls") {
                // Free-floating tiles need air from the card above them.
                Staggered(2, Modifier.animateItem().padding(top = 8.dp)) {
                    PcDashboard(vm, primary, DashboardSection.CONTROLS, { onTouchpad(id) }, onClipboard, onPresenter = { onPresenter(id) }, onBrowseFiles = { onBrowseFiles(id) }, onFiles = onFiles)
                }
            }
            if (live) {
                // The PC's commands, one tap each (the tile opens the full list to edit).
                if (commands.isNotEmpty()) {
                    item(key = "run-label") { SectionLabel("Run on PC", Modifier.animateItem()) }
                    item(key = "run") {
                        val context = LocalContext.current
                        CommandChips(commands, Modifier.animateItem()) { cmd -> runGuarded(context, vm, id, cmd, status) }
                    }
                }
                // What the PC's screen and camera sent: one compact row (the full strip is in Files).
                item(key = "captures") {
                    CapturesRow(
                        onTake = companionPlugin?.let { c ->
                            val context = LocalContext.current
                            { AppLock.ask(context, AppLock.SCREENSHOT) { vm.capture(c.screenshot) } }
                        },
                        onOpen = onCaptures,
                        modifier = Modifier.animateItem().padding(top = 8.dp),
                    )
                }
            }
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
                            iconTint = SystemColor.BLUE.color,
                            showDivider = i < available.lastIndex,
                            trailing = {
                                PillButton("Pair", onClick = { vm.requestPair(state.identity.deviceId) })
                            },
                        )
                    }
                }
            }
        }

        // Other paired PCs that are away (the first one is Home's own).
        val away = offline.filter { it != primary }
        if (away.isNotEmpty()) {
            item(key = "offline-label") { SectionLabel("Paired · not reachable", Modifier.animateItem()) }
            item(key = "offline") {
                SectionCard(Modifier.animateItem()) {
                    away.forEachIndexed { i, state ->
                        ListRow(
                            title = state.identity.deviceName,
                            subtitle = "Open KDE Connect on it, or join the same network",
                            icon = state.identity.deviceType.icon(),
                            iconTint = SystemColor.GRAY.color,
                            showDivider = i < away.lastIndex,
                            trailing = { ConnectionOrb(OrbState.OFFLINE, size = 10.dp) },
                            onClick = { onOpenDevice(state.identity.deviceId) },
                        )
                    }
                }
            }
        }

        if (primary == null && pairing.isEmpty() && available.isEmpty()) {
            item(key = "searching") { SearchingCard(vm, Modifier.animateItem()) }
        }
    }
}

/** Commands as glass capsules in a row that scrolls to the edge; a tap runs one. */
@Composable
private fun CommandChips(commands: List<RemoteCommand>, modifier: Modifier = Modifier, onRun: (RemoteCommand) -> Unit) {
    Row(
        modifier.fillMaxWidth().pressGroup().fadingHorizontalScroll(rememberScrollState(), bleed = 16.dp).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        commands.forEach { cmd ->
            Row(
                Modifier
                    .height(44.dp)
                    .pressable(glass = true) { onRun(cmd) }
                    .glassRaised(CircleShape)
                    .padding(start = 14.dp, end = 18.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Rounded.Terminal, null, tint = SystemColor.GRAY.color, modifier = Modifier.size(20.dp))
                Spacer(Modifier.width(8.dp))
                Text(cmd.name, style = MaterialTheme.typography.labelLarge, maxLines = 1)
            }
        }
    }
}

/** Shown inline (and as a notification) so the code can be compared side by side with the PC. */
@Composable
internal fun PairingCard(state: DeviceState, onAccept: () -> Unit, onReject: () -> Unit, modifier: Modifier = Modifier) {
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
        PairingCode(state.verificationKey)
        AnimatedVisibility(incoming, enter = fadeIn(), exit = fadeOut()) {
            Row(Modifier.fillMaxWidth().padding(top = 16.dp), horizontalArrangement = Arrangement.End) {
                PillButton("Decline", onClick = onReject, prominent = false, haptic = Haptic.Failure)
                Spacer(Modifier.width(8.dp))
                PillButton("Pair", onClick = onAccept, haptic = Haptic.Success)
            }
        }
    }
}

/**
 * The code to compare, filling in one character at a time (40 ms apart, each dropping into place
 * with a key tick), so the eye reads it left to right like the PC's. Dots hold the places until
 * the key arrives.
 */
@Composable
private fun PairingCode(key: String?) {
    val haptics = LocalHapticFeedback.current
    val chars = (key ?: "········").toList()
    val shown = remember(key) { List(chars.size) { Animatable(if (key == null) 1f else 0f) } }
    LaunchedEffect(key) {
        if (key == null) return@LaunchedEffect
        shown.forEachIndexed { i, a ->
            launch {
                delay(40L * i)
                haptics.perform(Haptic.Key)
                a.animateTo(1f, VyomaMotion.release())
            }
        }
    }
    Row(Modifier.fillMaxWidth().semantics(mergeDescendants = true) { contentDescription = key?.let { "Code ${it.toList().joinToString(" ")}" } ?: "Waiting for the code" }, horizontalArrangement = Arrangement.Center) {
        chars.forEachIndexed { i, c ->
            if (i == 4) Spacer(Modifier.width(16.dp))
            Text(
                c.toString(),
                style = MaterialTheme.typography.displayMedium.copy(fontFamily = FontFamily.Monospace),
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.graphicsLayer {
                    val p = shown[i].value
                    alpha = p.coerceIn(0f, 1f)
                    translationY = (1f - p) * -14.dp.toPx()
                },
            )
        }
    }
}

/**
 * Three radar sweeps (nothing loops at rest; a refresh sweeps again), then the three steps that
 * solve almost every "not found", and the ways round discovery (an address, the cable) folded
 * behind one button instead of a dead end.
 */
@Composable
internal fun SearchingCard(vm: AppViewModel, modifier: Modifier = Modifier) {
    var adding by rememberSaveable { mutableStateOf(false) }
    var usb by rememberSaveable { mutableStateOf(false) }
    var others by rememberSaveable { mutableStateOf(false) }
    var round by remember { mutableIntStateOf(0) }
    val color = MaterialTheme.colorScheme.primary
    val wave = remember { Animatable(0f) }
    LaunchedEffect(round) {
        wave.snapTo(0f)
        wave.animateTo(3f, tween(3 * 2400, easing = LinearEasing))
    }
    SectionCard(modifier, contentPadding = PaddingValues(24.dp)) {
        Box(
            Modifier.fillMaxWidth().height(120.dp)
                .semantics { contentDescription = "Search again" }
                .pressable { vm.refresh(); round++ },
            contentAlignment = Alignment.Center,
        ) {
            Canvas(Modifier.size(120.dp)) {
                val w = wave.value
                for (i in 0 until 3) {
                    // Each ring runs its course and stops; at rest only the dot stays.
                    val p = w - i / 3f
                    if (p <= 0f || p >= 3f) continue
                    val f = p % 1f
                    drawCircle(color.copy(alpha = 0.5f * (1f - f)), radius = size.minDimension / 2 * f, style = Stroke(width = 2.dp.toPx()))
                }
                drawCircle(color, radius = 8.dp.toPx())
            }
        }
        Text("Looking for your PC", style = MaterialTheme.typography.titleMedium, modifier = Modifier.fillMaxWidth(), textAlign = TextAlign.Center)
        Spacer(Modifier.height(16.dp))
        listOf(
            "Open KDE Connect on your PC",
            "Put both on the same Wi-Fi",
            "Tap your PC when it shows up here",
        ).forEachIndexed { i, step ->
            Row(Modifier.padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(28.dp).glassRaised(CircleShape), contentAlignment = Alignment.Center) {
                    Text("${i + 1}", style = MaterialTheme.typography.labelLarge)
                }
                Spacer(Modifier.width(12.dp))
                Text(step, style = MaterialTheme.typography.bodyMedium)
            }
        }
        Spacer(Modifier.height(16.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally)) {
            PillButton("Search again", onClick = { vm.refresh(); round++ })
            PillButton("Other ways", onClick = { others = true }, prominent = false)
        }
    }
    if (others) GlassDialog(
        onDismissRequest = { others = false },
        title = { Text("Other ways to connect") },
        text = {
            Column {
                ListRow(
                    title = "Enter its address", subtitle = "For Tailscale, or when Wi-Fi hides it",
                    icon = Icons.Rounded.Lan, iconTint = SystemColor.BLUE.color, horizontalPadding = 0.dp, showDivider = true,
                    onClick = { others = false; adding = true },
                )
                ListRow(
                    title = "Use a USB cable", subtitle = "No network needed",
                    icon = Icons.Rounded.Usb, iconTint = SystemColor.GRAY.color, horizontalPadding = 0.dp,
                    onClick = { others = false; usb = true },
                )
            }
        },
        confirmButton = { DialogButton(onClick = { others = false }) { Text("Close") } },
    )
    if (adding) AddHostDialog(vm) { adding = false }
    if (usb) UsbHelpDialog { usb = false }
}

/**
 * Home's PC is away: what to try, in one place, with one clear action. The rest of Home stays as it
 * was (dimmed last known state), so it's still recognisably this PC's page.
 */
@Composable
private fun AwayCard(vm: AppViewModel, modifier: Modifier = Modifier) {
    var usb by rememberSaveable { mutableStateOf(false) }
    SectionCard(modifier, contentPadding = PaddingValues(20.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            ConnectionOrb(OrbState.OFFLINE, size = 10.dp)
            Spacer(Modifier.width(8.dp))
            Text("Not connected", style = MaterialTheme.typography.titleMedium)
        }
        Spacer(Modifier.height(6.dp))
        Text(
            "Wake the PC and open KDE Connect, on the same Wi-Fi as this phone.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(16.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PillButton("Try again", onClick = vm::refresh)
            PillButton("Use USB", onClick = { usb = true }, prominent = false)
        }
    }
    if (usb) UsbHelpDialog { usb = false }
}

/** Once, after the first pair: what the companion adds. Dismissed for good. */
@Composable
private fun CompanionPromoCard(vm: AppViewModel, pcId: String?, onDismiss: () -> Unit, modifier: Modifier = Modifier) {
    var showing by rememberSaveable { mutableStateOf(false) }
    SectionCard(modifier, contentPadding = PaddingValues(20.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(36.dp).clip(SmoothCornerShape(11.dp)).background(SystemColor.BLUE.color), contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.AutoAwesome, null, tint = Color.White, modifier = Modifier.size(20.dp))
            }
            Spacer(Modifier.width(12.dp))
            Text("Get more from your PC", style = MaterialTheme.typography.titleMedium)
        }
        Spacer(Modifier.height(8.dp))
        Text(
            "The Vyoma companion on your PC adds its live screen, camera, files and load to this page.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(16.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PillButton("Set it up", onClick = { showing = true })
            PillButton("Not now", onClick = onDismiss, prominent = false)
        }
    }
    if (showing) dev.vyoma.connect.designsystem.GlassDialog(
        onDismissRequest = { showing = false },
        title = { Text("The Vyoma companion") },
        text = { Column(verticalArrangement = Arrangement.spacedBy(10.dp)) { CompanionSetup(vm, online = false, pcId) } },
        confirmButton = { dev.vyoma.connect.designsystem.DialogButton(onClick = { showing = false }) { Text("Close") } },
    )
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

/** MPRIS titles of the same video differ in suffixes (" - YouTube") between the two views of it. */
private fun sameVideo(a: String, b: String): Boolean {
    val x = a.trim().lowercase()
    val y = b.trim().lowercase()
    return x.isNotEmpty() && y.isNotEmpty() && (x == y || x.startsWith(y) || y.startsWith(x))
}

/**
 * A bare "i" for the PC details button: the glass circle is already its ring, so Material's
 * filled info circle read as a white disc inside a bubble.
 */
private val InfoGlyph: ImageVector by lazy {
    ImageVector.Builder("Info", 24.dp, 24.dp, 24f, 24f).apply {
        path(fill = SolidColor(Color.Black)) {
            // The dot.
            moveTo(12f, 3.6f)
            arcToRelative(1.9f, 1.9f, 0f, true, true, 0f, 3.8f)
            arcToRelative(1.9f, 1.9f, 0f, true, true, 0f, -3.8f)
            close()
            // The stem with a short foot, rounded like Inter's.
            moveTo(10.2f, 10f)
            horizontalLineTo(12.4f)
            arcToRelative(1.1f, 1.1f, 0f, false, true, 1.1f, 1.1f)
            verticalLineTo(18.6f)
            horizontalLineTo(14.6f)
            arcToRelative(0.9f, 0.9f, 0f, false, true, 0f, 1.8f)
            horizontalLineTo(9.4f)
            arcToRelative(0.9f, 0.9f, 0f, false, true, 0f, -1.8f)
            horizontalLineTo(10.9f)
            verticalLineTo(11.8f)
            horizontalLineTo(10.2f)
            arcToRelative(0.9f, 0.9f, 0f, false, true, 0f, -1.8f)
            close()
        }
    }.build()
}
