package dev.vyoma.connect.ui

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
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.VyomaTheme
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

sealed interface Screen {
    data object Home : Screen
    data class DeviceDetail(val id: String) : Screen
    data object Settings : Screen
    data class Touchpad(val deviceId: String?, val keyboard: Boolean) : Screen
    data object Clipboard : Screen
}

class MainActivity : ComponentActivity() {
    private val vm: AppViewModel by viewModels()
    private val backStack = mutableStateListOf<Screen>(Screen.Home)

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
                Crossfade(onboarding, animationSpec = VyomaMotion.effects(), label = "onboarding") { show ->
                    when (show) {
                        true -> OnboardingScreen(vm, onDone = { vm.finishOnboarding() })
                        false -> {
                            PermissionGate { vm.startService() }
                            AppNavigation(backStack, vm)
                        }
                        // Still reading settings: just the themed background, no flash of Home.
                        null -> Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background))
                    }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        vm.onForeground()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // Focus is what Android wants before it lets us read the clipboard.
        if (hasFocus) vm.onForeground()
    }

    private fun handleIntent(intent: Intent?) {
        intent ?: return
        val target = when (intent.getStringExtra(EXTRA_SCREEN)) {
            SCREEN_TOUCHPAD -> Screen.Touchpad(intent.getStringExtra(EXTRA_DEVICE), keyboard = false)
            SCREEN_CLIPBOARD -> Screen.Clipboard
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
 * Tiny back stack with One UI-style motion: pushes slide in from the side, and predictive
 * back shrinks and rounds the current page under your thumb before it leaves.
 */
@Composable
private fun AppNavigation(backStack: MutableList<Screen>, vm: AppViewModel) {
    val scope = rememberCoroutineScope()
    val backProgress = remember { Animatable(0f) }

    if (backStack.size > 1) {
        PredictiveBackHandler { events ->
            try {
                events.collect { e -> backProgress.snapTo(e.progress) }
                backStack.removeAt(backStack.lastIndex)
            } catch (e: CancellationException) {
                scope.launch { backProgress.animateTo(0f, VyomaMotion.spatial()) }
                throw e
            }
        }
    }

    val current = backStack.last()
    val pop: () -> Unit = { if (backStack.size > 1) backStack.removeAt(backStack.lastIndex) }
    AnimatedContent(
        targetState = current,
        transitionSpec = {
            // Home is always the root, so anything else is a push.
            if (targetState !is Screen.Home) {
                (slideInHorizontally(VyomaMotion.spatial()) { it / 3 } + fadeIn(VyomaMotion.effects()))
                    .togetherWith(scaleOut(VyomaMotion.effects(), targetScale = 0.94f) + fadeOut(VyomaMotion.effects()))
            } else {
                (scaleIn(VyomaMotion.effects(), initialScale = 0.94f) + fadeIn(VyomaMotion.effects()))
                    .togetherWith(slideOutHorizontally(VyomaMotion.spatial()) { it / 3 } + fadeOut(VyomaMotion.effects()))
            }
        },
        label = "nav",
    ) { screen ->
        LaunchedEffect(screen) { backProgress.snapTo(0f) }
        val isTop = screen == backStack.lastOrNull()
        Box(
            Modifier
                .fillMaxSize()
                .graphicsLayer {
                    if (isTop && backProgress.value > 0f) {
                        val p = backProgress.value
                        val s = 1f - 0.1f * p
                        scaleX = s
                        scaleY = s
                        translationX = 24.dp.toPx() * p
                        shape = RoundedCornerShape(32.dp * p)
                        clip = true
                    }
                },
        ) {
            when (screen) {
                Screen.Home -> HomeScreen(
                    vm = vm,
                    onOpenDevice = { backStack.add(Screen.DeviceDetail(it)) },
                    onOpenSettings = { backStack.add(Screen.Settings) },
                    onTouchpad = { id -> backStack.add(Screen.Touchpad(id, keyboard = false)) },
                    onClipboard = { backStack.add(Screen.Clipboard) },
                )
                is Screen.DeviceDetail -> DeviceScreen(
                    vm, screen.id,
                    onBack = pop,
                    onTouchpad = { kb -> backStack.add(Screen.Touchpad(screen.id, kb)) },
                    onClipboard = { backStack.add(Screen.Clipboard) },
                )
                Screen.Settings -> SettingsScreen(vm, onBack = pop)
                is Screen.Touchpad -> RemoteInputScreen(vm, screen.deviceId, screen.keyboard, onBack = pop)
                Screen.Clipboard -> ClipboardScreen(vm, onBack = pop)
            }
        }
    }
}
