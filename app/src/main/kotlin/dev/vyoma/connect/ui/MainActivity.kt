package dev.vyoma.connect.ui

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.LinkOff
import kotlinx.coroutines.coroutineScope
import androidx.compose.animation.core.Animatable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.EnterExitState
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.MutableState
import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.PredictiveBackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.SeekableTransitionState
import androidx.compose.animation.core.rememberTransition
import androidx.compose.animation.core.tween
import androidx.compose.ui.unit.IntOffset
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import dev.vyoma.connect.designsystem.MenuHost
import dev.vyoma.connect.platform.AppLock
import dev.vyoma.connect.designsystem.StatusHost
import dev.vyoma.connect.container
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import dev.vyoma.connect.BuildConfig
import dev.vyoma.connect.demo.DemoDiagnostics
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.luminance
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.withResumed
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.platform.LocalContext
import android.app.Application
import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.DarkMode
import dev.vyoma.connect.designsystem.Palette
import dev.vyoma.connect.designsystem.VyomaTheme
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val PAGE_SWIPE_MS = 420
private const val PC_FILES = "To open the PC's files"
private const val UNLOCK_SETTLE_MS = 500L

sealed interface Screen {
    /** The tabs (Home, Files, Settings) under the floating bar. */
    data object Home : Screen
    data object Clipboard : Screen
    data object Captures : Screen
    data class DeviceDetail(val id: String) : Screen
    data class Touchpad(val deviceId: String?, val keyboard: Boolean) : Screen
    data class Presenter(val deviceId: String?) : Screen
    data class Files(val pcId: String, val path: String = "") : Screen
    data object Automation : Screen
    data object Connection : Screen
    data object Notifications : Screen
    /** [ruleId] null: a new rule; "suggestion:key": a suggestion as a new rule. */
    data class RuleEditor(val ruleId: String?) : Screen
}

