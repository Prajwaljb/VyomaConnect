package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.plugin.builtin.PcSpecs
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class PcSpecsTest {
    private fun body(json: String) = Json.parseToJsonElement(json) as JsonObject

    @Test
    fun readsWhatTheCompanionSends() {
        val s = PcSpecs.parse(body("""{"model":"Framework Laptop 13","os":"Fedora Linux 43","kernel":"6.17.4-300.fc43.x86_64","desktop":"KDE Plasma",
            "session":"Wayland","cpu":"AMD Ryzen 7 7840U","cores":8,"threads":16,"gpus":["AMD Radeon 780M","NVIDIA GeForce RTX 4050"],
            "memTotal":32856000000,"diskTotal":1000204886016,"diskFree":612384000000,
            "displays":[{"w":2880,"h":1920,"internal":true},{"w":3840,"h":2160,"internal":false}]}"""))!!
        assertEquals("Fedora Linux 43", s.os)
        assertEquals(8, s.cores)
        assertEquals(listOf("AMD Radeon 780M", "NVIDIA GeForce RTX 4050"), s.gpus)
        assertEquals(PcSpecs.Display(2880, 1920, internal = true), s.displays.first())
        assertEquals(612384000000, s.diskFree)
    }

    @Test
    fun missingOrOddPartsAreLeftOut() {
        val s = PcSpecs.parse(body("""{"os":"Fedora Linux 43","cpu":null,"cores":"x","gpus":[3,""],"displays":[{"w":0,"h":10},{"w":"a"}],"memTotal":-1}"""))!!
        assertNull(s.cpu)
        assertNull(s.cores)
        assertNull(s.memTotal)
        assertEquals(emptyList(), s.gpus)
        assertEquals(emptyList(), s.displays)
    }

    @Test
    fun anErrorOrNothingUsefulIsNoSpecs() {
        assertNull(PcSpecs.parse(body("""{"error":"no"}""")))
        assertNull(PcSpecs.parse(body("""{"kernel":"6.1"}""")))
    }
}
