package dev.vyoma.connect.ui

import dev.vyoma.connect.designsystem.fadingVerticalScroll
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material.icons.rounded.TouchApp
import dev.vyoma.connect.designsystem.accent
import androidx.compose.material.icons.rounded.Park
import dev.vyoma.connect.designsystem.LocalStatus
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.text.BasicText
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.graphics.SolidColor
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.fitTogether
import dev.vyoma.connect.designsystem.toned
import dev.vyoma.connect.designsystem.DialogButton
import androidx.compose.foundation.layout.fillMaxWidth
import dev.vyoma.connect.designsystem.RowChevron
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.GlassDialog
import dev.vyoma.connect.designsystem.SegmentedControl
import dev.vyoma.connect.designsystem.VyomaSwitch
import dev.vyoma.connect.designsystem.SystemColor
import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.text.font.FontFamily
import androidx.lifecycle.compose.LifecycleResumeEffect
import dev.vyoma.connect.platform.AppLock
import dev.vyoma.connect.platform.NotificationBridge
import dev.vyoma.connect.platform.PhonePermissions
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.service.LinkService
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material.icons.rounded.Sms
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Lan
import androidx.compose.material.icons.rounded.Usb
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import android.content.Intent
import android.net.Uri
import android.os.PowerManager
import android.provider.Settings
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import dev.vyoma.connect.BuildConfig
import dev.vyoma.connect.demo.DemoDiagnostics
import dev.vyoma.connect.demo.DemoPc
import androidx.compose.material.icons.rounded.BugReport
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.designsystem.Haptics
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Bluetooth
import androidx.compose.material.icons.rounded.Speed
import dev.vyoma.connect.service.BluetoothFallback
import androidx.compose.material.icons.rounded.AutoMode
import androidx.compose.material.icons.rounded.Gradient
import androidx.compose.material.icons.rounded.Wallpaper
import dev.vyoma.connect.designsystem.AppCanvas
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.Palette
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.pressable

