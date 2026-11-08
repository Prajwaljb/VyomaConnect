package dev.vyoma.connect.ui

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.rounded.Folder
import dev.vyoma.connect.container
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.NetworkCheck
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import dev.vyoma.connect.designsystem.LiquidReveal
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.RowDivider
import dev.vyoma.connect.designsystem.fadingVerticalScroll
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material.icons.rounded.TouchApp
import dev.vyoma.connect.designsystem.accent
import androidx.compose.material.icons.rounded.Park
import dev.vyoma.connect.designsystem.LocalStatus
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.text.BasicText
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.graphics.SolidColor
import dev.vyoma.connect.designsystem.Haptic
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
import androidx.compose.runtime.DisposableEffect
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
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.Palette
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.pressable

@Composable
fun SettingsScreen(vm: AppViewModel, onBack: (() -> Unit)? = null, onAutomation: () -> Unit = {}, onConnection: () -> Unit = {}, onNotifications: () -> Unit = {}) {
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
        item(key = "sync-label") { SectionLabel("Share with your PC") }
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
                PhoneFilesRow(resumeTick)
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

        item(key = "look-label") { SectionLabel("Appearance") }
        item(key = "look") {
            SectionCard(contentPadding = PaddingValues(vertical = 16.dp)) {
                AccentPicker(theme.palette, vm::setPalette)
                Spacer(Modifier.height(18.dp))
                SegmentedControl(
                    options = DarkMode.entries.map { it.name.lowercase().replaceFirstChar(Char::uppercase) },
                    selected = theme.darkMode.ordinal,
                    onSelect = { vm.setDarkMode(DarkMode.entries[it]) },
                    modifier = Modifier.padding(horizontal = 20.dp),
                )
            }
        }

        // The look's on/off effects, apart from the choices above (four kinds of control in one card read busy).
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

        item(key = "hosts-label") { SectionLabel("Ways to connect") }
        item(key = "hosts") {
            SectionCard(Modifier.animateContentSize(VyomaMotion.spatial())) {
                BluetoothRow(vm, resumeTick)
                SteadierRow(resumeTick)
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
                    showDivider = true,
                    onClick = { showUsbHelp = true },
                )
                ListRow(
                    title = "Connection checks",
                    subtitle = "What's working, and what to do if it isn't",
                    icon = Icons.Rounded.NetworkCheck,
                    iconTint = SystemColor.GREEN.color,
                    trailing = { RowChevron() },
                    onClick = onConnection,
                )
            }
        }

        item(key = "security-label") { SectionLabel("Security") }
        item(key = "security") { SectionCard { FingerprintLockRow(resumeTick) } }

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

        item(key = "device-label") { SectionLabel("This phone") }
        item(key = "device") {
            SectionCard {
                ListRow(
                    title = "Device name",
                    subtitle = name,
                    icon = Icons.Rounded.Edit,
                    iconTint = SystemColor.BLUE.color,
                    trailing = { RowChevron() },
                    showDivider = true,
                    onClick = { editingName = true },
                )
                ListRow(
                    title = "Show setup again",
                    subtitle = "The first-run steps and tips",
                    icon = Icons.Rounded.Refresh,
                    iconTint = SystemColor.BLUE.color,
                    trailing = { RowChevron() },
                    onClick = vm::replayOnboarding,
                )
                ListRow(
                    title = "Vyoma",
                    subtitle = "Version ${context.packageManager.getPackageInfo(context.packageName, 0).versionName} · KDE Connect protocol v8",
                    icon = Icons.Rounded.Info,
                    iconTint = SystemColor.GRAY.color,
                )
            }
        }

    }

    if (showAdbHelp) ClipboardSetupDialog(vm) { showAdbHelp = false }
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

/**
 * Seven colours in a row, no words under them: seven names in a row couldn't breathe. The chosen one
 * is named in the heading; the two that come from a wallpaper say which by their icon, set apart
 * from the fixed colours.
 */
