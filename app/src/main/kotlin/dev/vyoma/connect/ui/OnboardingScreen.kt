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
import android.content.ClipData
import androidx.activity.compose.BackHandler
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.mutableFloatStateOf
import dev.vyoma.connect.BuildConfig
import dev.vyoma.connect.designsystem.Backdrop
import dev.vyoma.connect.platform.FrameBudget
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.glassChrome
import dev.vyoma.connect.designsystem.rememberBackdrop
import android.content.ClipboardManager
import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Lightbulb
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import dev.vyoma.connect.designsystem.LiquidReveal
import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.pressable
import kotlinx.coroutines.launch
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
import androidx.compose.material3.Typography
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
import dev.vyoma.connect.platform.NotificationBridge
import dev.vyoma.connect.platform.PhonePermissions
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.service.LinkService

/** What finding the PC needs: local network access (Android 17+) and our notification for the link. */
private val corePermissions = buildList {
    // Android 13+: on 12 there's no such permission and asking is always "denied".
    if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
    if (Build.VERSION.SDK_INT >= 37) add("android.permission.ACCESS_LOCAL_NETWORK")
}.toTypedArray()

private fun Context.granted(permission: String) =
    ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED

/** Where Vyoma's PC app is explained and installed from, for the phone to open on the PC. */
internal const val PC_APP_URL = "https://github.com/Prajwaljb/VyomaConnect/tree/main/desktop/daemon#install"
internal const val PC_APP_INSTALL = "git clone https://github.com/Prajwaljb/VyomaConnect && VyomaConnect/desktop/daemon/install.sh"

/**
 * First run: the show ([WelcomeShow]), then setup one step at a time on the same picture. Vyoma's PC
 * app comes before pairing: it's what the phone pairs with (KDE Connect is the fallback). Each step
 * opens when its turn comes, says why it's asking, and closes into a tick when done. The day runs
 * through it: night while it's looking for the PC, the sun rising as it pairs, morning at the end.
 */
@Composable
fun OnboardingScreen(vm: AppViewModel, onDone: () -> Unit) {
    val backdrop = rememberBackdrop()
    var showing by rememberSaveable { mutableStateOf(true) }
    // The picture holds still behind setup: no frames at rest, only the day moving on when a step's done.
    val clock = rememberShowClock(running = showing)
    val ripple = remember { Ripple() }
    DisposableEffect(Unit) {
        FrameBudget.paused = true
        onDispose { FrameBudget.paused = false }
    }
    val day = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    var titleShown by remember { mutableFloatStateOf(1f) }
    var lensTouched by remember { mutableStateOf(false) }
    val intro = rememberWelcomeIntro()
    Box(Modifier.fillMaxSize()) {
        DuneScene(backdrop, clock, ripple, { day.value }) {
            if (showing) WelcomeTitle({ titleShown }, lensTouched, intro)
        }
        Crossfade(showing, animationSpec = VyomaMotion.effects(), label = "first run") { show ->
            if (show) {
                WelcomeShow(backdrop, clock, intro, page = { at ->
                    titleShown = 1f - at.coerceIn(0f, 1f)
                    scope.launch { day.snapTo(at) }
                }, onLensTouched = { lensTouched = true }, onDone = { showing = false })
            } else {
                BackHandler { showing = true }
                SetupSteps(vm, backdrop, ripple, onStep = { target ->
                    scope.launch { day.animateTo(target, spring(dampingRatio = 1f, stiffness = 14f)) }
                }, onDone = {
                    FirstRun.assembling = true
                    onDone()
                })
            }
        }
    }
}

// Where the day stands at each step of setup: on from the show's last scene, past midnight.
// Each rests near a whole scene: halfway from night to morning the colours mix to a grey mauve.
private val DAY_AT_STEP = floatArrayOf(5f, 5.82f, 6f, 6.6f)

