package dev.vyoma.connect.platform

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LockSessionTest {
    private var clock = 10 * AWAY_MS
    private val session = LockSession { clock }.apply { started() }

    @Test fun startsClosedAndStaysOpenOncePassed() {
        assertFalse(session.open)
        session.passed()
        clock += 10 * AWAY_MS // left open on the screen all this time
        assertTrue(session.open)
    }

    @Test fun aQuickLookElsewhereKeepsIt() {
        session.passed()
        session.stopped()
        clock += AWAY_MS - 1
        session.started()
        assertTrue(session.open)
    }

    @Test fun stayingAwayEndsIt() {
        session.passed()
        session.stopped()
        clock += AWAY_MS
        session.started()
        assertFalse(session.open)
    }

    @Test fun movingBetweenItsOwnScreensDoesntCountAsAway() {
        session.passed()
        session.started() // the next activity starts before the last one stops
        session.stopped()
        clock += 10 * AWAY_MS
        session.started()
        session.stopped()
        assertTrue(session.open)
    }

    @Test fun openingSoonAfterUnlockingThePhoneCounts() {
        session.stopped()
        clock += 10 * AWAY_MS
        session.phoneUnlocked()
        assertFalse(session.open)
        clock += UNLOCK_GRACE_MS - 1
        session.started()
        assertTrue(session.open)
        clock += 10 * AWAY_MS // it lasts the go, not just the grace
        assertTrue(session.open)
    }

    @Test fun openingLaterAfterUnlockingThePhoneDoesnt() {
        session.stopped()
        clock += 10 * AWAY_MS
        session.phoneUnlocked()
        clock += UNLOCK_GRACE_MS
        session.started()
        assertFalse(session.open)
    }

    @Test fun unlockingOntoVyomaCounts() {
        session.phoneUnlocked()
        assertTrue(session.open)
    }

    @Test fun theScreenTurningOffForgetsTheUnlock() {
        session.stopped()
        session.phoneUnlocked()
        session.end()
        session.started()
        assertFalse(session.open)
    }

    @Test fun theScreenTurningOffEndsIt() {
        session.passed()
        session.end()
        assertFalse(session.open)
    }
}
