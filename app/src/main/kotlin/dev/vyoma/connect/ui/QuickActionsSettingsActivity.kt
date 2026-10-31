package dev.vyoma.connect.ui

import dev.vyoma.connect.designsystem.DialogButton
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.AddCircle
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.RemoveCircle
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.vectorResource
import androidx.compose.ui.unit.dp
import androidx.glance.appwidget.GlanceAppWidgetManager
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.container
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.designsystem.VyomaTheme
import dev.vyoma.connect.widget.ActionsWidget
import dev.vyoma.connect.widget.QuickAction
import dev.vyoma.connect.widget.QuickActionStore
import kotlinx.coroutines.launch

/**
 * Picks and orders the buttons of one quick-actions widget. One UI opens it from the widget's
 * long-press "Settings"; changes apply to the widget right away.
 */
class QuickActionsSettingsActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val widgetId = intent.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
        if (widgetId == AppWidgetManager.INVALID_APPWIDGET_ID) { finish(); return }
        // The widget works with defaults, so leaving without changes is still a success.
        setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widgetId))

        setContent {
            val theme by application.container.settings.theme.collectAsStateWithLifecycle(ThemeSettings())
            val scope = rememberCoroutineScope()
            var picked by remember { mutableStateOf(QuickActionStore.get(this, widgetId)) }
            fun save(list: List<QuickAction>) {
                picked = list
                QuickActionStore.set(this, widgetId, list)
                scope.launch {
                    runCatching { ActionsWidget().update(this@QuickActionsSettingsActivity, GlanceAppWidgetManager(this@QuickActionsSettingsActivity).getGlanceIdBy(widgetId)) }
                }
            }
            VyomaTheme(theme) {
                CollapsingHeaderScaffold(
                    title = "Quick actions",
                    subtitle = "Pick the buttons and their order. Bigger widgets show more of them.",
                    navigation = { BounceIconButton(onClick = ::finish) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
                    actions = {
                        DialogButton(onClick = { save(QuickAction.DEFAULT) }) { Text("Reset") }
                    },
                ) {
                    item(key = "on-label") { SectionLabel("On the widget · ${picked.size}", Modifier.animateItem()) }
                    item(key = "on") {
                        SectionCard(Modifier.animateItem().padding(top = 8.dp)) {
                            if (picked.isEmpty()) {
                                ListRow(title = "No buttons yet", subtitle = "Add some from below")
                            }
                            picked.forEachIndexed { i, action ->
                                ListRow(
                                    title = action.label,
                                    subtitle = action.description,
                                    icon = ImageVector.vectorResource(action.icon),
                                    showDivider = i < picked.lastIndex,
                                    trailing = {
                                        if (i > 0) {
                                            BounceIconButton(onClick = { save(picked.toMutableList().apply { add(i - 1, removeAt(i)) }) }) {
                                                Icon(Icons.Rounded.KeyboardArrowUp, "Move up")
                                            }
                                        }
                                        BounceIconButton(onClick = { save(picked - action) }) {
                                            Icon(Icons.Rounded.RemoveCircle, "Remove", tint = MaterialTheme.colorScheme.error)
                                        }
                                    },
                                )
                            }
                        }
                    }
                    val more = QuickAction.entries - picked.toSet()
                    if (more.isNotEmpty()) {
                        item(key = "more-label") { SectionLabel("More actions", Modifier.animateItem()) }
                        item(key = "more") {
                            SectionCard(Modifier.animateItem().padding(top = 8.dp), contentPadding = PaddingValues(vertical = 6.dp)) {
                                more.forEachIndexed { i, action ->
                                    ListRow(
                                        title = action.label,
                                        subtitle = action.description,
                                        icon = ImageVector.vectorResource(action.icon),
                                        iconTint = MaterialTheme.colorScheme.onSurfaceVariant,
                                        showDivider = i < more.lastIndex,
                                        trailing = { Icon(Icons.Rounded.AddCircle, "Add", tint = MaterialTheme.colorScheme.primary) },
                                        onClick = { save(picked + action) },
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
