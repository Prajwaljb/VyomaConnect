package dev.vyoma.connect.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class GamepadLayoutTest {
    @Test fun keepsAThumbInRange() {
        assertEquals(5f, 3f.keptIn(5f, 9f))
        assertEquals(9f, 12f.keptIn(5f, 9f))
        assertEquals(7f, 7f.keptIn(5f, 9f))
    }

    // Both triggers moved low: the range is empty, which crashed the app on a touch.
    @Test fun aLayoutWithNoRoomDoesntThrow() = assertEquals(300f, 100f.keptIn(350f, 300f))
}
