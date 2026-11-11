package dev.vyoma.connect.ui

import android.content.Context
import android.content.Intent
import androidx.collection.LruCache
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.NotificationsOff
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.VisibilityOff
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toBitmap
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.LocalMenu
import dev.vyoma.connect.designsystem.MenuItem
import dev.vyoma.connect.designsystem.RowChevron
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.cardSegment
import dev.vyoma.connect.designsystem.glassRaised
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.platform.NotifMode
import dev.vyoma.connect.platform.NotificationBridge
import dev.vyoma.connect.platform.NotificationFilters
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** An app that can post notifications, as the list shows it. */
private data class NotifApp(val pkg: String, val name: String)

/** Apps that posted within this long lead the list. */
private const val RECENT_MS = 14L * 24 * 60 * 60 * 1000
private const val RECENT_MAX = 8

/**
 * Settings › Notifications on PC: access to the phone's notifications, then each app's choice (on,
 * text hidden, off), apps that posted lately first, then every app, with a search.
 */
@Composable
fun NotificationAppsScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val filters = remember { NotificationFilters.load(context) }
    val modes by filters.modes.collectAsStateWithLifecycle()
    val recent by filters.recent.collectAsStateWithLifecycle()
    var resumeTick by remember { mutableIntStateOf(0) }
    LifecycleResumeEffect(Unit) {
        resumeTick++
        onPauseOrDispose { }
    }
    val access = remember(resumeTick) { NotificationBridge.isEnabled(context) }
    val installed by produceState(emptyList<NotifApp>()) { value = withContext(Dispatchers.IO) { launcherApps(context) } }
    var query by rememberSaveable { mutableStateOf("") }

    val now = remember { System.currentTimeMillis() }
    val names = remember(installed, recent) { installed.associate { it.pkg to it.name } + recent.mapValues { it.value.second } }
    val lately = remember(recent, names) {
        recent.entries.filter { now - it.value.first < RECENT_MS && it.key != context.packageName }
            .sortedByDescending { it.value.first }.take(RECENT_MAX)
            .map { NotifApp(it.key, names[it.key] ?: it.value.second) }
    }
    // Every app once: the launcher's, and any that posted but has no icon there.
    val everyone = remember(installed, recent, names) {
        (installed.map { it.pkg } + recent.keys).distinct().filter { it != context.packageName }
            .map { NotifApp(it, names[it] ?: it) }.sortedBy { it.name.lowercase() }
    }
    val q = query.trim()
    val found = if (q.isEmpty()) everyone else everyone.filter { it.name.contains(q, ignoreCase = true) }
    val changed = modes.size

    CollapsingHeaderScaffold(
        title = "Notifications",
        subtitle = when {
            !access -> "Access is off"
            changed == 0 -> "All apps on"
            else -> if (changed == 1) "1 app changed" else "$changed apps changed"
        },
        navigation = { BounceIconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
    ) {
        item(key = "access") {
            SectionCard {
                ListRow(
                    title = "Notification access",
                    subtitle = if (access) null else "Needed to show them on your PC",
                    icon = Icons.Rounded.Notifications,
                    iconTint = SystemColor.RED.color,
                    trailing = { RowChevron(if (access) "On" else "Off") },
                    onClick = { runCatching { context.startActivity(NotificationBridge.settingsIntent(context)) } },
                )
            }
        }
        item(key = "search") {
            SearchBox(query, { query = it }, Modifier.padding(top = 4.dp))
        }
        if (q.isEmpty() && lately.isNotEmpty()) {
            item(key = "recent-label") { SectionLabel("Recent") }
            lately.forEachIndexed { i, app ->
                item(key = "r:" + app.pkg) {
                    AppRow(app, modes[app.pkg] ?: NotifMode.SHOW, first = i == 0, last = i == lately.lastIndex)
                }
            }
        }
        item(key = "all-label") { SectionLabel(if (q.isEmpty()) "All apps" else "Results") }
        if (found.isEmpty()) {
            item(key = "none") {
                Text(
                    if (installed.isEmpty() && q.isEmpty()) "Loading apps…" else "No app called \"$q\"",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 12.dp),
                )
            }
        }
        found.forEachIndexed { i, app ->
            item(key = "a:" + app.pkg) {
                AppRow(app, modes[app.pkg] ?: NotifMode.SHOW, first = i == 0, last = i == found.lastIndex)
            }
        }
    }
}