class MainActivity : ComponentActivity() {
    private val vm: AppViewModel by viewModels()
    private val backStack = mutableStateListOf<Screen>(Screen.Home)
    private val tab = mutableStateOf(Tab.HOME)

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT),
        )
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) handleIntent(intent) // a recreated activity already acted on it
        setContent {
            val chosenTheme by vm.theme.collectAsStateWithLifecycle()
            val onboarding by vm.showOnboarding.collectAsStateWithLifecycle()
            // First run is drawn on its own picture: white words, and the sand's accent, whatever's chosen.
            val theme = if (onboarding == true) chosenTheme.copy(darkMode = DarkMode.DARK, palette = Palette.EMBER) else chosenTheme
            VyomaTheme(theme) {
                // Bar icons follow the app's theme, which can differ from the system's.
                val darkUi = MaterialTheme.colorScheme.background.luminance() < 0.5f
                LaunchedEffect(darkUi) {
                    val clear = android.graphics.Color.TRANSPARENT
                    val style = if (darkUi) SystemBarStyle.dark(clear) else SystemBarStyle.light(clear, clear)
                    enableEdgeToEdge(statusBarStyle = style, navigationBarStyle = style)
                }
                StatusHost { PcRefusals(); MenuHost { Crossfade(onboarding, animationSpec = VyomaMotion.effects(), label = "onboarding") { show ->
                    when (show) {
                        true -> OnboardingScreen(vm, onDone = {
                            // Setup ends on Home, even replayed from Settings.
                            tab.value = Tab.HOME
                            vm.finishOnboarding()
                        })
                        false -> {
                            PermissionGate { vm.startService() }
                            AppNavigation(backStack, tab, vm)
                        }
                        // Still reading settings: just the themed background, no flash of Home.
                        null -> Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background))
                    }
                } } }
            }
        }
    }

    /** Ahead of the system's volume handling, for a screen that has claimed the keys. */
    @android.annotation.SuppressLint("RestrictedApi") // Activity's own public method; lint sees androidx.core's override.
    override fun dispatchKeyEvent(event: android.view.KeyEvent): Boolean =
        HardwareKeys.handler?.invoke(event) == true || super.dispatchKeyEvent(event)

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        // Brings the link back if the system or the user stopped it while we were away. Idempotent.
        if (vm.showOnboarding.value == false) vm.startService()
        vm.onForeground()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        vm.clipboardFocus(hasFocus)
        // Focus is what Android wants before it lets us read the clipboard.
        if (hasFocus) vm.onForeground()
    }

    override fun dispatchTouchEvent(ev: android.view.MotionEvent): Boolean {
        vm.touched()
        return super.dispatchTouchEvent(ev)
    }

    override fun onStart() {
        super.onStart()
        // Came back after the phone was locked or left for a while: these are asked for again.
        if (!AppLock.isOpen(this)) {
            backStack.removeAll { it is Screen.Files || it is Screen.Touchpad || it == Screen.Clipboard }
            // Not mid-talk: the page stays and the fingerprint decides. Saying no leaves it.
            if (backStack.lastOrNull() is Screen.Presenter) lifecycleScope.launch {
                delay(UNLOCK_SETTLE_MS) // "Phone unlock counts" hears of the unlock just after this
                withResumed {
                    AppLock.ask(this@MainActivity, AppLock.PRESENTER, onDenied = { backStack.removeAll { it is Screen.Presenter } }) {}
                }
            } else backStack.removeAll { it is Screen.Presenter }
        }
    }

    private fun handleIntent(intent: Intent?) {
        intent ?: return
        if (intent.getStringExtra(EXTRA_SCREEN) == SCREEN_CLIPBOARD) {
            askOnScreen(AppLock.CLIPBOARD) {
                backStack.retainAll { it == Screen.Home }
                tab.value = Tab.FILES
                backStack.add(Screen.Clipboard)
            }
            return
        }
        if (intent.getStringExtra(EXTRA_SCREEN) == SCREEN_TOUCHPAD) {
            val target = Screen.Touchpad(intent.getStringExtra(EXTRA_DEVICE), keyboard = false)
            askOnScreen(AppLock.TOUCHPAD) { if (backStack.lastOrNull() != target) backStack.add(target) }
            return
        }
        val target = intent.getStringExtra(EXTRA_DEVICE)?.let { Screen.DeviceDetail(it) } ?: return
        if (backStack.lastOrNull() != target) backStack.add(target)
    }

    /** These arrive while the activity is still coming up, and Android only asks for an app on screen. */
    private fun askOnScreen(why: String, then: () -> Unit) {
        lifecycleScope.launch { withResumed { AppLock.ask(this@MainActivity, why, then = then) } }
    }

    companion object {
        const val EXTRA_DEVICE = "device"
        const val EXTRA_SCREEN = "screen"
        const val SCREEN_TOUCHPAD = "touchpad"
        const val SCREEN_CLIPBOARD = "clipboard"
    }
}

/** Asks once for what discovery needs, then starts the service either way (it degrades gracefully). */
@Composable
private fun PermissionGate(onReady: () -> Unit) {
    val permissions = remember {
        buildList {
            if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            if (Build.VERSION.SDK_INT >= 37) add("android.permission.ACCESS_LOCAL_NETWORK")
        }.toTypedArray()
    }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { onReady() }
    LaunchedEffect(Unit) { launcher.launch(permissions) }
}

/**
 * Tiny back stack with iOS-style motion: pages push in from the right over a parallaxed,
 * dimmed parent. The back gesture drags the page off under your thumb with the parent already
 * visible beneath, then finishes (or snaps back) from wherever you let go.
 */
