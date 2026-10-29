package dev.vyoma.connect.ui

import android.annotation.SuppressLint
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
import androidx.compose.foundation.layout.fillMaxWidth
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
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
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
fun SettingsScreen(vm: AppViewModel, onBack: () -> Unit) {
    val theme by vm.theme.collectAsStateWithLifecycle()
    val name by vm.deviceName.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var editingName by rememberSaveable { mutableStateOf(false) }
    val power = context.getSystemService(PowerManager::class.java)
    val unrestricted = power.isIgnoringBatteryOptimizations(context.packageName)

    CollapsingHeaderScaffold(
        title = "Settings",
        navigation = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
    ) {
        item(key = "device") {
            SectionCard {
                ListRow(
                    title = "Device name",
                    subtitle = name,
                    icon = Icons.Rounded.Edit,
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
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 20.dp)) {
                    DarkMode.entries.forEachIndexed { i, mode ->
                        SegmentedButton(
                            selected = theme.darkMode == mode,
                            onClick = { vm.setDarkMode(mode) },
                            shape = SegmentedButtonDefaults.itemShape(i, DarkMode.entries.size),
                        ) { Text(mode.name.lowercase().replaceFirstChar(Char::uppercase)) }
                    }
                }
                Spacer(Modifier.height(8.dp))
                ListRow(
                    title = "Pure black",
                    subtitle = "True black backgrounds in dark mode, easy on AMOLED",
                    icon = Icons.Rounded.DarkMode,
                    trailing = { Switch(checked = theme.amoled, onCheckedChange = vm::setAmoled) },
                    onClick = { vm.setAmoled(!theme.amoled) },
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
                    iconTint = if (unrestricted) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.primary,
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
                )
            }
        }
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
            Modifier.size(52.dp).clip(CircleShape).border(2.dp, ring, CircleShape).padding(inset),
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
private fun requestUnrestricted(context: android.content.Context) {
    val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${context.packageName}"))
    runCatching { context.startActivity(intent) }
}
