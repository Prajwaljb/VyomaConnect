package dev.vyoma.connect.automation

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.BatteryAlert
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.BrightnessMedium
import androidx.compose.material.icons.rounded.Call
import androidx.compose.material.icons.rounded.Chat
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.DesktopWindows
import androidx.compose.material.icons.rounded.Headphones
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.rounded.LinkOff
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.MusicNote
import androidx.compose.material.icons.rounded.MusicOff
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PauseCircle
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Power
import androidx.compose.material.icons.rounded.PowerOff
import androidx.compose.material.icons.rounded.RingVolume
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Screenshot
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Vibration
import androidx.compose.material.icons.rounded.VolumeOff
import androidx.compose.material.icons.rounded.VolumeUp
import androidx.compose.material.icons.rounded.WifiOff
import androidx.compose.ui.graphics.vector.ImageVector
import dev.vyoma.connect.designsystem.SystemColor

/** How a trigger, condition or action is named and drawn. Labels read as states, so they work after "When" and "Only if". */
class Look(val label: String, val icon: ImageVector, val color: SystemColor, val detail: String? = null)

fun Trigger.look(): Look = when (this) {
    is Trigger.PcLink ->
        if (connected) Look("PC is connected", Icons.Rounded.Link, SystemColor.BLUE)
        else Look("PC goes away", Icons.Rounded.LinkOff, SystemColor.GRAY, if (afterMin <= 0) "As soon as it's gone" else "Gone for $afterMin min")
    is Trigger.Call -> if (ringingOnly) Look("Phone is ringing", Icons.Rounded.RingVolume, SystemColor.GREEN) else Look("On a call", Icons.Rounded.Call, SystemColor.GREEN, "Ringing or talking")
    is Trigger.PhoneCharging -> if (charging) Look("Phone is charging", Icons.Rounded.BatteryChargingFull, SystemColor.GREEN) else Look("Phone is unplugged", Icons.Rounded.PowerOff, SystemColor.GRAY)
    is Trigger.PhoneBatteryBelow -> Look("Phone battery below $percent%", Icons.Rounded.BatteryAlert, SystemColor.ORANGE)
    is Trigger.PcBatteryBelow -> Look("PC battery below $percent%", Icons.Rounded.BatteryAlert, SystemColor.ORANGE)
    is Trigger.PcCharging -> if (charging) Look("PC is plugged in", Icons.Rounded.Power, SystemColor.GREEN) else Look("PC is on battery", Icons.Rounded.PowerOff, SystemColor.GRAY)
    is Trigger.PcLocked -> if (locked) Look("PC is locked", Icons.Rounded.Lock, SystemColor.RED) else Look("PC is unlocked", Icons.Rounded.LockOpen, SystemColor.TEAL)
    is Trigger.PcPlaying -> if (playing) Look("PC is playing", Icons.Rounded.MusicNote, SystemColor.PINK) else Look("PC isn't playing", Icons.Rounded.MusicOff, SystemColor.GRAY)
    is Trigger.Headphones -> if (connected) Look("Headphones connected", Icons.Rounded.Headphones, SystemColor.CYAN) else Look("Headphones disconnected", Icons.Rounded.Headphones, SystemColor.GRAY)
    is Trigger.HomeWifi -> if (home) Look("On home Wi-Fi", Icons.Rounded.Home, SystemColor.BLUE, "The Wi-Fi your PC is on") else Look("Away from home Wi-Fi", Icons.Rounded.WifiOff, SystemColor.GRAY)
    is Trigger.Time -> Look(
        if (end == null) "At ${clock(start)}" else "${clock(start)} – ${clock(end)}",
        Icons.Rounded.Schedule, SystemColor.YELLOW, days(days),
    )
}

