package dev.vyoma.connect.ui

import dev.vyoma.connect.protocol.link.TransportKind
import androidx.compose.material.icons.rounded.Bluetooth
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.compose.rememberLauncherForActivityResult
import android.app.AlarmManager
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.spring
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import dev.vyoma.connect.designsystem.VyomaMotion
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.AlarmOn
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Undo
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TimeInput
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.automation.Action
import dev.vyoma.connect.automation.EVERY_DAY
import dev.vyoma.connect.automation.RingerMode
import dev.vyoma.connect.automation.Rule
import dev.vyoma.connect.automation.Suggestions
import dev.vyoma.connect.automation.Trigger
import dev.vyoma.connect.automation.look
import dev.vyoma.connect.automation.runsOnPcWhenAway
import dev.vyoma.connect.automation.summary
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.DialogButton
import dev.vyoma.connect.designsystem.Glass
import dev.vyoma.connect.designsystem.GlassDialog
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.LiquidSlider
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.RowChevron
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.SegmentedControl
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.VyomaSwitch
import dev.vyoma.connect.designsystem.fadingVerticalScroll
import dev.vyoma.connect.designsystem.pressable
import kotlin.math.roundToInt

/** The rules: yours (each with its switch), ready-made ones to add, and what ran lately. */
@Composable
fun AutomationScreen(vm: AppViewModel, onBack: () -> Unit, onEdit: (String?) -> Unit) {
    val rules by vm.rules.collectAsStateWithLifecycle()
    val log by vm.ruleLog.collectAsStateWithLifecycle()
    val status = LocalStatus.current
    val context = LocalContext.current
    var resumeTick by remember { mutableIntStateOf(0) }
    LifecycleResumeEffect(Unit) {
        resumeTick++
        onPauseOrDispose { }
    }
    val exact = remember(resumeTick) { context.getSystemService(AlarmManager::class.java).canScheduleExactAlarms() }
    val timed = rules.any { r -> r.enabled && (listOf(r.trigger) + r.conditions).any { it is Trigger.Time } }
    val on = rules.count { it.enabled }
    // What the editor sent back (a new rule, a removed rule's suggestion) bubbles in once this page has slid
    // back into place; during the slide it would play out of sight.
    val arrival = remember { vm.ruleArrival.also { vm.ruleArrival = null } }
    var landed by remember { mutableStateOf(arrival == null) }
    LaunchedEffect(Unit) { if (!landed) { kotlinx.coroutines.delay(PAGE_SETTLE_MS); landed = true } }
    val initial = remember { rules.map { it.id }.toSet() - arrival }

    CollapsingHeaderScaffold(
        title = "Automation",
        subtitle = when {
            rules.isEmpty() -> "When this happens, do that"
            on == 0 -> "All rules off"
            else -> "$on ${if (on == 1) "rule" else "rules"} on"
        },
        navigation = { BounceIconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
        actions = { BounceIconButton(onClick = { onEdit(null) }) { Icon(Icons.Rounded.Add, "New rule") } },
    ) {
        // Rules already here when the page opened just show; ones added since bubble in.
        if (rules.isNotEmpty()) {
            item(key = "mine-label") { SectionLabel("Your rules", Modifier.animateItem(placementSpec = null)) }
            item(key = "mine") {
                SectionCard(Modifier.animateItem(placementSpec = null).animateContentSize(VyomaMotion.jelly())) { Column(Modifier.clipToBounds()) {
                    rules.forEachIndexed { i, rule ->
                        val look = rule.trigger.look()
                        key(rule.id) { Bubble(appear = rule.id !in initial, ready = landed) {
                            ListRow(
                                title = rule.name,
                                subtitle = rule.summary(),
                                subtitleMaxLines = 2,
                                icon = look.icon,
                                iconTint = look.color.color,
                                showDivider = i < rules.lastIndex,
                                // Off is a pause, not a removal: it stays put (the switch stays under the
                                // finger) and rests; Remove in the editor gives a suggestion back.
                                resting = !rule.enabled,
                                onClick = { onEdit(rule.id) },
                                trailing = {
                                    VyomaSwitch(
                                        checked = rule.enabled,
                                        onCheckedChange = { vm.setRuleEnabled(rule.id, it) },
                                        contentDescription = rule.name,
                                    )
                                },
                            )
                        } }
                    }
                } }
            }
        }
        if (timed && !exact) {
            item(key = "exact") {
                SectionCard {
                    ListRow(
                        title = "Run timed rules on the minute",
                        subtitle = "Without this, Android may run them a few minutes late",
                        icon = Icons.Rounded.AlarmOn,
                        iconTint = SystemColor.YELLOW.color,
                        trailing = { RowChevron("Allow") },
                        onClick = {
                            context.startActivity(Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, Uri.parse("package:${context.packageName}")))
                        },
                    )
                }
            }
        }
        val added = rules.mapNotNull { it.suggestion }.toSet()
        val offers = Suggestions.all.filter { it.key !in added }
        if (offers.isNotEmpty()) {
            item(key = "ideas-label") { SectionLabel(if (rules.isEmpty()) "Start with one of these" else "Suggestions", Modifier.animateItem(placementSpec = null)) }
            item(key = "ideas") {
                SectionCard(Modifier.animateItem(placementSpec = null).animateContentSize(VyomaMotion.jelly())) { Column(Modifier.clipToBounds()) {
                    // Every suggestion stays composed: an added one shrinks out, a removed rule's bubbles back.
                    val last = Suggestions.all.indexOfLast { it.key !in added }
                    Suggestions.all.forEachIndexed { i, s ->
                        val look = s.rule.trigger.look()
                        BubbleVisibility(visible = s.key !in added && (landed || arrival != SUGGESTION + s.key)) {
                            ListRow(
                                title = s.rule.name,
                                subtitle = s.description,
                                icon = look.icon,
                                iconTint = look.color.color,
                                showDivider = i < last,
                                // The row opens it to look at or change first; the pill adds it as it is.
                                onClick = { onEdit(SUGGESTION + s.key) },
                                trailing = {
                                    PillButton("Add", prominent = false, haptic = Haptic.ToggleOn, onClick = {
                                        vm.saveRule(s.rule.copy(id = java.util.UUID.randomUUID().toString()))
                                        status.show("${s.rule.name} is on", haptic = false)
                                    })
                                },
                            )
                        }
                    }
                } }
            }
        }
        if (log.isNotEmpty()) {
            item(key = "log-label") { SectionLabel("Recent", Modifier.animateItem(placementSpec = null)) }
            item(key = "log") {
                SectionCard(Modifier.animateItem(placementSpec = null).animateContentSize(VyomaMotion.effects())) {
                    val recent = log.take(6)
                    recent.forEachIndexed { i, e ->
                        ListRow(
                            title = e.ruleName,
                            subtitle = "${e.text} · ${ago(e.at)}",
                            subtitleMaxLines = 3,
                            icon = if (e.text.startsWith("Undone") || e.text.startsWith("Switched off")) Icons.Rounded.Undo else if (e.failed) Icons.Rounded.Close else Icons.Rounded.Check,
                            iconTint = if (e.failed) SystemColor.RED.color else SystemColor.GRAY.color,
                            showDivider = i < recent.lastIndex,
                            haptic = null,
                        )
                    }
                }
            }
        }
    }
}

