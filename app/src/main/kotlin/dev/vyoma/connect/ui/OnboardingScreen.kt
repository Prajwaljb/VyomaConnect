package dev.vyoma.connect.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.SystemColor
import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.Sms
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.platform.NotificationBridge
import dev.vyoma.connect.platform.PhonePermissions
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.service.LinkService

/** What finding the PC needs: local network access (Android 17+) and our notification for the link. */
private val corePermissions = buildList {
    add(Manifest.permission.POST_NOTIFICATIONS)
    if (Build.VERSION.SDK_INT >= 37) add("android.permission.ACCESS_LOCAL_NETWORK")
}.toTypedArray()

private fun Context.granted(permission: String) =
    ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED

/**
 * First run, as one checklist in the order it matters: allow finding the PC, pair with it, then
 * the optional extras (pairing used to sit below all four of them). Every step can be done later
 * from Settings, so nothing here blocks.
 */
@Composable
fun OnboardingScreen(vm: AppViewModel, onDone: () -> Unit) {
    val context = LocalContext.current
    // Re-read grants when coming back from system pages.
    var tick by remember { mutableIntStateOf(0) }
    LifecycleResumeEffect(Unit) {
        tick++
        onPauseOrDispose { }
    }
    var showAdbHelp by rememberSaveable { mutableStateOf(false) }

    val coreOn = remember(tick) { corePermissions.all(context::granted) }
    val notifOn = remember(tick) { NotificationBridge.isEnabled(context) }
    val phoneOn = remember(tick) { PhonePermissions.allGranted(context) }
    val batteryOn = remember(tick) { context.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName) }
    val clipOn = remember(tick) { vm.backgroundClipboardSync() }

    // The link service starts as soon as it can find PCs; discovery then fills the list below.
    LaunchedEffect(coreOn) { if (coreOn) vm.startService() }
    val askCore = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { tick++ }
    val askPhone = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        tick++
        if (result.values.any { it }) LinkService.start(context, refresh = true, visible = true)
        if (result.values.any { !it } && !PhonePermissions.allGranted(context)) {
            runCatching { context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))) }
        }
    }

    val devices by vm.devices.collectAsStateWithLifecycle()
    val pairing = devices.filter { it.pairState == PairState.REQUESTED_BY_PEER || it.pairState == PairState.REQUESTED_BY_US }
    val paired = devices.filter { it.isPaired }
    val available = devices.filter { !it.isPaired && it.isReachable && it !in pairing }

    CollapsingHeaderScaffold(
        title = "Welcome",
        subtitle = "Find your PC, pair, and you're set. The extras can wait for Settings.",
    ) {
        item(key = "needed-label") { SectionLabel("To find your PC") }
        item(key = "needed") {
            SectionCard(Modifier.padding(top = 8.dp)) {
                Step(
                    Icons.Rounded.Wifi, SystemColor.BLUE, "Nearby devices",
                    if (coreOn) "Vyoma can look for your PC on this network" else "Lets Vyoma find your PC on Wi-Fi and stay connected",
                    done = coreOn,
                ) { askCore.launch(corePermissions) }
            }
        }

        item(key = "pair-label") { SectionLabel("Pair with your PC") }
        items(pairing, key = { "p" + it.identity.deviceId }) { state ->
            PairingCard(
                state = state,
                onAccept = { vm.acceptPair(state.identity.deviceId) },
                onReject = { vm.rejectPair(state.identity.deviceId) },
                modifier = Modifier.padding(top = 8.dp).animateItem(),
            )
        }
        if (paired.isNotEmpty() || available.isNotEmpty()) {
            item(key = "pcs") {
                SectionCard(Modifier.padding(top = 8.dp).animateItem()) {
                    val rows = paired + available
                    rows.forEachIndexed { i, state ->
                        ListRow(
                            title = state.identity.deviceName,
                            subtitle = if (state.isPaired) "Paired" else transportLabel(state),
                            icon = state.identity.deviceType.icon(),
                            iconTint = SystemColor.BLUE.color,
                            showDivider = i < rows.lastIndex,
                            trailing = {
                                if (state.isPaired) Icon(Icons.Rounded.CheckCircle, null, tint = MaterialTheme.colorScheme.primary)
                                else PillButton("Pair", onClick = { vm.requestPair(state.identity.deviceId) })
                            },
                        )
                    }
                }
            }
        } else if (pairing.isEmpty()) {
            item(key = "searching") {
                if (coreOn) SearchingCard(vm, Modifier.padding(top = 8.dp).animateItem())
                else SectionCard(Modifier.padding(top = 8.dp)) {
                    ListRow(title = "Allow nearby devices first", subtitle = "Then open KDE Connect or GSConnect on your PC, on the same Wi-Fi")
                }
            }
        }

        item(key = "extras-label") { SectionLabel("Recommended") }
        item(key = "extras") {
            SectionCard(Modifier.padding(top = 8.dp)) {
                Step(Icons.Rounded.BatteryChargingFull, SystemColor.GREEN, "Stay connected", "Keeps your phone from putting Vyoma to sleep", batteryOn, divider = true) {
                    requestUnrestricted(context)
                }
                Step(Icons.Rounded.Notifications, SystemColor.RED, "Notifications on PC", "See, reply to and dismiss them from your PC", notifOn, divider = true) {
                    runCatching { context.startActivity(NotificationBridge.settingsIntent(context)) }
                }
                Step(Icons.Rounded.Sms, SystemColor.GREEN, "Calls & messages", "Text from your PC and see who's calling", phoneOn, divider = true) {
                    askPhone.launch(PhonePermissions.all)
                }
                Step(Icons.Rounded.ContentPaste, SystemColor.ORANGE, "Automatic clipboard", "One-time ADB grant, no root needed", clipOn, action = "How") {
                    showAdbHelp = true
                }
            }
        }

        item(key = "done") {
            Column(
                Modifier.fillMaxWidth().padding(top = 20.dp, bottom = 32.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                PillButton("Start using Vyoma", onClick = onDone, enabled = paired.isNotEmpty(), large = true, haptic = Haptic.Success)
                if (paired.isEmpty()) {
                    PillButton("Skip for now", onClick = onDone, prominent = false, large = true)
                }
            }
        }
    }

    if (showAdbHelp) ClipboardAdbDialog { showAdbHelp = false }
}

/** One checklist row: what it gives you, and either a tick or the button that sets it up. */
@Composable
private fun Step(
    icon: ImageVector,
    color: SystemColor,
    title: String,
    subtitle: String,
    done: Boolean,
    divider: Boolean = false,
    action: String = "Allow",
    onClick: () -> Unit,
) {
    ListRow(
        title = title,
        subtitle = subtitle,
        icon = icon,
        iconTint = color.color,
        showDivider = divider,
        trailing = {
            if (done) Icon(Icons.Rounded.CheckCircle, "Done", tint = MaterialTheme.colorScheme.primary)
            else PillButton(action, onClick = onClick, prominent = false)
        },
    )
}
