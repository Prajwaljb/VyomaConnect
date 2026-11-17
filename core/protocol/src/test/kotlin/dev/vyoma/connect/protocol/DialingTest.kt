package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.plugin.builtin.Dialing
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** A PC may only have the phone call a plain number: no codes that change the phone's settings. */
class DialingTest {
    @Test fun plainNumbersOnly() {
        assertEquals("+15550100123", Dialing.plain(" +1 (555) 010-0123 "))
        assertEquals("112", Dialing.plain("112"))
        for (code in listOf("*21*5550100#", "*#06#", "555,123", "555;1", "1-800-FLOWERS", "5+55", "12", "", "tel:5550100")) {
            assertNull(code, Dialing.plain(code))
        }
    }
}