// Rows bubble inside a clip at the card's inner edge: the swell past full size drew over the card's
// edge and the card above. Lists here pass placementSpec = null: the cards grow and shrink on their own springs and push what's
// below them along; a second (placement) spring on the next item lagged and slid it over the growing card.

/** A row that bubbles in when it arrives: it swells past its size on a jelly spring as the card makes room. */
@Composable
private fun Bubble(appear: Boolean, ready: Boolean, content: @Composable () -> Unit) {
    val state = remember { MutableTransitionState(!appear) }
    LaunchedEffect(ready) { if (ready) state.targetState = true }
    AnimatedVisibility(state, enter = bubbleIn, exit = bubbleOut) { content() }
}

@Composable
private fun BubbleVisibility(visible: Boolean, content: @Composable () -> Unit) {
    AnimatedVisibility(visible, enter = bubbleIn, exit = bubbleOut) { content() }
}

private val bubbleIn = expandVertically(spring(dampingRatio = 0.8f, stiffness = 420f), expandFrom = Alignment.Top) +
    scaleIn(spring(dampingRatio = 0.55f, stiffness = 320f), initialScale = 0.6f) + fadeIn(spring(stiffness = 500f))
private val bubbleOut = shrinkVertically(spring(dampingRatio = 1f, stiffness = 500f), shrinkTowards = Alignment.Top) +
    scaleOut(spring(stiffness = 600f), targetScale = 0.7f) + fadeOut(spring(stiffness = 900f))

/** Roughly how long a page takes to slide back into place. */
private const val PAGE_SETTLE_MS = 320L

/** An editor "id" that opens a suggestion as a new rule. */
const val SUGGESTION = "suggestion:"

/** What's being picked or tuned in the editor. */
private sealed interface Slot {
    data object When : Slot
    /** [index] -1 adds one. */
    data class OnlyIf(val index: Int) : Slot
    data class Do(val index: Int) : Slot
}

