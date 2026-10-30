package dev.vyoma.connect.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
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
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.DarkMode
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
fun SettingsScreen(vm: AppViewModel, onBack: (() -> Unit)? = null) {
    val theme by vm.theme.collectAsStateWithLifecycle()
    val name by vm.deviceName.collectAsStateWithLifecycle()
    val context = LocalContext.current
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
    val unrestricted = power.isIgnoringBatteryOptimizations(context.packageName)

    CollapsingHeaderScaffold(
        title = "Settings",
        navigation = onBack?.let { back -> { IconButton(onClick = back) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } } },
    ) {
        item(key = "device") {
            SectionCard {
                ListRow(
                    title = "Device name",
                    subtitle = name,
                    icon = Icons.Rounded.Edit,
                    iconTint = SystemColor.BLUE.color,
                    onClick = { editingName = true },
                )
            }
        }

        item(key = "look-label") { SectionLabel("Appearance") }
        item(key = "look") {
            SectionCard(contentPadding = PaddingValues(vertical = 16.dp)) {
                Text("Accent", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(horizontal = 20.dp))
                Spacer(Modifier.height(12.dp))
                Row(
                    Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Palette.entries.forEach { p ->
                        PaletteSwatch(p, selected = theme.palette == p) { vm.setPalette(p) }
                    }
                }
                Spacer(Modifier.height(20.dp))
                SegmentedControl(
                    options = DarkMode.entries.map { it.name.lowercase().replaceFirstChar(Char::uppercase) },
                    selected = theme.darkMode.ordinal,
                    onSelect = { vm.setDarkMode(DarkMode.entries[it]) },
                    modifier = Modifier.padding(horizontal = 20.dp),
                )
                Spacer(Modifier.height(8.dp))
                ListRow(
                    title = "Pure black",
                    subtitle = "True black backgrounds in dark mode, easy on AMOLED",
                    icon = Icons.Rounded.DarkMode,
                    iconTint = SystemColor.GRAY.color,
                    trailing = { VyomaSwitch(checked = theme.amoled, onCheckedChange = { vm.setAmoled(it) }) },
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
                    subtitle = if (notifOn) "On: reply and dismiss from your PC" else "Allow notification access to mirror them",
                    icon = Icons.Rounded.Notifications,
                    iconTint = SystemColor.RED.color,
                    showDivider = true,
                    onClick = { runCatching { context.startActivity(NotificationBridge.settingsIntent(context)) } },
                )
                ListRow(
                    title = "Calls & messages on PC",
                    subtitle = if (phoneOn) "On: text from the PC, see who's calling, synced contacts"
                    else "Text from your PC and see who's calling",
                    icon = Icons.Rounded.Sms,
                    iconTint = SystemColor.GREEN.color,
                    showDivider = true,
                    onClick = { if (!phoneOn) askPhone.launch(PhonePermissions.all) },
                )
                ListRow(
                    title = "Automatic clipboard sync",
                    subtitle = if (clipOn) "On: copies sync even while Vyoma is closed"
                    else "Needs a one-time ADB grant (no root). Tap for the command.",
                    icon = Icons.Rounded.ContentPaste,
                    iconTint = SystemColor.ORANGE.color,
                    onClick = { if (!clipOn) showAdbHelp = true },
                )
            }
        }

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
                            IconButton(onClick = { vm.removeManualHost(host) }) { Icon(Icons.Rounded.Close, "Remove $host") }
                        },
                    )
                }
                ListRow(
                    title = "Add a PC by IP or name",
                    subtitle = "For Tailscale, or when your PC isn't found automatically",
                    icon = Icons.Rounded.Add,
                    iconTint = SystemColor.BLUE.color,
                    showDivider = true,
                    onClick = { addingHost = true },
                )
                ListRow(
                    title = "Over a USB cable",
                    subtitle = "Faster and steadier than Wi-Fi; works with no network at all",
                    icon = Icons.Rounded.Usb,
                    iconTint = SystemColor.CYAN.color,
                    onClick = { showUsbHelp = true },
                )
            }
        }

        item(key = "bg-label") { SectionLabel("Staying connected") }
        item(key = "bg") {
            SectionCard {
                ListRow(
                    title = if (unrestricted) "Background activity allowed" else "Allow background activity",
                    subtitle = if (unrestricted) "One UI won't put Vyoma to sleep" else "Recommended on Galaxy phones so devices stay connected",
                    icon = Icons.Rounded.BatteryChargingFull,
                    iconTint = SystemColor.GREEN.color,
                    onClick = { if (!unrestricted) requestUnrestricted(context) },
                )
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

    if (addingHost) {
        var draft by rememberSaveable { mutableStateOf("") }
        var invalid by rememberSaveable { mutableStateOf(false) }
        AlertDialog(
            onDismissRequest = { addingHost = false },
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
                TextButton(onClick = {
                    if (vm.addManualHost(draft)) addingHost = false else invalid = true
                }) { Text("Add") }
            },
            dismissButton = { TextButton(onClick = { addingHost = false }) { Text("Cancel") } },
        )
    }

    if (editingName) {
        var draft by rememberSaveable { mutableStateOf(name) }
        AlertDialog(
            onDismissRequest = { editingName = false },
            title = { Text("Device name") },
            text = { OutlinedTextField(draft, { draft = it.take(32) }, singleLine = true) },
            confirmButton = {
                TextButton(enabled = draft.isNotBlank(), onClick = {
                    vm.setDeviceName(draft.trim())
                    editingName = false
                }) { Text("Save") }
            },
            dismissButton = { TextButton(onClick = { editingName = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun PaletteSwatch(palette: Palette, selected: Boolean, onClick: () -> Unit) {
    val dark = MaterialTheme.colorScheme.background.red < 0.5f
    val ring by animateColorAsState(if (selected) MaterialTheme.colorScheme.onSurface else Color.Transparent, label = "ring")
    val inset by animateDpAsState(if (selected) 5.dp else 0.dp, VyomaMotion.spatial(), label = "inset")
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.pressable(onClick = onClick)) {
        Box(
            Modifier.size(52.dp).clip(CircleShape).border(2.dp, ring, CircleShape).padding(inset.coerceAtLeast(0.dp)), // the spring overshoots below zero
            contentAlignment = Alignment.Center,
        ) {
            val fill = if (palette == Palette.DYNAMIC) {
                // Wallpaper option previews the live Material You primary with a soft sweep.
                Brush.sweepGradient(listOf(MaterialTheme.colorScheme.primary, Color(0xFF5CE1E6), MaterialTheme.colorScheme.primary))
            } else {
                Brush.linearGradient(listOf(if (dark) palette.dark else palette.light, palette.light))
            }
            Box(Modifier.size(52.dp).clip(CircleShape).background(fill), contentAlignment = Alignment.Center) {
                if (selected) Icon(Icons.Rounded.Check, null, tint = Color.White, modifier = Modifier.size(20.dp))
            }
        }
        Spacer(Modifier.height(6.dp))
        Text(palette.label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@SuppressLint("BatteryLife") // Core function: the app must stay connected in the background.
internal fun requestUnrestricted(context: android.content.Context) {
    val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${context.packageName}"))
    runCatching { context.startActivity(intent) }
}

/** How to grant automatic clipboard sync once over ADB (no root). */
@Composable
internal fun ClipboardAdbDialog(onDismiss: () -> Unit) {
    val context = LocalContext.current
    val cmd = "adb shell pm grant ${context.packageName} android.permission.READ_LOGS && " +
        "adb shell appops set ${context.packageName} SYSTEM_ALERT_WINDOW allow"
    AlertDialog(
        onDismissRequest = { onDismiss() },
        title = { Text("Automatic clipboard sync") },
        text = {
            Column {
                Text("Android only lets the app in front read the clipboard. With the Vyoma companion on your PC, just plug in with USB debugging on and it's allowed for you. Otherwise run this once from a computer, then reopen Vyoma and tap Allow when Android asks about log access:")
                Spacer(Modifier.height(12.dp))
                Text(cmd, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace))
            }
        },
        confirmButton = {
            TextButton(onClick = {
                context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("adb", cmd))
                onDismiss()
            }) { Text("Copy command") }
        },
        dismissButton = { TextButton(onClick = { onDismiss() }) { Text("Close") } },
    )
}

/** The two ways to link over the cable; Vyoma picks either up on its own and prefers it to Wi-Fi. */
@Composable
private fun UsbHelpDialog(onDismiss: () -> Unit) {
    val context = LocalContext.current
    val cmd = "adb reverse tcp:${LanBackend.USB_REVERSE_PORT} tcp:1716"
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Connect over USB") },
        text = {
            Column {
                Text("Plug in the cable, then either:", style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.height(12.dp))
                Text("USB tethering", style = MaterialTheme.typography.titleSmall)
                Text(
                    "Turn it on in Settings › Connections › Mobile Hotspot and Tethering. Your PC finds Vyoma over the cable like on Wi-Fi.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(12.dp))
                Text("USB debugging", style = MaterialTheme.typography.titleSmall)
                Text("With the Vyoma companion on your PC, that's all: it sets up the link when you plug in. Without it, run this on the PC once per plug-in:", style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.height(6.dp))
                Text(cmd, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace))
                Spacer(Modifier.height(12.dp))
                Text(
                    "Either way, Vyoma switches to the cable by itself and back to Wi-Fi when you unplug.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = {
                context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("adb", cmd))
                onDismiss()
            }) { Text("Copy command") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )
}