@Composable
private fun AccentPicker(chosen: Palette, onPick: (Palette) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp), verticalAlignment = Alignment.CenterVertically) {
        Text("Accent", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
        androidx.compose.animation.AnimatedContent(chosen, label = "accent name") { p ->
            Text(
                when (p) {
                    Palette.PC -> "PC wallpaper"
                    Palette.PHONE -> "Phone wallpaper"
                    else -> p.label
                },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
    if (chosen == Palette.PC) Spacer(Modifier.height(2.dp))
    LiquidReveal(chosen == Palette.PC) {
        Text(
            if (AppCanvas.wallpaper == null) "Needs the companion on your PC" else "Colours and light follow your desktop",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 20.dp),
        )
    }
    Spacer(Modifier.height(14.dp))
    val gap = 17.dp // the line between the wallpapers and the fixed colours
    BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = 20.dp)) {
        val share = (maxWidth - gap) / Palette.entries.size
        val size = (share - 10.dp).coerceAtMost(40.dp)
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Palette.entries.forEach { p ->
                PaletteSwatch(p, selected = chosen == p, size, Modifier.width(share)) { onPick(p) }
                if (p == Palette.PHONE) {
                    Box(Modifier.width(gap), contentAlignment = Alignment.Center) {
                        Box(Modifier.width(1.dp).height(size * 0.6f).background(MaterialTheme.colorScheme.outlineVariant))
                    }
                }
            }
        }
    }
}

@Composable
private fun PaletteSwatch(palette: Palette, selected: Boolean, size: androidx.compose.ui.unit.Dp, modifier: Modifier, onClick: () -> Unit) {
    val dark = MaterialTheme.colorScheme.background.red < 0.5f
    val ring by animateColorAsState(if (selected) MaterialTheme.colorScheme.onSurface else Color.Transparent, label = "ring")
    val inset by animateDpAsState(if (selected) 4.dp else 0.dp, VyomaMotion.spatial(), label = "inset")
    Box(
        modifier
            .semantics(mergeDescendants = true) { contentDescription = "Accent: ${palette.label}"; this.selected = selected; role = Role.RadioButton }
            .pressable(haptic = Haptic.Select, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(size).clip(CircleShape).border(2.dp, ring, CircleShape).padding(inset.coerceAtLeast(0.dp)), // the spring overshoots below zero
            contentAlignment = Alignment.Center,
        ) {
            val context = LocalContext.current
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
                when {
                    palette == Palette.PC -> Icon(Icons.Rounded.Computer, null, tint = Color.White, modifier = Modifier.size(18.dp))
                    palette == Palette.PHONE -> Icon(Icons.Rounded.PhoneAndroid, null, tint = Color.White, modifier = Modifier.size(18.dp))
                    selected -> Icon(Icons.Rounded.Check, null, tint = Color.White, modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

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

/**
 * Sync when closed: Android only lets a computer grant this, and the companion is that computer. Three
 * steps with the first and last ticking themselves off, so it's clear how far along it is. The
 * command is folded away for anyone without the companion.
 */
@Composable
internal fun ClipboardSetupDialog(vm: AppViewModel, onDismiss: () -> Unit) {
    val context = LocalContext.current
    var debugging by remember { mutableStateOf(false) }
    var granted by remember { mutableStateOf(vm.backgroundClipboardSync()) }
    var manual by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        while (!granted) {
            debugging = Settings.Global.getInt(context.contentResolver, Settings.Global.ADB_ENABLED, 0) == 1
            granted = vm.backgroundClipboardSync()
            kotlinx.coroutines.delay(1_000)
        }
    }
    val cmd = "adb shell pm grant ${context.packageName} android.permission.READ_LOGS && " +
        "adb shell appops set ${context.packageName} SYSTEM_ALERT_WINDOW allow"
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (granted) "Clipboard sync is on" else "Sync when closed") },
        text = {
            Column(Modifier.heightIn(max = 460.dp).fadingVerticalScroll(rememberScrollState())) {
                if (granted) {
                    Text("Anything you copy now reaches your PC, even with Vyoma closed. It stays until you uninstall, so you can turn USB debugging off again.")
                } else {
                    Text("Android only lets the app in front read the clipboard. A computer can lift that once, and your PC can do it for you.")
                    Spacer(Modifier.height(16.dp))
                    SetupStep(1, "Turn on USB debugging", if (debugging) "On" else "It's under Developer options. Not there? Tap Build number 7 times in About phone.", debugging) {
                        val options = Settings.Global.getInt(context.contentResolver, Settings.Global.DEVELOPMENT_SETTINGS_ENABLED, 0) == 1
                        runCatching { context.startActivity(Intent(if (options) Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS else Settings.ACTION_DEVICE_INFO_SETTINGS)) }
                    }
                    SetupStep(2, "Plug into your PC", "Then tap Allow when this phone asks about USB debugging.", done = false)
                    SetupStep(3, "Click Allow on your PC", "In the notification Vyoma shows, or on this phone's page in Vyoma's window. This turns on by itself after that.", done = false)
                    Spacer(Modifier.height(4.dp))
                    Text(
                        if (manual) "Hide the command" else "No companion on the PC?",
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(vertical = 8.dp).pressable(haptic = Haptic.Tap) { manual = !manual },
                    )
                    LiquidReveal(manual) {
                        Column {
                            Text("Run this from any computer with adb, then reopen Vyoma:", style = MaterialTheme.typography.bodySmall)
                            Spacer(Modifier.height(6.dp))
                            Text(
                                cmd,
                                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                                modifier = Modifier.pressable(haptic = Haptic.Tap) {
                                    context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("adb", cmd))
                                },
                            )
                            Spacer(Modifier.height(4.dp))
                            Text("Tap it to copy.", style = MaterialTheme.typography.bodySmall)
                        }
                    }
                }
            }
        },
        confirmButton = { DialogButton(onClick = onDismiss) { Text(if (granted) "Done" else "Close") } },
    )
}