fun Action.look(): Look = when (this) {
    Action.LockPc -> Look("Lock PC", Icons.Rounded.Lock, SystemColor.RED)
    is Action.MutePc -> if (mute) Look("Mute PC", Icons.Rounded.VolumeOff, SystemColor.PINK) else Look("Unmute PC", Icons.Rounded.VolumeUp, SystemColor.PINK)
    is Action.PcVolume -> Look("PC volume $percent%", Icons.Rounded.VolumeUp, SystemColor.PINK)
    is Action.PcMedia -> if (play) Look("Play PC media", Icons.Rounded.PlayArrow, SystemColor.PINK) else Look("Pause PC media", Icons.Rounded.Pause, SystemColor.PINK)
    is Action.PcBrightness -> Look("PC brightness $percent%", Icons.Rounded.BrightnessMedium, SystemColor.YELLOW)
    is Action.RunCommand -> Look("Run \"$name\"", Icons.Rounded.Terminal, SystemColor.GRAY)
    Action.SendClipboard -> Look("Send clipboard to PC", Icons.Rounded.ContentPaste, SystemColor.ORANGE)
    is Action.MessagePc -> Look("Message on PC", Icons.Rounded.Chat, SystemColor.CYAN, text)
    Action.PcScreenshot -> Look("Screenshot to phone", Icons.Rounded.Screenshot, SystemColor.BLUE)
    Action.WakePcScreen -> Look("Wake PC screen", Icons.Rounded.DesktopWindows, SystemColor.TEAL)
    Action.SleepPc -> Look("Put PC to sleep", Icons.Rounded.Bedtime, SystemColor.BROWN)
    is Action.NotifyMe -> Look("Notify me", Icons.Rounded.Notifications, SystemColor.RED, text)
    is Action.PhoneRinger -> if (mode == RingerMode.VIBRATE) Look("Phone on vibrate", Icons.Rounded.Vibration, SystemColor.MINT) else Look("Phone ringer on", Icons.Rounded.RingVolume, SystemColor.MINT)
    Action.PausePhoneMedia -> Look("Pause phone media", Icons.Rounded.PauseCircle, SystemColor.PINK)
}

/** One line for a rule's row: what sets it off and what it does. */
fun Rule.summary(): String {
    val does = actions.joinToString(", ") { it.look().label.inSentence() }.replaceFirstChar(Char::uppercase)
    val only = conditions.takeIf { it.isNotEmpty() }?.joinToString(" and ", prefix = ", if ") { it.look().label.inSentence() }.orEmpty()
    return "When ${trigger.look().label.inSentence()}$only: $does"
}

/** Lower-cases a label's first word mid-sentence, but never an acronym ("PC", "Wi-Fi"). */
internal fun String.inSentence(): String =
    if (length > 1 && this[1].isUpperCase()) this else replaceFirstChar(Char::lowercase)

fun clock(minutes: Int): String = "%02d:%02d".format((minutes / 60) % 24, minutes % 60)

private val DAY_NAMES = listOf("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")

fun days(mask: Int): String = when (mask and EVERY_DAY) {
    EVERY_DAY -> "Every day"
    0b0011111 -> "Weekdays"
    0b1100000 -> "Weekends"
    0 -> "Never"
    else -> DAY_NAMES.filterIndexed { i, _ -> mask and (1 shl i) != 0 }.joinToString(", ")
}

/** Ready-made rules to switch on in one tap; each a good example of what the builder can do. */
object Suggestions {
    class Suggestion(val key: String, val description: String, val rule: Rule)

    val all = listOf(
        Suggestion(
            "walkaway", "Your PC locks itself once your phone has been gone a minute (needs the companion).",
            Rule(name = "Lock PC when I leave", trigger = Trigger.PcLink(connected = false, afterMin = 1), actions = listOf(Action.LockPc), suggestion = "walkaway"),
        ),
        Suggestion(
            "calls", "The PC goes quiet while you're on the phone, and picks up again after.",
            Rule(name = "Quiet PC during calls", trigger = Trigger.Call(ringingOnly = false), actions = listOf(Action.PcMedia(play = false), Action.MutePc(mute = true)), undo = true, suggestion = "calls"),
        ),
        Suggestion(
            "desk", "At your desk the phone stays on vibrate; it rings again when you leave.",
            Rule(name = "Desk mode", trigger = Trigger.PcLink(connected = true), actions = listOf(Action.PhoneRinger(RingerMode.VIBRATE)), undo = true, suggestion = "desk"),
        ),
        Suggestion(
            "follow", "Start something on the PC and the phone's music pauses, so they never play over each other.",
            Rule(name = "One thing playing", trigger = Trigger.PcPlaying(playing = true), actions = listOf(Action.PausePhoneMedia), suggestion = "follow"),
        ),
        Suggestion(
            "headphones", "Put headphones on your phone and the PC's media pauses.",
            Rule(name = "Headphones take over", trigger = Trigger.Headphones(connected = true), actions = listOf(Action.PcMedia(play = false)), suggestion = "headphones"),
        ),
        Suggestion(
            "pcbattery", "A heads-up on your phone before the laptop dies.",
            Rule(name = "PC battery alert", trigger = Trigger.PcBatteryBelow(20), conditions = listOf(Trigger.PcCharging(charging = false)), actions = listOf(Action.NotifyMe("Your PC is below 20%. Plug it in.")), suggestion = "pcbattery"),
        ),
        Suggestion(
            "night", "Late at night the PC's sound goes off and its screen dims, until morning.",
            Rule(name = "Night", trigger = Trigger.Time(start = 23 * 60, end = 7 * 60), actions = listOf(Action.MutePc(mute = true), Action.PcBrightness(20)), undo = true, suggestion = "night"),
        ),
    )
}