/**
 * One rule, built in three cards: When, Only if, Do. Saved when you leave the page (if it has a
 * trigger and an action) or with the tick; a new rule only starts running from then, never
 * half-built.
 */
@Composable
fun RuleEditorScreen(vm: AppViewModel, ruleId: String?, onBack: () -> Unit) {
    val status = LocalStatus.current
    val original = remember(ruleId) { ruleId?.takeUnless { it.startsWith(SUGGESTION) }?.let(vm::rule) }
    // A suggestion opened from the list: a new rule, filled in.
    val template = remember(ruleId) {
        ruleId?.takeIf { it.startsWith(SUGGESTION) }?.removePrefix(SUGGESTION)
            ?.let { key -> Suggestions.all.firstOrNull { it.key == key }?.rule?.copy(id = java.util.UUID.randomUUID().toString()) }
    }
    val base = original ?: template
    var name by remember { mutableStateOf(base?.name.orEmpty()) }
    var trigger by remember { mutableStateOf(base?.trigger) }
    var conditions by remember { mutableStateOf(base?.conditions.orEmpty()) }
    var actions by remember { mutableStateOf(base?.actions.orEmpty()) }
    var undo by remember { mutableStateOf(base?.undo ?: true) }
    var picking by remember { mutableStateOf<Slot?>(null) }
    var tuning by remember { mutableStateOf<Pair<Slot, Any>?>(null) }
    var renaming by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }
    var deleted by remember { mutableStateOf(false) }

    val newId = remember { java.util.UUID.randomUUID().toString() }
    fun draft(): Rule? {
        val t = trigger ?: return null
        if (actions.isEmpty()) return null
        return Rule(
            id = base?.id ?: newId,
            name = name.ifBlank { t.look().label },
            enabled = original?.enabled ?: true,
            trigger = t,
            conditions = conditions,
            actions = actions,
            undo = undo,
            suggestion = base?.suggestion,
        )
    }
    val latest by rememberUpdatedState(::draft)
    var saved by remember { mutableStateOf(false) }
    // Leaving saves; nothing to lose with the back gesture.
    DisposableEffect(Unit) {
        onDispose {
            if (deleted || saved) return@onDispose
            val rule = latest() ?: return@onDispose
            // A suggestion only looked at and left as it was isn't added.
            if (rule != base) { if (original == null) vm.ruleArrival = rule.id; vm.saveRule(rule) }
        }
    }
    val away = (trigger as? Trigger.PcLink)?.connected == false
    val valid = trigger != null && actions.isNotEmpty()

    CollapsingHeaderScaffold(
        title = name.ifBlank { trigger?.look()?.label ?: "New rule" },
        subtitle = if (valid) "Saves when you go back" else "Pick when it runs and what it does",
        navigation = { BounceIconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
        actions = {
            if (valid) BounceIconButton(onClick = {
                draft()?.let { if (original == null) vm.ruleArrival = it.id; vm.saveRule(it) }
                saved = true
                status.show("Saved", haptic = false)
                onBack()
            }) { Icon(Icons.Rounded.Check, "Save") }
        },
    ) {
        item(key = "when-label") { SectionLabel("When") }
        item(key = "when") {
            SectionCard {
                val t = trigger
                if (t == null) {
                    ListRow(title = "Choose what starts it", icon = Icons.Rounded.Add, iconTint = SystemColor.BLUE.color, trailing = { RowChevron() }, onClick = { picking = Slot.When })
                } else {
                    val look = t.look()
                    ListRow(
                        title = look.label, subtitle = look.detail, icon = look.icon, iconTint = look.color.color,
                        trailing = { RowChevron(if (tunable(t)) "Edit" else "Change") },
                        onClick = { if (tunable(t)) tuning = Slot.When to t else picking = Slot.When },
                    )
                    if (tunable(t)) ListRow(title = "Something else", icon = Icons.Rounded.Edit, iconTint = SystemColor.GRAY.color, trailing = { RowChevron() }, onClick = { picking = Slot.When })
                }
            }
        }
        item(key = "if-label") { SectionLabel("Only if") }
        item(key = "if") {
            SectionCard {
                conditions.forEachIndexed { i, cnd ->
                    val look = cnd.look()
                    ListRow(
                        title = look.label, subtitle = look.detail, icon = look.icon, iconTint = look.color.color, showDivider = true,
                        onClick = if (tunable(cnd)) ({ tuning = Slot.OnlyIf(i) to cnd }) else null,
                        trailing = { RemoveButton("Remove condition") { conditions = conditions.filterIndexed { j, _ -> j != i } } },
                    )
                }
                ListRow(
                    title = if (conditions.isEmpty()) "Add a condition" else "Add another",
                    subtitle = if (conditions.isEmpty()) "Optional: e.g. only at home, only at night" else null,
                    icon = Icons.Rounded.Add, iconTint = SystemColor.GRAY.color, trailing = { RowChevron() },
                    onClick = { picking = Slot.OnlyIf(-1) },
                )
            }
        }
        item(key = "do-label") { SectionLabel("Do") }
        item(key = "do") {
            SectionCard {
                actions.forEachIndexed { i, a ->
                    val look = a.look()
                    ListRow(
                        title = look.label, subtitle = look.detail?.takeIf { it.isNotBlank() }, icon = look.icon, iconTint = look.color.color, showDivider = true,
                        onClick = if (tunable(a)) ({ tuning = Slot.Do(i) to a }) else null,
                        trailing = { RemoveButton("Remove action") { actions = actions.filterIndexed { j, _ -> j != i } } },
                    )
                }
                ListRow(
                    title = if (actions.isEmpty()) "Add an action" else "Add another",
                    icon = Icons.Rounded.Add, iconTint = SystemColor.BLUE.color, trailing = { RowChevron() },
                    onClick = { picking = Slot.Do(-1) },
                )
            }
        }
        if (away) {
            item(key = "away-note") {
                Text(
                    "Your phone can't reach the PC once it's gone, so the PC does these itself through the companion: " +
                        "lock, pause and mute. It waits until it hasn't heard from the phone for about half a minute." +
                        if (!vm.hasCompanion()) " Install the companion on your PC for these to work." else "",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 20.dp),
                )
            }
        }
        item(key = "options") {
            SectionCard(Modifier.padding(top = 8.dp)) {
                val canUndo = trigger?.ends != false && actions.any { it.undoable }
                if (trigger?.ends != false) {
                    ListRow(
                        title = "Undo when it ends",
                        subtitle = if (canUndo || actions.isEmpty()) "Put things back as they were" else "None of these actions can be put back",
                        icon = Icons.Rounded.Undo,
                        iconTint = SystemColor.TEAL.color,
                        enabled = canUndo || actions.isEmpty(),
                        haptic = if (undo) Haptic.ToggleOff else Haptic.ToggleOn,
                        onClick = { undo = !undo },
                        trailing = { VyomaSwitch(checked = undo && (canUndo || actions.isEmpty()), onCheckedChange = { undo = it }, contentDescription = "Undo when it ends") },
                        showDivider = true,
                    )
                }
                ListRow(
                    title = "Name", subtitle = name.ifBlank { trigger?.look()?.label ?: "Not named yet" },
                    icon = Icons.Rounded.Edit, iconTint = SystemColor.BLUE.color, trailing = { RowChevron() },
                    onClick = { renaming = true }, showDivider = valid,
                )
                if (valid) ListRow(
                    title = "Test now", subtitle = "Runs the actions once, right away",
                    icon = Icons.Rounded.PlayArrow, iconTint = SystemColor.GREEN.color,
                    onClick = {
                        draft()?.let(vm::testRule)
                        status.show("Running ${actions.size} ${if (actions.size == 1) "action" else "actions"}", haptic = false)
                    },
                )
            }
        }
        if (original != null) {
            item(key = "delete") {
                SectionCard(Modifier.padding(top = 8.dp)) {
                    ListRow(
                        title = if (original.suggestion != null) "Remove" else "Delete rule",
                        subtitle = if (original.suggestion != null) "It goes back to Suggestions" else null,
                        icon = Icons.Rounded.Delete, iconTint = SystemColor.RED.color, onClick = { deleting = true },
                    )
                }
            }
        }
    }

    picking?.let { slot ->
        PickerDialog(
            vm = vm,
            slot = slot,
            away = away,
            onDismiss = { picking = null },
            onPick = { choice ->
                picking = null
                // Settings first where the choice has them, else straight in.
                if (tunable(choice)) tuning = slot to choice else apply(slot, choice, { trigger = it }, { conditions = it }, { actions = it }, conditions, actions)
            },
        )
    }
    tuning?.let { (slot, value) ->
        TuneDialog(
            vm = vm,
            value = value,
            onDismiss = { tuning = null },
            onDone = { tuned ->
                tuning = null
                apply(slot, tuned, { trigger = it }, { conditions = it }, { actions = it }, conditions, actions)
            },
        )
    }
    if (renaming) {
        var draftName by remember { mutableStateOf(name) }
        GlassDialog(
            onDismissRequest = { renaming = false },
            title = { Text("Name") },
            text = { OutlinedTextField(draftName, { draftName = it }, singleLine = true, placeholder = { Text(trigger?.look()?.label ?: "My rule") }) },
            confirmButton = { DialogButton(onClick = { name = draftName.trim(); renaming = false }) { Text("Done") } },
            dismissButton = { DialogButton(onClick = { renaming = false }) { Text("Cancel") } },
        )
    }
    if (deleting && original != null) {
        GlassDialog(
            onDismissRequest = { deleting = false },
            title = { Text(if (original.suggestion != null) "Remove ${original.name}?" else "Delete ${original.name}?") },
            text = { Text("If it's running, what it changed is put back first.") },
            confirmButton = {
                DialogButton(destructive = true, onClick = {
                    deleted = true
                    deleting = false
                    original.suggestion?.let { vm.ruleArrival = SUGGESTION + it }
                    vm.deleteRule(original.id)
                    onBack()
                }) { Text(if (original.suggestion != null) "Remove" else "Delete") }
            },
            dismissButton = { DialogButton(onClick = { deleting = false }) { Text("Cancel") } },
        )
    }
}