/** One step at a time; nothing but the first two matters for using the app, and all of it is in Settings later. */
@Composable
private fun SetupSteps(vm: AppViewModel, backdrop: Backdrop, ripple: Ripple, onStep: (Float) -> Unit, onDone: () -> Unit) {
    val context = LocalContext.current
    // Re-read grants when coming back from system pages.
    var tick by remember { mutableIntStateOf(0) }
    LifecycleResumeEffect(Unit) {
        tick++
        onPauseOrDispose { }
    }
    var showClipboard by rememberSaveable { mutableStateOf(false) }

    val coreOn = remember(tick) { corePermissions.all(context::granted) }
    val notifOn = remember(tick) { NotificationBridge.isEnabled(context) }
    val phoneOn = remember(tick) { PhonePermissions.allGranted(context) }
    val batteryOn = remember(tick) { context.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName) }
    val clipOn = remember(tick, showClipboard) { vm.backgroundClipboardSync() }

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
    // The demo's laptop is paired from the start, which would skip the best part: it waits to be asked.
    var demoPaired by rememberSaveable { mutableStateOf(!BuildConfig.DEMO) }
    val realPaired = devices.filter { it.isPaired && !it.isCompanion }
    val paired = if (demoPaired) realPaired else emptyList()
    val available = (if (demoPaired) emptyList() else realPaired) + devices.filter { !it.isPaired && !it.isCompanion && it.isReachable && it !in pairing }
    // Vyoma on the PC, seen before pairing too (its announcement says what it can do): its own app, or
    // the companion beside KDE Connect. A PC that's only KDE Connect still pairs, with less to offer.
    val vyomaPc = devices.firstOrNull { it.identity.featuresOf != null && it.isReachable && (it.isPaired || !it.isCompanion) }
    val vyomaOn = vyomaPc != null
    // KDE Connect (or GSConnect) instead: the PC step stands aside and pairing opens.
    var withoutVyoma by rememberSaveable { mutableStateOf(false) }
    val anyPc = paired.isNotEmpty() || available.isNotEmpty() || pairing.isNotEmpty()

    // The first thing left to do is the one that's open; a finger can open another. Once any PC shows
    // up, pairing comes next whatever it runs: Vyoma's PC app takes over KDE Connect's pairing anyway.
    val current = when {
        !coreOn -> 0
        paired.isEmpty() && !vyomaOn && !anyPc && !withoutVyoma -> 1
        paired.isEmpty() -> 2
        else -> 3
    }
    var opened by rememberSaveable { mutableIntStateOf(current) }
    LaunchedEffect(current) {
        opened = current
        onStep(DAY_AT_STEP[current])
    }
    // Only a pairing seen happening, not a PC paired before this began.
    var moment by remember { mutableStateOf<String?>(null) }
    var seenUnpaired by remember { mutableStateOf(false) }
    LaunchedEffect(paired.isEmpty()) {
        if (paired.isEmpty()) seenUnpaired = true
        else if (seenUnpaired) moment = paired.first().identity.deviceName
    }

    // Cards fade while the moment of pairing has the picture to itself.
    val cardsShown by animateFloatAsState(if (moment == null) 1f else 0f, VyomaMotion.effects(), label = "cards")
    // Clear glass over a bright picture: the words carry their own shadow instead of the glass going grey.
    MaterialTheme(colorScheme = MaterialTheme.colorScheme.copy(onSurfaceVariant = Color.White), typography = MaterialTheme.typography.shadowed()) { CompositionLocalProvider(LocalContentColor provides Color.White) { LazyColumn(
        Modifier.fillMaxSize().graphicsLayer { alpha = cardsShown },
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp + WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        // Set on the picture itself rather than in a page scaffold, whose own layer left this glass with nothing to sample.
        item(key = "title") {
            Column(Modifier.fillMaxWidth().statusBarsPadding().padding(top = 72.dp, bottom = 40.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("Let's set up", style = MaterialTheme.typography.displaySmall.copy(fontWeight = FontWeight.Bold), color = Color.White)
                Spacer(Modifier.height(6.dp))
                Text(
                    when (current) {
                        0 -> "Let's find your PC. This takes a minute."
                        1 -> "Step 2 of 4: Vyoma on your PC"
                        2 -> "Step 3 of 4: pair with your PC"
                        else -> "Nearly there"
                    },
                    style = MaterialTheme.typography.bodyLarge,
                    color = Color.White.copy(alpha = 0.9f),
                    textAlign = TextAlign.Center,
                )
            }
        }
        item(key = "find") {
            SetupCard(backdrop, 1, "Let Vyoma look for your PC", if (coreOn) "Allowed" else "Needs one permission", coreOn, opened == 0, { opened = 0 }) {
                Tip("Vyoma only looks on the network you're on, and nothing leaves it. It also needs its notification to stay connected.")
                if (!coreOn) PillButton("Allow", onClick = { askCore.launch(corePermissions) }, large = true)
            }
        }
        item(key = "pc-app") {
            val status = when {
                vyomaOn -> "On " + vyomaPc!!.identity.deviceName
                anyPc -> "Your PC has KDE Connect: Vyoma does more"
                withoutVyoma -> "Using KDE Connect"
                else -> "What your phone pairs with"
            }
            SetupCard(backdrop, 2, "Vyoma on your PC", status, vyomaOn, opened == 1, { opened = 1 }) {
                CompanionSetup(vm, vyomaOn, paired.firstOrNull()?.identity?.deviceId)
                if (!vyomaOn && !anyPc && !withoutVyoma) {
                    Tip("Already use KDE Connect or GSConnect? Vyoma pairs with them too, with fewer features.")
                    PillButton("Use KDE Connect", onClick = { withoutVyoma = true }, prominent = false)
                }
            }
        }
        item(key = "pair") {
            SetupCard(backdrop, 3, "Pair with your PC", paired.firstOrNull()?.identity?.deviceName ?: "Both need to agree once", paired.isNotEmpty(), opened == 2, { opened = 2 }) {
                pairing.forEach { state ->
                    PairingCard(
                        state = state,
                        onAccept = { vm.acceptPair(state.identity.deviceId) },
                        onReject = { vm.rejectPair(state.identity.deviceId) },
                    )
                }
                when {
                    !coreOn -> Tip("Do step 1 first, then this starts looking.")
                    paired.isNotEmpty() || available.isNotEmpty() -> {
                        val rows = paired + available
                        rows.forEach { state ->
                            ListRow(
                                title = state.identity.deviceName,
                                subtitle = if (state in paired) "Paired" else transportLabel(state),
                                icon = state.identity.deviceType.icon(),
                                iconTint = SystemColor.BLUE.color,
                                horizontalPadding = 0.dp,
                                trailing = {
                                    if (state in paired) Icon(Icons.Rounded.CheckCircle, null, tint = MaterialTheme.colorScheme.primary)
                                    else PillButton("Pair", onClick = { if (demoPaired) vm.requestPair(state.identity.deviceId) else demoPaired = true })
                                },
                            )
                        }
                        if (paired.isEmpty()) Tip("You'll check that a code matches on both screens, and accept on the PC too: Vyoma asks there in a notification and in its window.")
                    }
                    pairing.isEmpty() -> {
                        SearchingCard(vm)
                        Tip("Not showing up? Check Vyoma (or KDE Connect) is running on the PC and both are on the same Wi-Fi, or plug the phone in with a cable.")
                    }
                }
            }
        }
        item(key = "extras") {
            SetupCard(backdrop, 4, "A few things that help", "Each one is optional", notifOn && phoneOn && batteryOn && clipOn, opened == 3, { opened = 3 }) {
                Step(Icons.Rounded.BatteryChargingFull, SystemColor.GREEN, "Stay connected", "Otherwise Android puts Vyoma to sleep, and the PC loses the phone", batteryOn) {
                    requestUnrestricted(context)
                }
                Step(Icons.Rounded.Notifications, SystemColor.RED, "Notifications on PC", "See, reply to and dismiss them from your PC. You choose which apps", notifOn) {
                    runCatching { context.startActivity(NotificationBridge.settingsIntent(context)) }
                }
                Step(Icons.Rounded.Sms, SystemColor.GREEN, "Calls & messages", "Text from your PC and see who's calling", phoneOn) {
                    askPhone.launch(PhonePermissions.all)
                }
                Step(Icons.Rounded.ContentPaste, SystemColor.ORANGE, "Sync when closed", "Copy here, paste on your PC, even with Vyoma closed", clipOn, action = "Set up") {
                    showClipboard = true
                }
            }
        }

        item(key = "tips") {
            LiquidReveal(paired.isNotEmpty()) {
                Column(Modifier.fillMaxWidth().glassChrome(backdrop, SETUP_SHAPE, blur = 3.dp, bevel = 16.dp).padding(20.dp)) {
                    Text("Good to know", style = MaterialTheme.typography.titleSmall)
                    Spacer(Modifier.height(10.dp))
                    Tip("Tiles with three dots do more when you hold them: hold Send clip for your clipboard history.")
                    Tip("Hold the round touchpad button for the game controller and presenter.")
                    Tip("Everything here can be changed later in Settings.")
                }
            }
        }

        item(key = "done") {
            Column(
                Modifier.fillMaxWidth().padding(top = 8.dp, bottom = 32.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                // One way forward, saying what it does.
                GlassPill(if (paired.isNotEmpty()) "Start using Vyoma" else "Skip for now", backdrop, Modifier.fillMaxWidth(), large = true, onClick = onDone)
            }
        }
    } } }

    if (showClipboard) ClipboardSetupDialog(vm) { showClipboard = false }
    moment?.let { PairedMoment(backdrop, ripple, it) { moment = null } }
}

private val SETUP_SHAPE = RoundedCornerShape(26.dp)

private fun Typography.shadowed() = copy(
    displaySmall = displaySmall.copy(shadow = Soft),
    titleMedium = titleMedium.copy(shadow = Soft),
    titleSmall = titleSmall.copy(shadow = Soft),
    bodyLarge = bodyLarge.copy(shadow = Soft),
    bodyMedium = bodyMedium.copy(shadow = Soft),
    bodySmall = bodySmall.copy(shadow = Soft),
    labelLarge = labelLarge.copy(shadow = Soft),
)

/** A step of the setup: number or tick, what it is, and its content when it's the open one. */
@Composable
private fun SetupCard(backdrop: Backdrop, number: Int, title: String, status: String, done: Boolean, open: Boolean, onOpen: () -> Unit, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().glassChrome(backdrop, SETUP_SHAPE, blur = 3.dp, bevel = 16.dp)) {
        Row(
            Modifier.fillMaxWidth().pressable(haptic = Haptic.Tap, onClick = onOpen).padding(horizontal = 20.dp, vertical = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            val tone by animateColorAsState(
                when {
                    done -> SystemColor.GREEN.color
                    open -> MaterialTheme.colorScheme.primary
                    else -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.14f)
                },
                label = "step",
            )
            Box(Modifier.size(32.dp).clip(CircleShape).background(tone), contentAlignment = Alignment.Center) {
                if (done) Icon(Icons.Rounded.Check, "Done", tint = Color.White, modifier = Modifier.size(20.dp))
                else Text("$number", style = MaterialTheme.typography.titleSmall, color = if (open) Color.White else MaterialTheme.colorScheme.onSurface)
            }
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleSmall)
                Text(status, style = MaterialTheme.typography.bodySmall, color = Color.White)
            }
        }
        LiquidReveal(open) {
            Column(Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, bottom = 20.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) { content() }
        }
    }
}