@Composable
fun SettingsScreen(vm: AppViewModel, onBack: (() -> Unit)? = null, onAutomation: () -> Unit = {}, onNotifications: () -> Unit = {}) {
    val theme by vm.theme.collectAsStateWithLifecycle()
    val name by vm.deviceName.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val status = LocalStatus.current
    var editingName by rememberSaveable { mutableStateOf(false) }
    var showAdbHelp by rememberSaveable { mutableStateOf(false) }
    var showUsbHelp by rememberSaveable { mutableStateOf(false) }
    var addingHost by rememberSaveable { mutableStateOf(false) }
    val hosts by vm.manualHosts.collectAsStateWithLifecycle()
    // Re-check permission state whenever we come back from system settings.
    var resumeTick by remember { mutableIntStateOf(0) }
    LifecycleResumeEffect(Unit) {
        resumeTick++
        onPauseOrDispose { }
    }
    val power = context.getSystemService(PowerManager::class.java)
    // Read on every resume (nothing read the tick, so it showed only when something else redrew,
    // seconds later), and again a moment after in case the system commits the change late.
    var unrestricted by remember { mutableStateOf(power.isIgnoringBatteryOptimizations(context.packageName)) }
    LaunchedEffect(resumeTick) {
        repeat(4) { i ->
            unrestricted = power.isIgnoringBatteryOptimizations(context.packageName)
            if (unrestricted) return@LaunchedEffect
            kotlinx.coroutines.delay(if (i == 0) 150L else 400L)
        }
    }

    CollapsingHeaderScaffold(
        title = "Settings",
        navigation = onBack?.let { back -> { BounceIconButton(onClick = back) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } } },
    ) {
        item(key = "device-label") { SectionLabel("This phone") }
        item(key = "device") {
            SectionCard {
                ListRow(
                    title = "Device name",
                    subtitle = name,
                    icon = Icons.Rounded.Edit,
                    iconTint = SystemColor.BLUE.color,
                    trailing = { RowChevron() },
                    onClick = { editingName = true },
                )
            }
        }

        item(key = "look-label") { SectionLabel("Appearance") }
        item(key = "look") {
            SectionCard(contentPadding = PaddingValues(vertical = 16.dp)) {
                Text("Accent", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(horizontal = 20.dp))
                Spacer(Modifier.height(12.dp))
                // All seven in view, evenly spread: a sixth sliver at the edge hid one choice.
                BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = 20.dp)) {
                // One size for all seven names (see fitTogether), each allowed a little past its share.
                val label = fitTogether(
                    Palette.entries.map { it.label }, MaterialTheme.typography.labelSmall, maxWidth / Palette.entries.size + SWATCH_BLEED,
                )
                // Each circle leaves a gap in its share (seven at 44 dp touched on a 412 dp phone).
                val swatch = (maxWidth / Palette.entries.size - 8.dp).coerceAtMost(44.dp)
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Palette.entries.forEach { p ->
                        PaletteSwatch(p, selected = theme.palette == p, label, swatch, Modifier.weight(1f)) { vm.setPalette(p) }
                    }
                }
                }
                Spacer(Modifier.height(20.dp))
                SegmentedControl(
                    options = DarkMode.entries.map { it.name.lowercase().replaceFirstChar(Char::uppercase) },
                    selected = theme.darkMode.ordinal,
                    onSelect = { vm.setDarkMode(DarkMode.entries[it]) },
                    modifier = Modifier.padding(horizontal = 20.dp),
                )
            }
        }

        // The look's on/off effects, apart from the choices above (four kinds of control in one card read busy).
        item(key = "effects-label") { SectionLabel("Effects") }
        item(key = "effects") {
            SectionCard {
                val canvasGlow by vm.canvasGlow.collectAsStateWithLifecycle()
                ListRow(
                    title = "Background glow",
                    subtitle = "A soft glow behind each screen",
                    icon = Icons.Rounded.Gradient,
                    iconTint = SystemColor.BLUE.color,
                    showDivider = true,
                    // The whole row toggles (like every other row acts), with the switch's haptic.
                    haptic = if (canvasGlow) Haptic.ToggleOff else Haptic.ToggleOn,
                    onClick = { vm.setCanvasGlow(!canvasGlow) },
                    trailing = { VyomaSwitch(checked = canvasGlow, onCheckedChange = { vm.setCanvasGlow(it) }, contentDescription = "Background glow") },
                )
                val wallpaper by vm.canvasWallpaper.collectAsStateWithLifecycle()
                ListRow(
                    title = "PC wallpaper colours",
                    subtitle = if (AppCanvas.wallpaper == null) "Needs the companion on your PC" else "Glow in your desktop's colours",
                    icon = Icons.Rounded.Wallpaper,
                    iconTint = SystemColor.CYAN.color,
                    showDivider = true,
                    enabled = canvasGlow,
                    haptic = if (wallpaper) Haptic.ToggleOff else Haptic.ToggleOn,
                    onClick = { vm.setCanvasWallpaper(!wallpaper) },
                    trailing = { VyomaSwitch(checked = wallpaper && canvasGlow, onCheckedChange = { vm.setCanvasWallpaper(it) }, contentDescription = "PC wallpaper colours") },
                )
                val glass by vm.liteGlass.collectAsStateWithLifecycle()
                ListRow(
                    title = "Lighter glass",
                    subtitle = when {
                        glass.detected && !glass.chosen -> "Turned on for this phone, so it stays smooth"
                        else -> "Frosted bars for slower phones"
                    },
                    icon = Icons.Rounded.Speed,
                    iconTint = SystemColor.GRAY.color,
                    haptic = if (glass.on) Haptic.ToggleOff else Haptic.ToggleOn,
                    onClick = { vm.setLiteGlass(!glass.on) },
                    trailing = { VyomaSwitch(checked = glass.on, onCheckedChange = { vm.setLiteGlass(it) }, contentDescription = "Lighter glass") },
                    showDivider = false,
                )
            }
        }

        item(key = "auto-label") { SectionLabel("Automation") }
        item(key = "auto") {
            val rules by vm.rules.collectAsStateWithLifecycle()
            val on = rules.count { it.enabled }
            SectionCard {
                ListRow(
                    title = "Rules",
                    subtitle = "When this happens, do that: calls, leaving, bedtime…",
                    icon = Icons.Rounded.AutoMode,
                    iconTint = SystemColor.ORANGE.color,
                    trailing = { RowChevron(if (rules.isEmpty()) null else "$on on") },
                    onClick = onAutomation,
                )
            }
        }

        item(key = "sync-label") { SectionLabel("Sync") }
        item(key = "sync") {
            val notifOn = remember(resumeTick) { NotificationBridge.isEnabled(context) }
            val clipOn = remember(resumeTick) { vm.backgroundClipboardSync() }
            var phoneTick by remember { mutableIntStateOf(0) }
            val phoneOn = remember(resumeTick, phoneTick) { PhonePermissions.allGranted(context) }
            val askPhone = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
                phoneTick++
                if (result.values.any { it }) LinkService.start(context, refresh = true, visible = true)
                // Denied twice: Android stops asking, so send them to the app's permission page.
                if (result.values.any { !it } && !PhonePermissions.allGranted(context)) {
                    runCatching {
                        context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}")))
                    }
                }
            }
            SectionCard {
                ListRow(
                    title = "Notifications on PC",
                    subtitle = if (notifOn) "Choose which apps show, and what" else "Mirror them to your PC",
                    icon = Icons.Rounded.Notifications,
                    iconTint = SystemColor.RED.color,
                    showDivider = true,
                    trailing = { RowChevron(if (notifOn) "On" else "Off") },
                    // Off: straight to Android's switch; on: the per-app choices (access is on that page too).
                    onClick = { if (notifOn) onNotifications() else runCatching { context.startActivity(NotificationBridge.settingsIntent(context)) } },
                )
                ListRow(
                    title = "Calls & messages on PC",
                    subtitle = if (phoneOn) "Texts, calls and contacts on your PC"
                    else "Text and see calls from your PC",
                    icon = Icons.Rounded.Sms,
                    iconTint = SystemColor.GREEN.color,
                    showDivider = true,
                    trailing = { RowChevron(if (phoneOn) "On" else "Off") },
                    onClick = { if (!phoneOn) askPhone.launch(PhonePermissions.all) },
                )
                BluetoothRow(resumeTick)
                SteadierRow(resumeTick)
                ListRow(
                    // Named as on the Clipboard tab: one feature, one name.
                    title = "Sync when closed",
                    subtitle = if (clipOn) "Copies sync even when Vyoma is closed"
                    else "One-time setup from your PC",
                    icon = Icons.Rounded.ContentPaste,
                    iconTint = SystemColor.ORANGE.color,
                    trailing = { RowChevron(if (clipOn) "On" else "Off") },
                    onClick = { if (!clipOn) showAdbHelp = true },
                )
            }
        }

        item(key = "security-label") { SectionLabel("Security") }
        item(key = "security") { SectionCard { FingerprintLockRow(resumeTick) } }

        item(key = "hosts-label") { SectionLabel("Connect by address") }
        item(key = "hosts") {
            SectionCard {
                hosts.forEach { host ->
                    ListRow(
                        title = host,
                        subtitle = if (host.startsWith("100.") || host.endsWith(".ts.net")) "Tailscale" else "Added manually",
                        icon = Icons.Rounded.Lan,
                        iconTint = SystemColor.TEAL.color,
                        showDivider = true,
                        trailing = {
                            BounceIconButton(onClick = { vm.removeManualHost(host) }) { Icon(Icons.Rounded.Close, "Remove $host") }
                        },
                    )
                }
                ListRow(
                    title = "Add a PC by IP or name",
                    subtitle = "For Tailscale, or when it isn't found",
                    icon = Icons.Rounded.Add,
                    iconTint = SystemColor.BLUE.color,
                    showDivider = true,
                    trailing = { RowChevron() },
                    onClick = { addingHost = true },
                )
                ListRow(
                    title = "Over a USB cable",
                    subtitle = "Faster and steadier than Wi-Fi",
                    icon = Icons.Rounded.Usb,
                    iconTint = SystemColor.CYAN.color,
                    trailing = { RowChevron() },
                    onClick = { showUsbHelp = true },
                )
            }
        }

        item(key = "bg-label") { SectionLabel("Staying connected") }
        item(key = "bg") {
            SectionCard {
                ListRow(
                    title = if (unrestricted) "Background activity allowed" else "Allow background activity",
                    subtitle = if (unrestricted) "Vyoma won't be put to sleep" else "Keeps your PC connected",
                    icon = Icons.Rounded.BatteryChargingFull,
                    iconTint = SystemColor.GREEN.color,
                    trailing = { RowChevron(if (unrestricted) "On" else "Off") },
                    showDivider = true,
                    onClick = { if (!unrestricted) requestUnrestricted(context) },
                )
                val outdoor by vm.outdoorMode.collectAsStateWithLifecycle()
                ListRow(
                    title = "Outdoor mode",
                    subtitle = "Off Wi-Fi with no PC connected, Vyoma stops in the background and starts again on Wi-Fi",
                    icon = Icons.Rounded.Park,
                    iconTint = SystemColor.GREEN.color,
                    haptic = if (outdoor) Haptic.ToggleOff else Haptic.ToggleOn,
                    onClick = { vm.setOutdoorMode(!outdoor) },
                    trailing = { VyomaSwitch(checked = outdoor, onCheckedChange = { vm.setOutdoorMode(it) }, contentDescription = "Outdoor mode") },
                )
            }
        }

        if (BuildConfig.DEMO) {
            item(key = "demo-label") { SectionLabel("Demo") }
            item(key = "demo") {
                val scope = rememberCoroutineScope()
                var saving by remember { mutableStateOf(false) }
                SectionCard {
                    ListRow(
                        title = "Simulated laptop",
                        subtitle = "${DemoPc.NAME} is simulated, offline",
                        icon = Icons.Rounded.Laptop,
                        iconTint = SystemColor.BLUE.color,
                        showDivider = true,
                    )
                    ListRow(
                        title = if (saving) "Saving log…" else "Generate log",
                        subtitle = "Saves a diagnostics log to Downloads",
                        icon = Icons.Rounded.BugReport,
                        iconTint = SystemColor.ORANGE.color,
                        trailing = { RowChevron() },
                        onClick = {
                            if (!saving) {
                                saving = true
                                scope.launch {
                                    DemoDiagnostics.writeLog(context).fold(
                                        onSuccess = { status.show("Saved to Downloads: $it") },
                                        onFailure = { status.show("Couldn't save the log: ${it.message}", failure = true) },
                                    )
                                    saving = false
                                }
                            }
                        },
                    )
                }
            }
        }

        item(key = "about") {
            SectionCard {
                ListRow(
                    title = "Vyoma",
                    subtitle = "Version ${context.packageManager.getPackageInfo(context.packageName, 0).versionName} · KDE Connect protocol v8",
                    icon = Icons.Rounded.Info,
                    iconTint = SystemColor.GRAY.color,
                )
            }
        }
    }

    if (showAdbHelp) ClipboardAdbDialog { showAdbHelp = false }
    if (showUsbHelp) UsbHelpDialog { showUsbHelp = false }

    if (addingHost) AddHostDialog(vm) { addingHost = false }

    if (editingName) {
        var draft by rememberSaveable { mutableStateOf(name) }
        GlassDialog(
            onDismissRequest = { editingName = false },
            title = { Text("Device name") },
            text = { OutlinedTextField(draft, { draft = it.take(32) }, singleLine = true) },
            confirmButton = {
                DialogButton(enabled = draft.isNotBlank(), onClick = {
                    vm.setDeviceName(draft.trim())
                    editingName = false
                }) { Text("Save") }
            },
            dismissButton = { DialogButton(onClick = { editingName = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun PaletteSwatch(palette: Palette, selected: Boolean, label: TextStyle, size: androidx.compose.ui.unit.Dp, modifier: Modifier, onClick: () -> Unit) {
    val dark = MaterialTheme.colorScheme.background.red < 0.5f
    val ring by animateColorAsState(if (selected) MaterialTheme.colorScheme.onSurface else Color.Transparent, label = "ring")
    val inset by animateDpAsState(if (selected) 5.dp else 0.dp, VyomaMotion.spatial(), label = "inset")
    // Equal shares (labels differ in width, so spreading the columns spaced the circles unevenly).
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = modifier.pressable(haptic = Haptic.Select, onClick = onClick)) {
        Box(
            Modifier.size(size).clip(CircleShape).border(2.dp, ring, CircleShape).padding(inset.coerceAtLeast(0.dp)), // the spring overshoots below zero
            contentAlignment = Alignment.Center,
        ) {
            val context = androidx.compose.ui.platform.LocalContext.current
            val pcWall = AppCanvas.wallpaper
            val fill = if (palette == Palette.PC && pcWall != null) {
                // The PC option previews its accent over the wallpaper's own colours.
                val a = pcWall.accent(dark) ?: MaterialTheme.colorScheme.primary
                Brush.sweepGradient(listOf(a) + pcWall.colors.take(4).map { Color(0xFF000000.toInt() or it).toned(1.3f) } + a)
            } else if (palette == Palette.PC || palette == Palette.PHONE) {
                // The phone's Material You primary (not the app's: that may be the PC's), with a soft sweep.
                val phone = remember(context, dark) {
                    if (dark) androidx.compose.material3.dynamicDarkColorScheme(context).primary else androidx.compose.material3.dynamicLightColorScheme(context).primary
                }
                Brush.sweepGradient(listOf(phone, Color(0xFF5CE1E6), phone))
            } else {
                // Flat: a dark-to-light gradient made them glossy balls. The deep shade the accent fills
                // with (switches, sliders), not the pastel dark-theme text tone, which read washed out.
                SolidColor(if (dark) palette.dark.toned(1.2f, 0.5f) else palette.light)
            }
            Box(Modifier.size(size).clip(CircleShape).background(fill), contentAlignment = Alignment.Center) {
                if (selected) Icon(Icons.Rounded.Check, null, tint = Color.White, modifier = Modifier.size(18.dp))
            }
        }
        Spacer(Modifier.height(6.dp))
        // One line, allowed a little past its share ("Wallpaper" is a hair wider than a sixth of the
        // card), at the row's shared size: at larger text sizes they ran into each other on a OnePlus.
        BasicText(
            palette.label,
            style = label.copy(color = MaterialTheme.colorScheme.onSurfaceVariant),
            maxLines = 1,
            softWrap = false,
            modifier = Modifier.layout { m, c ->
                val share = c.maxWidth
                val p = m.measure(c.copy(minWidth = 0, maxWidth = share + SWATCH_BLEED.roundToPx()))
                layout(share, p.height) { p.place((share - p.width) / 2, 0) }
            },
        )
    }
}

/** How far a swatch's label may reach past its share, half into each gap. */
private val SWATCH_BLEED = 6.dp

@SuppressLint("BatteryLife") // Core function: the app must stay connected in the background.
internal fun requestUnrestricted(context: android.content.Context) {
    val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${context.packageName}"))
    runCatching { context.startActivity(intent) }
}

/** A PC by IP or host name, for Tailscale or when discovery can't see it. */
@Composable
internal fun AddHostDialog(vm: AppViewModel, onDismiss: () -> Unit) {
    var draft by rememberSaveable { mutableStateOf("") }
    var invalid by rememberSaveable { mutableStateOf(false) }
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add a PC") },
        text = {
            Column {
                Text(
                    "Its IP address (for Tailscale, the 100.x address shown in the Tailscale app) or host name. " +
                        "Vyoma will keep trying it alongside automatic discovery.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    draft,
                    { draft = it; invalid = false },
                    singleLine = true,
                    isError = invalid,
                    placeholder = { Text("192.168.1.20 or my-pc") },
                    supportingText = if (invalid) ({ Text("That doesn't look like an address") }) else null,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                )
            }
        },
        confirmButton = {
            DialogButton(onClick = { if (vm.addManualHost(draft)) onDismiss() else invalid = true }) { Text("Add") }
        },
        dismissButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** How to grant automatic clipboard sync once over ADB (no root). */
@Composable
internal fun ClipboardAdbDialog(onDismiss: () -> Unit) {
    val context = LocalContext.current
    val cmd = "adb shell pm grant ${context.packageName} android.permission.READ_LOGS && " +
        "adb shell appops set ${context.packageName} SYSTEM_ALERT_WINDOW allow"
    GlassDialog(
        onDismissRequest = { onDismiss() },
        title = { Text("Automatic clipboard sync") },
        text = {
            Column {
                Text("Android only lets the app in front read the clipboard. Two permissions lift that, and Android only lets a computer grant them, once.")
                Spacer(Modifier.height(12.dp))
                Text("With the Vyoma companion on your PC: turn on USB debugging, plug in, then allow it when the PC asks. Without it, run this from any computer with adb:")
                Spacer(Modifier.height(8.dp))
                Text(cmd, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace))
                Spacer(Modifier.height(12.dp))
                Text(
                    "Then reopen Vyoma and tap Allow when Android asks about log access. The permissions stay until you uninstall, so you can turn USB debugging off again right after.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            DialogButton(onClick = {
                context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("adb", cmd))
                onDismiss()
            }) { Text("Copy command") }
        },
        dismissButton = { DialogButton(onClick = { onDismiss() }) { Text("Close") } },
    )
}

/** The two ways to link over the cable; Vyoma picks either up on its own and prefers it to Wi-Fi. */
@Composable
internal fun UsbHelpDialog(onDismiss: () -> Unit) {
    val context = LocalContext.current
    val cmd = "adb reverse tcp:${LanBackend.USB_REVERSE_PORT} tcp:1716"
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text("Connect over USB") },
        text = {
            Column {
                Text("Plug in the cable, then either:", style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.height(12.dp))
                Text("USB tethering (no setup)", style = MaterialTheme.typography.titleSmall)
                Text(
                    "Turn it on in Settings › Connections › Mobile Hotspot and Tethering. Your PC finds Vyoma over the cable like on Wi-Fi. It also shares this phone's internet with the PC.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(12.dp))
                Text("USB debugging", style = MaterialTheme.typography.titleSmall)
                Text(
                    "For developers who keep it on anyway. With the Vyoma companion on your PC it connects by itself when you plug in; without it, run this on the PC once per plug-in:",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(6.dp))
                Text(cmd, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace))
                Spacer(Modifier.height(12.dp))
                Text(
                    "Either way, Vyoma switches to the cable by itself and back to Wi-Fi when you unplug. Neither is needed for Wi-Fi, which works on its own.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            DialogButton(onClick = {
                context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("adb", cmd))
                onDismiss()
            }) { Text("Copy command") }
        },
        dismissButton = { DialogButton(onClick = onDismiss) { Text("Close") } },
    )
}