/** One numbered step in a how-to: a tick once it's done, and a button if the phone can take you there. */
@Composable
private fun SetupStep(number: Int, title: String, body: String, done: Boolean, onOpen: (() -> Unit)? = null) {
    Row(Modifier.fillMaxWidth().padding(bottom = 14.dp)) {
        val tone by animateColorAsState(if (done) SystemColor.GREEN.color else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.14f), label = "step")
        Box(Modifier.size(26.dp).clip(CircleShape).background(tone), contentAlignment = Alignment.Center) {
            if (done) Icon(Icons.Rounded.Check, "Done", tint = Color.White, modifier = Modifier.size(16.dp))
            else Text("$number", style = MaterialTheme.typography.labelMedium)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(body, style = MaterialTheme.typography.bodySmall)
            if (onOpen != null && !done) {
                Spacer(Modifier.height(8.dp))
                PillButton("Open settings", onClick = onOpen, prominent = false)
            }
        }
    }
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
        else runCatching {
            context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + context.packageName)))
        }
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
                LiquidReveal(on && waiting != null) { Text(waiting.orEmpty(), style = MaterialTheme.typography.bodySmall, color = SystemColor.ORANGE.color) }
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
private fun BluetoothRow(vm: AppViewModel, resumeTick: Int) {
    val context = androidx.compose.ui.platform.LocalContext.current
    remember { BluetoothFallback.init(context) }
    val on by BluetoothFallback.enabled.collectAsStateWithLifecycle()
    var asked by remember { mutableIntStateOf(0) }
    // Pairing ends in Android's dialog and the PC's: recheck when it does.
    var pairing by remember { mutableStateOf(false) }
    DisposableEffect(Unit) {
        val bonds = object : android.content.BroadcastReceiver() {
            override fun onReceive(c: android.content.Context, i: Intent) {
                @Suppress("DEPRECATION") // the typed getter is API 33
                val device = i.getParcelableExtra<android.bluetooth.BluetoothDevice>(android.bluetooth.BluetoothDevice.EXTRA_DEVICE)
                if (device?.address !in BluetoothFallback.addresses(context).values) return // headphones and the like
                pairing = i.getIntExtra(android.bluetooth.BluetoothDevice.EXTRA_BOND_STATE, 0) == android.bluetooth.BluetoothDevice.BOND_BONDING
                asked++
            }
        }
        androidx.core.content.ContextCompat.registerReceiver(
            context, bonds, android.content.IntentFilter(android.bluetooth.BluetoothDevice.ACTION_BOND_STATE_CHANGED),
            androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        onDispose { context.unregisterReceiver(bonds) }
    }
    val ready = remember(resumeTick, on, asked) { BluetoothFallback.readiness(context) }
    // Android's Bluetooth page stops calling the PC paired once its audio and calls are off, so this says it.
    val devices by vm.devices.collectAsStateWithLifecycle()
    val known = remember(resumeTick, on, asked) { BluetoothFallback.addresses(context).keys }
    val pcName = devices.firstOrNull { it.identity.deviceId in known }?.identity?.deviceName ?: "your PC"
    val overRadio = devices.any { TransportKind.BLUETOOTH in it.transports && it.isReachable }
    fun openBluetoothSettings() = runCatching { context.startActivity(Intent(Settings.ACTION_BLUETOOTH_SETTINGS)) }
    // Not paired: the app starts it (both screens show a code), else Bluetooth settings.
    fun pair() {
        if (BluetoothFallback.pair(context)) pairing = true else openBluetoothSettings()
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        asked++
        if (granted) {
            BluetoothFallback.setEnabled(context, true)
            if (BluetoothFallback.readiness(context) == BluetoothFallback.Readiness.NOT_PAIRED) pair()
        } else runCatching {
            context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + context.packageName)))
        }
    }
    fun toggle(want: Boolean) {
        if (!want) BluetoothFallback.setEnabled(context, false)
        else if (BluetoothFallback.permitted(context)) {
            BluetoothFallback.setEnabled(context, true)
            if (ready == BluetoothFallback.Readiness.NOT_PAIRED) pair()
        } else ask.launch(android.Manifest.permission.BLUETOOTH_CONNECT)
    }
    val needsPairing = ready == BluetoothFallback.Readiness.NOT_PAIRED
    val needsRadio = on && ready == BluetoothFallback.Readiness.RADIO_OFF
    ListRow(
        title = "Bluetooth",
        subtitle = when {
            // Said whether the switch is on or not: without the pairing it can never work.
            pairing -> "Pairing: confirm the code on the phone and on $pcName"
            needsPairing -> "Not paired with $pcName yet. Tap to pair"
            !on -> "Connect without Wi-Fi or cable"
            ready == BluetoothFallback.Readiness.NO_ADAPTER -> "This phone has no Bluetooth"
            ready == BluetoothFallback.Readiness.RADIO_OFF -> "Turn on Bluetooth to use it"
            ready == BluetoothFallback.Readiness.PC_UNKNOWN -> "Connect once over Wi-Fi so it can find your PC"
            overRadio -> "Connected to $pcName over Bluetooth"
            else -> "Paired with $pcName, ready if Wi-Fi drops"
        },
        icon = Icons.Rounded.Bluetooth,
        iconTint = SystemColor.BLUE.color,
        showDivider = true,
        haptic = if (on) Haptic.ToggleOff else Haptic.ToggleOn,
        onClick = { if (needsPairing) pair() else if (needsRadio) openBluetoothSettings() else toggle(!on) },
        trailing = { VyomaSwitch(checked = on, onCheckedChange = ::toggle, contentDescription = "Bluetooth") },
    )
}