/** One app: its icon and name, its choice in words; tapped, a menu of the three choices. */
@Composable
private fun AppRow(app: NotifApp, mode: NotifMode, first: Boolean, last: Boolean) {
    val context = LocalContext.current
    val menus = LocalMenu.current
    val placed = remember { arrayOfNulls<LayoutCoordinates>(1) }
    val icon = appIcon(app.pkg)
    fun item(m: NotifMode, label: String, glyph: androidx.compose.ui.graphics.vector.ImageVector) =
        MenuItem(label, if (m == mode) Icons.Rounded.Check else glyph) { NotificationFilters.set(context, app.pkg, m) }
    Row(
        Modifier.fillMaxWidth()
            .then(if (!last) Modifier.joinNext() else Modifier)
            .cardSegment(first = first, last = last)
            .onPlaced { placed[0] = it }
            .pressable(haptic = Haptic.Tap) {
                val at = placed[0]?.takeIf { it.isAttached }?.boundsInRoot() ?: return@pressable
                menus.show(at, listOf(
                    item(NotifMode.SHOW, "Show on PC", Icons.Rounded.Notifications),
                    item(NotifMode.PRIVATE, "Hide what it says", Icons.Rounded.VisibilityOff),
                    item(NotifMode.OFF, "Don't send", Icons.Rounded.NotificationsOff),
                ), title = app.name)
            }
            .padding(horizontal = 20.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(36.dp)) {
            if (icon != null) Image(icon, null, Modifier.size(36.dp))
        }
        Spacer(Modifier.width(14.dp))
        Text(
            app.name, Modifier.weight(1f),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurface,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
        Spacer(Modifier.width(12.dp))
        // The texting app's words follow the texts lock: a PC sees them only once Messages is unlocked there.
        val textsHeld = remember(app.pkg, mode) {
            mode == NotifMode.SHOW && dev.vyoma.connect.platform.TextsLock.enabled(context) && NotificationBridge.isTextingApp(context, app.pkg)
        }
        RowChevron(if (textsHeld) "Hidden when locked" else mode.label)
    }
}

@Composable
private fun SearchBox(value: String, onChange: (String) -> Unit, modifier: Modifier = Modifier) {
    val ink = MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier.fillMaxWidth().glassRaised(RoundedCornerShape(50)).padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.Search, null, tint = ink, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(10.dp))
        Box(Modifier.weight(1f)) {
            if (value.isEmpty()) Text("Search apps", style = MaterialTheme.typography.bodyLarge, color = ink)
            BasicTextField(
                value, onChange,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (value.isNotEmpty()) {
            Spacer(Modifier.width(8.dp))
            Icon(
                Icons.Rounded.Close, "Clear search", tint = ink,
                modifier = Modifier.size(20.dp).pressable(haptic = Haptic.Tap) { onChange("") },
            )
        }
    }
}

/** Apps with a launcher icon (what Android lets us list without seeing every package). */
private fun launcherApps(context: Context): List<NotifApp> {
    val pm = context.packageManager
    val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
    return pm.queryIntentActivities(launcher, 0)
        .map { it.activityInfo.packageName to it.loadLabel(pm).toString() }
        .distinctBy { it.first }
        .map { (pkg, name) -> NotifApp(pkg, name) }
}

private val icons = LruCache<String, ImageBitmap>(120)

/** The app's icon, drawn off the main thread once and kept (null until then, or if it has none). */
@Composable
private fun appIcon(pkg: String): ImageBitmap? {
    val context = LocalContext.current
    val size = with(androidx.compose.ui.platform.LocalDensity.current) { 36.dp.roundToPx() }
    return produceState(icons[pkg], pkg) {
        if (value != null) return@produceState
        value = withContext(Dispatchers.IO) {
            runCatching {
                context.packageManager.getApplicationIcon(pkg).toBitmap(size, size).asImageBitmap().also { icons.put(pkg, it) }
            }.getOrNull()
        }
    }.value
}