private fun apply(
    slot: Slot, value: Any,
    setTrigger: (Trigger) -> Unit, setConditions: (List<Trigger>) -> Unit, setActions: (List<Action>) -> Unit,
    conditions: List<Trigger>, actions: List<Action>,
) {
    when (slot) {
        Slot.When -> setTrigger(value as Trigger)
        is Slot.OnlyIf -> setConditions(if (slot.index < 0) conditions + (value as Trigger) else conditions.mapIndexed { i, c -> if (i == slot.index) value as Trigger else c })
        is Slot.Do -> setActions(if (slot.index < 0) actions + (value as Action) else actions.mapIndexed { i, a -> if (i == slot.index) value as Action else a })
    }
}

/** Choices with a setting of their own (a level, a time, words, which command). */
private fun tunable(x: Any): Boolean = when (x) {
    is Trigger.PcLink -> !x.connected
    is Trigger.PhoneBatteryBelow, is Trigger.PcBatteryBelow, is Trigger.Time -> true
    is Trigger.BluetoothDevice, is Trigger.PcVia, is Trigger.WifiNetwork -> true
    is Action.PcVolume, is Action.PcBrightness, is Action.MessagePc, is Action.NotifyMe, is Action.RunCommand -> true
    else -> false
}

@Composable
private fun RemoveButton(label: String, onClick: () -> Unit) {
    BounceIconButton(onClick = onClick) {
        Icon(Icons.Rounded.Close, label, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp))
    }
}

