package dev.vyoma.connect.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NoCameraMessageTest {
    @Test fun aDesktopWithoutAWebcamIsNotToldAboutALid() {
        val message = noCameraMessage(behindLid = false)
        assertTrue(message, message.contains("no camera"))
        assertFalse(message, message.contains("lid"))
    }

    @Test fun aClosedLaptopAndAnOlderCompanionMeanTheLid() {
        assertTrue(noCameraMessage(behindLid = true).contains("lid"))
        assertTrue(noCameraMessage(behindLid = null).contains("lid"))
    }
}