/** The controller and touchpad over Bluetooth. Most people's Wi-Fi doesn't need it, so the row opens to say why it's there. */
@androidx.compose.runtime.Composable
private fun SteadierRow(resumeTick: Int) {
    val context = androidx.compose.ui.platform.LocalContext.current
    remember { BluetoothFallback.init(context) }
    val on by BluetoothFallback.steadier.collectAsStateWithLifecycle()
    var open by remember { mutableStateOf(false) }
    var asked by remember { mutableIntStateOf(0) }
    val ready = remember(resumeTick, on, asked) { BluetoothFallback.readiness(context) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        asked++
        if (granted) BluetoothFallback.setSteadier(context, true)
    }
    fun toggle(want: Boolean) {
        if (want && !BluetoothFallback.permitted(context)) ask.launch(android.Manifest.permission.BLUETOOTH_CONNECT)
        else BluetoothFallback.setSteadier(context, want)
    }
    val waiting = when (ready) {
        BluetoothFallback.Readiness.NO_ADAPTER -> "This phone has no Bluetooth."
        BluetoothFallback.Readiness.RADIO_OFF -> "Turn on Bluetooth to use it."
        BluetoothFallback.Readiness.PC_UNKNOWN -> "Connect once with the companion on your PC, so Vyoma can find it."
        BluetoothFallback.Readiness.NOT_PAIRED -> "Pair this phone with your PC in Bluetooth settings."
        else -> null
    }
    ListRow(
        title = "Steadier over Bluetooth",
        subtitle = "If the controller or cursor stutters",
        icon = Icons.Rounded.TouchApp,
        iconTint = SystemColor.BLUE.color,
        showDivider = true,
        trailing = { RowChevron(if (on) "On" else "Off") },
        onClick = { open = true },
    )
    if (open) GlassDialog(
        onDismissRequest = { open = false },
        title = { Text("Steadier over Bluetooth") },
        text = {
            Column(Modifier.heightIn(max = 420.dp).fadingVerticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    "On a busy Wi-Fi network, the controller and touchpad can stall for a moment: a game reacts late, and the cursor stutters or jumps.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text(
                    "This sends them over Bluetooth instead. Good Wi-Fi can be quicker, but Bluetooth is usually more even.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text(
                    "If they already feel smooth, you don't need it. A cable is always used first, and typing and files stay as they are.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text(
                    "Needs the companion on your PC, which asks once to allow control of the cursor.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (on && waiting != null) Text(waiting, style = MaterialTheme.typography.bodySmall, color = SystemColor.ORANGE.color)
                ListRow(
                    title = "Use Bluetooth for both",
                    horizontalPadding = 0.dp,
                    haptic = if (on) Haptic.ToggleOff else Haptic.ToggleOn,
                    onClick = { toggle(!on) },
                    trailing = { VyomaSwitch(checked = on, onCheckedChange = ::toggle, contentDescription = "Use Bluetooth for the controller and touchpad") },
                )
            }
        },
        confirmButton = { DialogButton(onClick = { open = false }) { Text("Done") } },
    )
}