@Composable
private fun AppNavigation(backStack: MutableList<Screen>, tabState: MutableState<Tab>, vm: AppViewModel) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val status = LocalStatus.current
    val scope = rememberCoroutineScope()
    var tab by tabState
    val toTab: (Tab) -> Unit = { t -> backStack.retainAll { it == Screen.Home }; tab = t }

    val current = backStack.last()
    if (BuildConfig.DEMO) LaunchedEffect(current, tab) {
        DemoDiagnostics.screen = when (current) {
            Screen.Home -> tab.name.lowercase().replaceFirstChar(Char::uppercase)
            is Screen.DeviceDetail -> "Device"
            is Screen.Touchpad -> if (current.keyboard) "Keyboard" else "Touchpad"
            is Screen.Presenter -> "Presenter"
            is Screen.Files -> "PC files"
            Screen.Clipboard -> "Clipboard"
            Screen.Captures -> "Captures"
            Screen.Automation -> "Rules"
            Screen.Connection -> "Connection"
            Screen.Notifications -> "Notifications"
            is Screen.RuleEditor -> "Rule"
        }
    }
    val nav = remember { SeekableTransitionState<Screen>(current) }
    // Where a screen sits in the stack decides push or pop (a fixed depth per kind of screen slid
    // pages opened from a notification, or Clipboard over a PC page, the wrong way). One already
    // popped counts as the deepest.
    fun order(s: Screen) = backStack.lastIndexOf(s).let { if (it < 0) Int.MAX_VALUE else it }
    // True while a back swipe drives the transition (and while its release animates).
    var swiping by remember { mutableStateOf(false) }
    LaunchedEffect(current) {
        if (!swiping && nav.currentState == nav.targetState && nav.currentState != current) {
            // Composing a whole new page takes ~25 ms (one long frame). Do it parked just off
            // screen at progress 0 and start the slide after: started together, the spring had
            // already run 25 ms by its first visible frame and the page jumped in by a tenth.
            nav.seekTo(0f, current)
            withFrameNanos {}
            withFrameNanos {}
        }
        nav.animateTo(current, if (swiping) VyomaMotion.page() else null)
        swiping = false
    }
    // Each screen's saved state (scroll positions, the open tab's lists) lives here while it's on
    // the back stack, so coming back from a pushed page lands where you left; popped pages drop it.
    val saved = rememberSaveableStateHolder()
    PredictiveBackHandler(enabled = backStack.size > 1) { events ->
        val previous = backStack[backStack.lastIndex - 1]
        try {
            swiping = true
            events.collect { e -> nav.seekTo(e.progress, previous) }
            saved.removeState(backStack.removeAt(backStack.lastIndex).toString())
        } catch (e: CancellationException) {
            scope.launch {
                // Rewind the swipe by seeking back to 0, then settle. animateTo(currentState) turned the
                // half-done pop around into a push, and for a frame or two the page underneath was
                // drawn over this one (the preview "glitching in" after a cancelled back).
                val back = Animatable(nav.fraction)
                coroutineScope {
                    val run = launch { back.animateTo(0f, VyomaMotion.page()) }
                    while (run.isActive) {
                        nav.seekTo(back.value.coerceIn(0f, 1f), previous)
                        withFrameNanos {}
                    }
                }
                nav.snapTo(nav.currentState)
                swiping = false
            }
            throw e
        }
    }

    val pop: () -> Unit = {
        if (backStack.size > 1) saved.removeState(backStack.removeAt(backStack.lastIndex).toString())
    }
    val navTransition = rememberTransition(nav, label = "nav")
    navTransition.AnimatedContent(
        transitionSpec = {
            // A swipe seeks through the animation, so it needs a linear timeline; buttons get springs.
            val slide: FiniteAnimationSpec<IntOffset> = if (swiping) tween(PAGE_SWIPE_MS, easing = LinearEasing) else VyomaMotion.page()
            if (order(targetState) > order(initialState)) {
                slideInHorizontally(slide) { it } togetherWith slideOutHorizontally(slide) { -it / 4 }
            } else {
                slideInHorizontally(slide) { -it / 4 } togetherWith slideOutHorizontally(slide) { it }
            }.apply {
                // The deeper page is always on top. A page keeps the z it entered with, so a fixed -1
                // for going back left a page you'd come back to level with its parent: swiping back
                // from it drew the parent over it, cut off at 3/4 of the width.
                targetContentZIndex = order(targetState).toFloat()
            }
        },
    ) { screen ->
        // The page underneath dims while another slides over it. A drawn shade, not a layer fade:
        // fading would push the whole page (and every glass element replaying it) through an
        // offscreen buffer each frame.
        val below = order(screen) < maxOf(order(navTransition.currentState), order(navTransition.targetState))
        val dim by transition.animateFloat(
            transitionSpec = { if (swiping) tween(PAGE_SWIPE_MS, easing = LinearEasing) else VyomaMotion.effects() },
            label = "dim",
        ) { if (it == EnterExitState.Visible) 0f else 0.4f }
        Box(
            Modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background)
                .drawWithContent {
                    drawContent()
                    if (below && dim > 0.005f) drawRect(Color.Black, alpha = dim)
                },
        ) { saved.SaveableStateProvider(screen.toString()) {
            when (screen) {
                Screen.Home -> TabsRoot(
                    tab = tab,
                    onTab = toTab,
                    // At the root, back on another tab goes Home first, like One UI apps.
                    backToHome = backStack.size == 1,
                    onTouchpad = {
                        val away = vm.awayReason()
                        if (away != null) status.show(away, icon = Icons.Rounded.LinkOff)
                        else AppLock.ask(context, AppLock.TOUCHPAD) { backStack.add(Screen.Touchpad(null, keyboard = false)) }
                    },
                    onGamepad = { GamepadActivity.open(context, null) },
                    onPresenter = { AppLock.ask(context, AppLock.PRESENTER) { backStack.add(Screen.Presenter(null)) } },
                ) { t ->
                    when (t) {
                        Tab.HOME -> HomeScreen(
                            vm = vm,
                            onOpenDevice = { backStack.add(Screen.DeviceDetail(it)) },
                            onTouchpad = { id -> AppLock.ask(context, AppLock.TOUCHPAD) { backStack.add(Screen.Touchpad(id, keyboard = false)) } },
                            onPresenter = { id -> AppLock.ask(context, AppLock.PRESENTER) { backStack.add(Screen.Presenter(id)) } },
                            onClipboard = { AppLock.ask(context, AppLock.CLIPBOARD) { backStack.add(Screen.Clipboard) } },
                            onBrowseFiles = { id -> AppLock.ask(context, PC_FILES) { backStack.add(Screen.Files(id)) } },
                            onFiles = { toTab(Tab.FILES) },
                            onCaptures = { backStack.add(Screen.Captures) },
                        )
                        Tab.FILES -> FilesHubScreen(
                            vm = vm,
                            onBrowse = { id, path -> AppLock.ask(context, PC_FILES) { backStack.add(Screen.Files(id, path)) } },
                            onClipboard = { AppLock.ask(context, AppLock.CLIPBOARD) { backStack.add(Screen.Clipboard) } },
                            onCaptures = { backStack.add(Screen.Captures) },
                        )
                        Tab.SETTINGS -> SettingsScreen(
                            vm, onAutomation = { backStack.add(Screen.Automation) },
                            onNotifications = { backStack.add(Screen.Notifications) },
                            onConnection = { backStack.add(Screen.Connection) },
                        )
                    }
                }
                is Screen.DeviceDetail -> DeviceScreen(vm, screen.id, onBack = pop)
                is Screen.Touchpad -> RemoteInputScreen(
                    vm, screen.deviceId, screen.keyboard, onBack = pop,
                    onPresenter = { id -> AppLock.ask(context, AppLock.PRESENTER) { backStack.add(Screen.Presenter(id)) } },
                )
                is Screen.Presenter -> PresenterScreen(vm, screen.deviceId, onBack = pop)
                is Screen.Files -> FilesScreen(
                    vm, screen.pcId, screen.path, onBack = pop,
                    onOpen = { backStack.add(Screen.Files(screen.pcId, it)) },
                    onCrumb = { crumb ->
                        // Back through the folders opened on the way down; a crumb above where you came in
                        // replaces them with that folder.
                        fun inside(s: Screen) = s is Screen.Files && s.pcId == screen.pcId && s.path != crumb &&
                            (crumb.isEmpty() || s.path.startsWith("$crumb/"))
                        while (backStack.size > 1 && inside(backStack.last())) saved.removeState(backStack.removeAt(backStack.lastIndex).toString())
                        if (backStack.last() != Screen.Files(screen.pcId, crumb)) backStack.add(Screen.Files(screen.pcId, crumb))
                    },
                )
                Screen.Clipboard -> ClipboardScreen(vm, onBack = pop)
                Screen.Captures -> CapturesScreen(onBack = pop)
                Screen.Notifications -> NotificationAppsScreen(onBack = pop)
                Screen.Connection -> ConnectionChecksScreen(vm, onBack = pop)
                Screen.Automation -> AutomationScreen(vm, onBack = pop, onEdit = { backStack.add(Screen.RuleEditor(it)) })
                is Screen.RuleEditor -> RuleEditorScreen(vm, screen.ruleId, onBack = pop)
            }
        } }
    }
}

/** Shows what the PC refused (VyomaApp passes it on) in the status bubble while this screen is started. */
@Composable
private fun PcRefusals() {
    val status = LocalStatus.current
    val app = LocalContext.current.applicationContext as Application
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(Unit) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            app.container.refusals.collect { status.show(it, failure = true) }
        }
    }
}
