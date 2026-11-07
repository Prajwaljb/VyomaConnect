package dev.vyoma.connect.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class SpecSizesTest {
    @Test
    fun memoryReadsAsTheSizeItWasSoldAs() {
        assertEquals("16 GB", memorySize(16_400_000_000))
        assertEquals("32 GB", memorySize(32_856_000_000))
        assertEquals("8 GB", memorySize(7_900_000_000))
        assertEquals("512 MB", memorySize(512L * 1024 * 1024))
    }

    @Test
    fun disksReadInThousands() {
        assertEquals("512 GB", diskSize(512_110_190_592))
        assertEquals("1 TB", diskSize(1_000_204_886_016))
        assertEquals("2.5 TB", diskSize(2_500_000_000_000))
        assertEquals("4.2 GB", diskSize(4_200_000_000))
    }
}