private class Group(val title: String, val items: List<Any>)

private fun triggerGroups(condition: Boolean) = listOf(
    Group("PC", listOfNotNull(
        Trigger.PcLink(connected = true), Trigger.PcLink(connected = false, afterMin = 1).takeIf { !condition },
        Trigger.PcLocked(locked = true), Trigger.PcLocked(locked = false),
        Trigger.PcPlaying(playing = true), Trigger.PcPlaying(playing = false),
        Trigger.PcBatteryBelow(20), Trigger.PcCharging(charging = true), Trigger.PcCharging(charging = false),
    )),
    Group("Phone", listOf(
        Trigger.Call(ringingOnly = false), Trigger.Call(ringingOnly = true),
        Trigger.PhoneCharging(charging = true), Trigger.PhoneCharging(charging = false), Trigger.PhoneBatteryBelow(20),
        Trigger.Headphones(connected = true), Trigger.Headphones(connected = false),
    )),
    Group("Connections", listOf(
        Trigger.BluetoothDevice("", "", connected = true),
        Trigger.PcVia(TransportKind.USB),
        Trigger.WifiNetwork("", "", on = true),
    )),
    Group("Time and place", listOf(
        // A condition is a stretch of time; "at a moment" only starts things.
        if (condition) Trigger.Time(22 * 60, 7 * 60) else Trigger.Time(22 * 60, null),
        Trigger.HomeWifi(home = true), Trigger.HomeWifi(home = false),
    )),
)

private fun actionGroups(away: Boolean) = listOf(
    Group(if (away) "On the PC, by itself" else "On the PC", listOf(
        Action.LockPc, Action.MutePc(mute = true), Action.MutePc(mute = false), Action.PcVolume(50),
        Action.PcMedia(play = false), Action.PcMedia(play = true), Action.PcBrightness(50),
        Action.RunCommand("", ""), Action.SendClipboard, Action.MessagePc(""),
        Action.PcScreenshot, Action.WakePcScreen, Action.SleepPc,
    ).filter { !away || it.runsOnPcWhenAway() }),
    Group("On this phone", listOf(
        Action.NotifyMe(""), Action.PhoneRinger(RingerMode.VIBRATE), Action.PhoneRinger(RingerMode.NORMAL), Action.PausePhoneMedia,
    )),
)

