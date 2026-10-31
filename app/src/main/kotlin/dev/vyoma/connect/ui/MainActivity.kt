package dev.vyoma.connect.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.EnterExitState
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.MutableState
import androidx.activity.compose.BackHandler
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
import dev.vyoma.connect.designsystem.StatusHost
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import dev.vyoma.connect.BuildConfig
import dev.vyoma.connect.demo.DemoDiagnostics
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.luminance
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.VyomaTheme
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

private const val PAGE_SWIPE_MS = 420

sealed interface Screen {
    /** The tabs (Home, Files, Settings) under the floating bar. */
    data object Home : Screen
    data object Clipboard : Screen
    data object Captures : Screen
    data class DeviceDetail(val id: String) : Screen
    data class Touchpad(val deviceId: String?, val keyboard: Boolean) : Screen
    data class Files(val pcId: String, val path: String = "") : Screen
}

/** How deep a screen sits; deeper means the transition is a push. */
private val Screen.depth get() = when (this) {
    Screen.Home -> 0
    is Screen.DeviceDetail -> 1
    is Screen.Touchpad -> 2
    is Screen.Files -> 2
    Screen.Clipboard -> 1
    Screen.Captures -> 1
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
        handleIntent(intent)
        setContent {
            val theme by vm.theme.collectAsStateWithLifecycle()
            VyomaTheme(theme) {
                // Bar icons follow the app's theme, which can differ from the system's.
                val darkUi = MaterialTheme.colorScheme.background.luminance() < 0.5f
                LaunchedEffect(darkUi) {
                    val clear = android.graphics.Color.TRANSPARENT
                    val style = if (darkUi) SystemBarStyle.dark(clear) else SystemBarStyle.light(clear, clear)
                    enableEdgeToEdge(statusBarStyle = style, navigationBarStyle = style)
                }
                val onboarding by vm.showOnboarding.collectAsStateWithLifecycle()
                StatusHost { MenuHost { Crossfade(onboarding, animationSpec = VyomaMotion.effects(), label = "onboarding") { show ->
                    when (show) {
                        true -> OnboardingScreen(vm, onDone = { vm.finishOnboarding() })
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
        // Focus is what Android wants before it lets us read the clipboard.
        if (hasFocus) vm.onForeground()
    }

    private fun handleIntent(intent: Intent?) {
        intent ?: return
        if (intent.getStringExtra(EXTRA_SCREEN) == SCREEN_CLIPBOARD) {
            backStack.retainAll { it == Screen.Home }
            tab.value = Tab.FILES
            backStack.add(Screen.Clipboard)
            return
        }
        val target = when (intent.getStringExtra(EXTRA_SCREEN)) {
            SCREEN_TOUCHPAD -> Screen.Touchpad(intent.getStringExtra(EXTRA_DEVICE), keyboard = false)
            else -> intent.getStringExtra(EXTRA_DEVICE)?.let { Screen.DeviceDetail(it) }
        } ?: return
        if (backStack.lastOrNull() != target) backStack.add(target)
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
            add(Manifest.permission.POST_NOTIFICATIONS)
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
    val scope = rememberCoroutineScope()
    var tab by tabState
    val toTab: (Tab) -> Unit = { t -> backStack.retainAll { it == Screen.Home }; tab = t }

    // At the root, back on another tab goes Home first, like One UI apps.
    BackHandler(enabled = backStack.size == 1 && tab != Tab.HOME) { tab = Tab.HOME }

    val current = backStack.last()
    if (BuildConfig.DEMO) LaunchedEffect(current, tab) {
        DemoDiagnostics.screen = when (current) {
            Screen.Home -> tab.name.lowercase().replaceFirstChar(Char::uppercase)
            is Screen.DeviceDetail -> "Device"
            is Screen.Touchpad -> if (current.keyboard) "Keyboard" else "Touchpad"
            is Screen.Files -> "PC files"
            Screen.Clipboard -> "Clipboard"
            Screen.Captures -> "Captures"
        }
    }
    val nav = remember { SeekableTransitionState<Screen>(current) }
    // True while a back swipe drives the transition (and while its release animates).
    var swiping by remember { mutableStateOf(false) }
    LaunchedEffect(current) {
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
                nav.animateTo(nav.currentState, VyomaMotion.page())
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
            if (targetState.depth > initialState.depth) {
                slideInHorizontally(slide) { it } togetherWith slideOutHorizontally(slide) { -it / 4 }
            } else {
                (slideInHorizontally(slide) { -it / 4 } togetherWith slideOutHorizontally(slide) { it }).apply { targetContentZIndex = -1f }
            }
        },
    ) { screen ->
        // The page underneath dims while another slides over it. A drawn shade, not a layer fade:
        // fading would push the whole page (and every glass element replaying it) through an
        // offscreen buffer each frame.
        val below = screen.depth < maxOf(navTransition.currentState.depth, navTransition.targetState.depth)
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
                    onTouchpad = { backStack.add(Screen.Touchpad(null, keyboard = false)) },
                ) { t ->
                    when (t) {
                        Tab.HOME -> HomeScreen(
                            vm = vm,
                            onOpenDevice = { backStack.add(Screen.DeviceDetail(it)) },
                            onKeyboard = { id -> backStack.add(Screen.Touchpad(id, keyboard = true)) },
                            onClipboard = { backStack.add(Screen.Clipboard) },
                            onBrowseFiles = { id -> backStack.add(Screen.Files(id)) },
                            onFiles = { toTab(Tab.FILES) },
                            onCaptures = { backStack.add(Screen.Captures) },
                        )
                        Tab.FILES -> FilesHubScreen(
                            vm = vm,
                            onBrowse = { id, path -> backStack.add(Screen.Files(id, path)) },
                            onClipboard = { backStack.add(Screen.Clipboard) },
                            onCaptures = { backStack.add(Screen.Captures) },
                        )
                        Tab.SETTINGS -> SettingsScreen(vm)
                    }
                }
                is Screen.DeviceDetail -> DeviceScreen(vm, screen.id, onBack = pop)
                is Screen.Touchpad -> RemoteInputScreen(vm, screen.deviceId, screen.keyboard, onBack = pop)
                is Screen.Files -> FilesScreen(vm, screen.pcId, onBack = pop, initialPath = screen.path)
                Screen.Clipboard -> ClipboardScreen(vm, onBack = pop)
                Screen.Captures -> CapturesScreen(onBack = pop)
            }
        } }
    }
}