/** A cue: a sentence that saves the question. */
@Composable
private fun Tip(text: String) {
    Row(Modifier.padding(bottom = 4.dp)) {
        Icon(Icons.Rounded.Lightbulb, null, tint = SystemColor.YELLOW.color, modifier = Modifier.padding(top = 1.dp).size(16.dp))
        Spacer(Modifier.width(8.dp))
        Text(text, style = MaterialTheme.typography.bodySmall, color = Color.White)
    }
}

/** What Vyoma's PC app adds, and how to get it: from here the phone can open its page on the PC itself. */
@Composable
internal fun CompanionSetup(vm: AppViewModel, online: Boolean, pcId: String?) {
    val context = LocalContext.current
    val status = LocalStatus.current
    val scope = rememberCoroutineScope()
    if (online) {
        Tip("It's running there. Pair with it next; its window on the PC walks through the same steps.")
        return
    }
    Text(
        "Vyoma's app for your PC, what this phone pairs with: its screen on this phone, this phone as its webcam and game controller, its stats and power, files both ways, and a window to run it all. It takes over from KDE Connect and keeps its pairings.",
        style = MaterialTheme.typography.bodyMedium,
    )
    Text("Run this on your PC. It installs what it needs and starts by itself:", style = MaterialTheme.typography.bodyMedium)
    Text(PC_APP_INSTALL, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace))
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (pcId != null) PillButton("Open on PC", onClick = {
            scope.launch {
                if (vm.openOnPc(pcId, PC_APP_URL)) status.show("Opened on your PC", haptic = false)
                else status.show("Couldn't reach your PC", failure = true)
            }
        }, prominent = false)
        // No PC paired yet to open it on: send the page to yourself (mail, a chat) and open it there.
        if (pcId == null) PillButton("Send link", onClick = {
            val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, PC_APP_URL)
            runCatching { context.startActivity(Intent.createChooser(send, "Send the link to your PC")) }
        }, prominent = false)
        PillButton("Copy", onClick = {
            context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("install", PC_APP_INSTALL))
            status.show("Copied", haptic = false)
        }, prominent = false)
    }
}

/** One row of what helps: what it gives you, and either a tick or the button that sets it up. */
@Composable
private fun Step(
    icon: ImageVector,
    color: SystemColor,
    title: String,
    subtitle: String,
    done: Boolean,
    action: String = "Allow",
    onClick: () -> Unit,
) {
    ListRow(
        title = title,
        subtitle = subtitle,
        icon = icon,
        iconTint = color.color,
        horizontalPadding = 0.dp,
        trailing = {
            if (done) Icon(Icons.Rounded.CheckCircle, "Done", tint = MaterialTheme.colorScheme.primary)
            else PillButton(action, onClick = onClick, prominent = false)
        },
    )
}
