package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.plugin.builtin.SharePlugin.Companion.safeFolder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ShareFolderTest {
    @Test fun aSentFolderIsRebuiltAsItWas() {
        assertEquals("Photos/2024", safeFolder("Photos/2024"))
        assertEquals("My files/notes", safeFolder("My files\\notes"))
    }

    @Test fun aFolderNeverLeadsUpOrOut() {
        assertEquals("etc", safeFolder("../../etc"))
        assertEquals("a/b", safeFolder("/a//./b/"))
        assertNull(safeFolder("../.."))
        assertNull(safeFolder(""))
    }

    @Test fun hiddenAndOddPartsBecomePlain() {
        assertEquals("git/objects", safeFolder(".git/objects"))
        assertEquals("ab", safeFolder("a\u0000b"))
        assertEquals(16, safeFolder((1..40).joinToString("/") { "d$it" })!!.split('/').size)
    }
}