/** The fallback link: a switch, and what it still needs in words. Off until asked for (it needs a permission). */
@androidx.compose.runtime.Composable
private fun BluetoothRow(resumeTick: Int) {
    val context = androidx.compose.ui.platform.LocalContext.current
    remember { BluetoothFallback.init(context) }
    val on by BluetoothFallback.enabled.collectAsStateWithLifecycle()
    var asked by remember { mutableIntStateOf(0) }
    val ready = remember(resumeTick, on, asked) { BluetoothFallback.readiness(context) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        asked++
        if (granted) BluetoothFallback.setEnabled(context, true)
        else runCatching {
            context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + context.packageName)))
        }
    }
    fun openBluetoothSettings() = runCatching { context.startActivity(Intent(Settings.ACTION_BLUETOOTH_SETTINGS)) }
    fun toggle(want: Boolean) {
        if (!want) BluetoothFallback.setEnabled(context, false)
        else if (BluetoothFallback.permitted(context)) BluetoothFallback.setEnabled(context, true)
        else ask.launch(android.Manifest.permission.BLUETOOTH_CONNECT)
    }
    val needsPairing = on && (ready == BluetoothFallback.Readiness.NOT_PAIRED || ready == BluetoothFallback.Readiness.RADIO_OFF)
    ListRow(
        title = "Bluetooth",
        subtitle = when {
            !on -> "Connect without Wi-Fi or cable"
            ready == BluetoothFallback.Readiness.NO_ADAPTER -> "This phone has no Bluetooth"
            ready == BluetoothFallback.Readiness.RADIO_OFF -> "Turn on Bluetooth to use it"
            ready == BluetoothFallback.Readiness.PC_UNKNOWN -> "Connect once over Wi-Fi so it can find your PC"
            ready == BluetoothFallback.Readiness.NOT_PAIRED -> "Pair this phone with your PC in Bluetooth settings"
            else -> "Connect without Wi-Fi or cable"
        },
        icon = Icons.Rounded.Bluetooth,
        iconTint = SystemColor.BLUE.color,
        showDivider = true,
        haptic = if (on) Haptic.ToggleOff else Haptic.ToggleOn,
        onClick = { if (needsPairing) openBluetoothSettings() else toggle(!on) },
        trailing = { VyomaSwitch(checked = on, onCheckedChange = ::toggle, contentDescription = "Bluetooth") },
    )
}

