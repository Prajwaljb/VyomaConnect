package dev.vyoma.connect.automation

import java.util.Calendar
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class RulesTest {
    /** 2026-11-02 is a Monday (day 0). */
    private fun at(day: Int, hour: Int, minute: Int): Long = Calendar.getInstance().apply {
        set(2026, Calendar.NOVEMBER, 2 + day, hour, minute, 0); set(Calendar.MILLISECOND, 0)
    }.timeInMillis

    @Test
    fun aPeriodPastMidnightBelongsToTheDayItStarted() {
        val weekdayNights = Trigger.Time(start = 23 * 60, end = 7 * 60, days = 0b0011111)
        assertTrue(TimeRules.holds(weekdayNights, at(0, 23, 30)))   // Monday night
        assertTrue(TimeRules.holds(weekdayNights, at(1, 6, 59)))    // ...into Tuesday morning
        assertFalse(TimeRules.holds(weekdayNights, at(1, 7, 0)))
        assertTrue(TimeRules.holds(weekdayNights, at(5, 3, 0)))     // Friday night, early Saturday
        assertFalse(TimeRules.holds(weekdayNights, at(5, 23, 30)))  // Saturday night isn't a weekday
        assertFalse(TimeRules.holds(weekdayNights, at(0, 12, 0)))
    }

    @Test
    fun aMomentLastsTenMinutesAndTheNextBoundaryFollows() {
        val alarm = Trigger.Time(start = 8 * 60, end = null)
        assertFalse(TimeRules.holds(alarm, at(0, 7, 59)))
        assertTrue(TimeRules.holds(alarm, at(0, 8, 0)))
        assertTrue(TimeRules.holds(alarm, at(0, 8, 9)))
        assertFalse(TimeRules.holds(alarm, at(0, 8, 10)))
        assertEquals(at(0, 8, 0) + 1_000, TimeRules.nextBoundary(alarm, at(0, 7, 0)))
        assertEquals(at(0, 8, 10) + 1_000, TimeRules.nextBoundary(alarm, at(0, 8, 2)))
        assertEquals(at(1, 8, 0) + 1_000, TimeRules.nextBoundary(alarm, at(0, 9, 0)))
    }

    @Test
    fun rulesSurviveARoundTrip() {
        val rules = Suggestions.all.map { it.rule } + Rule(
            name = "Everything",
            trigger = Trigger.Time(60, 120, 0b1010101),
            conditions = listOf(Trigger.HomeWifi(true), Trigger.PcBatteryBelow(30), Trigger.Call(ringingOnly = true)),
            actions = listOf(Action.RunCommand("k1", "Backup"), Action.MessagePc("hi"), Action.PhoneRinger(RingerMode.NORMAL), Action.PausePhoneMedia, Action.SleepPc),
            enabled = false,
        )
        assertEquals(rules, RuleJson.decode(RuleJson.encode(rules)))
    }

    @Test
    fun summariesKeepAcronyms() {
        val r = Rule(name = "x", trigger = Trigger.PcLink(true), conditions = listOf(Trigger.HomeWifi(true)), actions = listOf(Action.MutePc(true), Action.NotifyMe("")))
        assertEquals("When PC is connected, if on home Wi-Fi: Mute PC, notify me", r.summary())
    }

    @Test
    fun onlyLockPauseAndMuteRunOnTheirOwnOnThePc() {
        assertTrue(Action.LockPc.runsOnPcWhenAway())
        assertTrue(Action.PcMedia(play = false).runsOnPcWhenAway())
        assertFalse(Action.PcMedia(play = true).runsOnPcWhenAway())
        assertFalse(Action.SendClipboard.runsOnPcWhenAway())
    }
}