/** "Browse this phone from your PC": the switch, and Android's All files access it needs. */
@Composable
private fun PhoneFilesRow(resumeTick: Int) {
    val context = androidx.compose.ui.platform.LocalContext.current
    remember { dev.vyoma.connect.platform.PhoneFileSharing.init(context) }
    val on by dev.vyoma.connect.platform.PhoneFileSharing.on.collectAsStateWithLifecycle()
    val allowed = remember(resumeTick) { dev.vyoma.connect.platform.PhoneFileSharing.allowed() }
    fun set(want: Boolean) {
        dev.vyoma.connect.platform.PhoneFileSharing.setOn(context, want)
        if (!want) (context.applicationContext as android.app.Application).container.phoneFiles.stopAll()
        else if (!dev.vyoma.connect.platform.PhoneFileSharing.allowed()) runCatching { context.startActivity(dev.vyoma.connect.platform.PhoneFileSharing.accessIntent(context)) }
    }
    ListRow(
        title = "Browse this phone from your PC",
        subtitle = when {
            !on -> "Its file manager opens your phone's storage"
            !allowed -> "Allow All files access to finish"
            else -> "Your PC can open and change files here"
        },
        icon = Icons.Rounded.Folder,
        iconTint = SystemColor.BLUE.color,
        showDivider = true,
        haptic = if (on) Haptic.ToggleOff else Haptic.ToggleOn,
        onClick = { if (on && !allowed) runCatching { context.startActivity(dev.vyoma.connect.platform.PhoneFileSharing.accessIntent(context)) } else set(!on) },
        trailing = { VyomaSwitch(checked = on, onCheckedChange = ::set, contentDescription = "Browse this phone from your PC") },
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
        onClick = { toggle(!on) },
        trailing = { VyomaSwitch(checked = on, onCheckedChange = ::toggle, contentDescription = "Fingerprint lock") },
    )
    LiquidReveal(on && usable) {
        Column {
            RowDivider()
            UnlockCountsRow()
        }
    }
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
