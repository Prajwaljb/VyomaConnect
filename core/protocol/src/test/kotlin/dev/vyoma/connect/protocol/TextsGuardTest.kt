package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.plugin.builtin.TextsGuard
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/** Who gets the texts: Vyoma's PCs once unlocked, stock desktops unless cut off. */
class TextsGuardTest {
    private class Guard(var unlocked: Boolean = false, var cut: Boolean = false) : TextsGuard {
        override fun open(peerId: String) = unlocked
        override fun ask(peerId: String, peerName: String, answer: (Boolean) -> Unit) = answer(false)
        override fun close(peerId: String) = Unit
        override fun cutOff(peerId: String, claims: Boolean) = cut && !holds(peerId, claims)
    }

    @Test
    fun vyomaPcsWaitForTheOwner() {
        val g = Guard()
        assertFalse(g.lets("pc", claims = true))
        g.unlocked = true
        assertTrue(g.lets("pc", claims = true))
        g.cut = true // only stock desktops are cut off
        assertTrue(g.lets("pc", claims = true))
    }

    @Test
    fun stockPcsUnlessCutOff() {
        val g = Guard()
        assertTrue(g.lets("kde", claims = false))
        g.cut = true
        assertFalse(g.lets("kde", claims = false))
        g.unlocked = true // a yes for a Vyoma PC doesn't open texts to a cut-off one
        assertFalse(g.lets("kde", claims = false))
    }
}