@Composable
private fun PickerDialog(vm: AppViewModel, slot: Slot, away: Boolean, onDismiss: () -> Unit, onPick: (Any) -> Unit) {
    val groups = when (slot) {
        Slot.When -> triggerGroups(condition = false)
        is Slot.OnlyIf -> triggerGroups(condition = true)
        is Slot.Do -> actionGroups(away)
    }
    val companion = remember { vm.hasCompanion() }
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text(when (slot) { Slot.When -> "When"; is Slot.OnlyIf -> "Only if"; is Slot.Do -> "Do" }) },
        text = {
            Column(Modifier.heightIn(max = 440.dp).fadingVerticalScroll(rememberScrollState())) {
                groups.forEach { g ->
                    Text(
                        g.title, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 12.dp, bottom = 2.dp),
                    )
                    g.items.forEachIndexed { i, item ->
                        val look = when (item) { is Trigger -> item.look(); else -> (item as Action).look() }
                        val needsCompanion = (item as? Action)?.companion == true && !companion
                        val label = when (item) {
                            is Action.RunCommand -> "Run a PC command"
                            is Action.MessagePc, is Action.NotifyMe -> look.label
                            is Trigger.PcLink -> if (item.connected) look.label else "PC goes away"
                            is Trigger.Time -> if (item.end == null) "At a time" else "During a time of day"
                            is Trigger.PcBatteryBelow -> "PC battery is low"
                            is Trigger.PhoneBatteryBelow -> "Phone battery is low"
                            is Trigger.BluetoothDevice -> "A Bluetooth device"
                            is Trigger.PcVia -> "How the PC is connected"
                            is Trigger.WifiNetwork -> "A Wi-Fi network"
                            is Action.PcVolume -> "Set PC volume"
                            is Action.PcBrightness -> "Set PC brightness"
                            else -> look.label
                        }
                        ListRow(
                            title = label,
                            subtitle = if (needsCompanion) "Needs the companion on your PC" else null,
                            icon = look.icon,
                            iconTint = look.color.color,
                            horizontalPadding = 0.dp,
                            showDivider = i < g.items.lastIndex,
                            enabled = !needsCompanion,
                            haptic = Haptic.Select,
                            onClick = { onPick(item) },
                        )
                    }
                }
            }
        },
        confirmButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** The setting for one choice: a level, how long, a time, words or a command. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TuneDialog(vm: AppViewModel, value: Any, onDismiss: () -> Unit, onDone: (Any) -> Unit) {
    when (value) {
        is Trigger.PhoneBatteryBelow -> PercentDialog("Phone battery below", value.percent, 5..95, onDismiss) { onDone(Trigger.PhoneBatteryBelow(it)) }
        is Trigger.PcBatteryBelow -> PercentDialog("PC battery below", value.percent, 5..95, onDismiss) { onDone(Trigger.PcBatteryBelow(it)) }
        is Action.PcVolume -> PercentDialog("PC volume", value.percent, 0..100, onDismiss) { onDone(Action.PcVolume(it)) }
        is Action.PcBrightness -> PercentDialog("PC brightness", value.percent, 5..100, onDismiss) { onDone(Action.PcBrightness(it)) }
        is Trigger.PcLink -> {
            val options = listOf(0, 1, 5, 15)
            var pick by remember { mutableIntStateOf(options.indexOf(value.afterMin).coerceAtLeast(0)) }
            GlassDialog(
                onDismissRequest = onDismiss,
                title = { Text("PC goes away") },
                text = {
                    Column {
                        Text("How long the PC has to be out of reach first. A short wait skips Wi-Fi hiccups.", style = MaterialTheme.typography.bodyMedium)
                        Spacer(Modifier.height(16.dp))
                        SegmentedControl(listOf("At once", "1 min", "5 min", "15 min"), pick, { pick = it })
                    }
                },
                confirmButton = { DialogButton(onClick = { onDone(Trigger.PcLink(connected = false, afterMin = options[pick])) }) { Text("Done") } },
                dismissButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
            )
        }
        is Trigger.BluetoothDevice -> BluetoothDialog(vm, value, onDismiss) { onDone(it) }
        is Trigger.PcVia -> {
            val routes = listOf(TransportKind.USB, TransportKind.BLUETOOTH, TransportKind.LAN)
            var pick by remember { mutableIntStateOf(routes.indexOf(value.via)) }
            GlassDialog(
                onDismissRequest = onDismiss,
                title = { Text("PC connected by") },
                text = {
                    Column {
                        Text("When it can use more than one, the cable counts first, then Wi-Fi.", style = MaterialTheme.typography.bodyMedium)
                        Spacer(Modifier.height(16.dp))
                        SegmentedControl(listOf("Cable", "Bluetooth", "Wi-Fi"), pick, { pick = it })
                    }
                },
                confirmButton = { DialogButton(onClick = { onDone(Trigger.PcVia(routes[pick])) }) { Text("Done") } },
                dismissButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
            )
        }
        is Trigger.WifiNetwork -> WifiDialog(vm, value, onDismiss) { onDone(it) }
        is Trigger.Time -> {
            var period by remember { mutableStateOf(value.end != null) }
            var days by remember { mutableIntStateOf(value.days) }
            val start = rememberTimePickerState(value.start / 60, value.start % 60, is24Hour = true)
            val end = rememberTimePickerState((value.end ?: (value.start + 8 * 60) % (24 * 60)) / 60, (value.end ?: value.start) % 60, is24Hour = true)
            GlassDialog(
                onDismissRequest = onDismiss,
                title = { Text("Time") },
                text = {
                    // Outside the scrolling part: its clip cut off the drop as it lifted.
                    Column {
                    SegmentedControl(listOf("At a time", "From – to"), if (period) 1 else 0, { period = it == 1 })
                    Spacer(Modifier.height(16.dp))
                    Column(Modifier.heightIn(max = 420.dp).fadingVerticalScroll(rememberScrollState())) {
                        Text(if (period) "From" else "At", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        TimeInput(start)
                        if (period) {
                            Text("To", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            TimeInput(end)
                        }
                        Text("On", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.height(8.dp))
                        DayChips(days) { days = it }
                    }
                    }
                },
                confirmButton = {
                    DialogButton(enabled = days != 0, onClick = {
                        onDone(Trigger.Time(start.hour * 60 + start.minute, if (period) end.hour * 60 + end.minute else null, days))
                    }) { Text("Done") }
                },
                dismissButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
            )
        }
        is Action.MessagePc, is Action.NotifyMe -> {
            val onPc = value is Action.MessagePc
            var text by remember { mutableStateOf(if (value is Action.MessagePc) value.text else (value as Action.NotifyMe).text) }
            GlassDialog(
                onDismissRequest = onDismiss,
                title = { Text(if (onPc) "Message on PC" else "Notify me") },
                text = {
                    Column {
                        Text(
                            if (onPc) "Shows as a notification on the PC." else "Shows as a notification on this phone.",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                        Spacer(Modifier.height(12.dp))
                        OutlinedTextField(text, { text = it }, placeholder = { Text("Leave empty to use the rule's name") }, maxLines = 4)
                    }
                },
                confirmButton = { DialogButton(onClick = { onDone(if (onPc) Action.MessagePc(text.trim()) else Action.NotifyMe(text.trim())) }) { Text("Done") } },
                dismissButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
            )
        }
        is Action.RunCommand -> {
            val commands = remember { vm.pcCommands() }
            GlassDialog(
                onDismissRequest = onDismiss,
                title = { Text("Run a PC command") },
                text = {
                    if (commands.isEmpty()) {
                        Text("Connect to your PC and add commands there first (PC details › Commands).")
                    } else {
                        Column(Modifier.heightIn(max = 360.dp).fadingVerticalScroll(rememberScrollState())) {
                            commands.forEachIndexed { i, cmd ->
                                ListRow(
                                    title = cmd.name,
                                    icon = Action.RunCommand("", "").look().icon,
                                    iconTint = SystemColor.GRAY.color,
                                    horizontalPadding = 0.dp,
                                    showDivider = i < commands.lastIndex,
                                    haptic = Haptic.Select,
                                    trailing = if (cmd.key == value.key) ({ Icon(Icons.Rounded.Check, "Chosen") }) else null,
                                    onClick = { onDone(Action.RunCommand(cmd.key, cmd.name)) },
                                )
                            }
                        }
                    }
                },
                confirmButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
            )
        }
        else -> LaunchedEffect(Unit) { onDone(value) }
    }
}

/** Which paired device, and whether it connecting or disconnecting counts. */
@Composable
private fun BluetoothDialog(vm: AppViewModel, value: Trigger.BluetoothDevice, onDismiss: () -> Unit, onDone: (Trigger) -> Unit) {
    val context = LocalContext.current
    var devices by remember { mutableStateOf(vm.pairedBluetooth()) }
    var blocked by remember { mutableStateOf(vm.needsBluetoothPermission(context)) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        blocked = vm.needsBluetoothPermission(context)
        devices = vm.pairedBluetooth()
    }
    var address by remember { mutableStateOf(value.address) }
    var connected by remember { mutableStateOf(value.connected) }
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text("Bluetooth device") },
        text = {
            Column {
                SegmentedControl(listOf("Connects", "Disconnects"), if (connected) 0 else 1, { connected = it == 0 })
                Spacer(Modifier.height(12.dp))
                when {
                    blocked -> Text("Vyoma needs the Nearby devices permission to see your Bluetooth devices.", style = MaterialTheme.typography.bodyMedium)
                    devices.isEmpty() -> Text("Nothing is paired with this phone yet. Pair the device in Android's Bluetooth settings first.", style = MaterialTheme.typography.bodyMedium)
                    else -> Column(Modifier.heightIn(max = 320.dp).fadingVerticalScroll(rememberScrollState())) {
                        devices.forEachIndexed { i, (a, name) ->
                            ListRow(
                                title = name,
                                icon = Icons.Rounded.Bluetooth,
                                iconTint = SystemColor.BLUE.color,
                                horizontalPadding = 0.dp,
                                showDivider = i < devices.lastIndex,
                                haptic = Haptic.Select,
                                trailing = if (a == address) ({ Icon(Icons.Rounded.Check, "Chosen") }) else null,
                                onClick = { address = a },
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            if (blocked) DialogButton(onClick = { ask.launch(android.Manifest.permission.BLUETOOTH_CONNECT) }) { Text("Allow") }
            else DialogButton(enabled = address.isNotEmpty(), onClick = {
                // A device since unpaired keeps the name it was picked by.
                val name = devices.firstOrNull { it.first == address }?.second ?: value.name
                onDone(Trigger.BluetoothDevice(address, name, connected))
            }) { Text("Done") }
        },
        dismissButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * One Wi-Fi network, added while on it: reading a network's real name needs the location permission,
 * so it's recognised as home Wi-Fi is, and the user names it.
 */
@Composable
private fun WifiDialog(vm: AppViewModel, value: Trigger.WifiNetwork, onDismiss: () -> Unit, onDone: (Trigger) -> Unit) {
    val print = remember { value.print.ifEmpty { vm.currentWifi().orEmpty() } }
    var name by remember { mutableStateOf(value.name) }
    var on by remember { mutableStateOf(value.on) }
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text("Wi-Fi network") },
        text = {
            if (print.isEmpty()) {
                Text("Connect this phone to the Wi-Fi you want, then add it here.", style = MaterialTheme.typography.bodyMedium)
            } else Column {
                SegmentedControl(listOf("On it", "Off it"), if (on) 0 else 1, { on = it == 0 })
                Spacer(Modifier.height(12.dp))
                Text(
                    if (value.print.isEmpty()) "Uses the network you're on now, recognised by its address and router, so no location permission."
                    else "Recognised by its address and router, so no location permission.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(name, { name = it }, singleLine = true, label = { Text("Name") }, placeholder = { Text("Office") })
            }
        },
        confirmButton = {
            if (print.isEmpty()) DialogButton(onClick = onDismiss) { Text("OK") }
            else DialogButton(enabled = name.isNotBlank(), onClick = { onDone(Trigger.WifiNetwork(print, name.trim(), on)) }) { Text("Done") }
        },
        dismissButton = if (print.isEmpty()) null else ({ DialogButton(onClick = onDismiss) { Text("Cancel") } }),
    )
}

@Composable
private fun PercentDialog(title: String, initial: Int, range: IntRange, onDismiss: () -> Unit, onDone: (Int) -> Unit) {
    var percent by remember { mutableIntStateOf(initial.coerceIn(range)) }
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    "$percent%", style = MaterialTheme.typography.displaySmall.copy(fontWeight = FontWeight.SemiBold),
                    textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(16.dp))
                LiquidSlider(
                    value = { (percent - range.first).toFloat() / (range.last - range.first) },
                    onChange = { f -> percent = (range.first + f * (range.last - range.first)).roundToInt().let { (it + 2) / 5 * 5 }.coerceIn(range) },
                    onDone = {},
                    fill = MaterialTheme.colorScheme.primary,
                    track = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.15f),
                    modifier = Modifier.fillMaxWidth().height(44.dp),
                )
            }
        },
        confirmButton = { DialogButton(onClick = { onDone(percent) }) { Text("Done") } },
        dismissButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun DayChips(days: Int, onChange: (Int) -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        listOf("M", "T", "W", "T", "F", "S", "S").forEachIndexed { i, d ->
            val on = days and (1 shl i) != 0
            Box(
                Modifier
                    .size(36.dp)
                    .pressable(haptic = if (on) Haptic.ToggleOff else Haptic.ToggleOn) { onChange(days xor (1 shl i)) }
                    .background(if (on) Glass.accent else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f), CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Text(d, style = MaterialTheme.typography.labelLarge, color = if (on) Color.White else MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
    if (days == 0) Text("Pick at least one day", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(top = 6.dp))
    else if (days != EVERY_DAY) Spacer(Modifier.height(0.dp))
}