@Composable
private fun FingerprintLockRow(resumeTick: Int) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var on by remember { mutableStateOf(AppLock.enabled(context)) }
    val usable = remember(resumeTick) { AppLock.canAsk(context) }
    val scope = rememberCoroutineScope()
    // The clipboard widget hides what was copied while the lock is on.
    fun widgets() { scope.launch { dev.vyoma.connect.widget.Widgets.refresh(context) } }
    fun toggle(want: Boolean) {
        if (want) {
            AppLock.setEnabled(context, true)
            on = true
            widgets()
        } else {
            AppLock.confirm(context, "To turn off the fingerprint lock") {
                AppLock.setEnabled(context, false)
                on = false
                widgets()
            }
        }
    }
    ListRow(
        title = "Fingerprint lock",
        subtitle = if (usable) "Unlock, files, screen and more" else "Set a screen lock on this phone to use it",
        icon = Icons.Rounded.Fingerprint,
        iconTint = SystemColor.MINT.color,
        haptic = if (on) Haptic.ToggleOff else Haptic.ToggleOn,
        showDivider = on && usable,
        onClick = { toggle(!on) },
        trailing = { VyomaSwitch(checked = on, onCheckedChange = ::toggle, contentDescription = "Fingerprint lock") },
    )
    if (on && usable) UnlockCountsRow()
}

@Composable
private fun UnlockCountsRow() {
    val context = androidx.compose.ui.platform.LocalContext.current
    var on by remember { mutableStateOf(AppLock.unlockCounts(context)) }
    fun toggle(want: Boolean) {
        if (want) {
            // Loosens the lock, so it's asked like turning the lock off.
            AppLock.confirm(context, "To skip the fingerprint after unlocking") {
                AppLock.setUnlockCounts(context, true)
                on = true
            }
        } else {
            AppLock.setUnlockCounts(context, false)
            on = false
        }
    }
    ListRow(
        title = "Phone unlock counts",
        subtitle = "No prompt within 30 s of unlocking",
        icon = Icons.Rounded.LockOpen,
        iconTint = SystemColor.MINT.color,
        haptic = if (on) Haptic.ToggleOff else Haptic.ToggleOn,
        onClick = { toggle(!on) },
        trailing = { VyomaSwitch(checked = on, onCheckedChange = ::toggle, contentDescription = "Phone unlock counts") },
    )
}
