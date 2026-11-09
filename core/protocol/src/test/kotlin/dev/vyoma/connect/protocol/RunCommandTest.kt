package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.plugin.builtin.RemoteCommand
import dev.vyoma.connect.protocol.plugin.builtin.RunCommandPlugin
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlin.test.Test
import kotlin.test.assertEquals

class RunCommandTest {
    @Test
    fun fingerprintFlagsFromThePc() {
        val list = Json.parseToJsonElement(
            """
            {
              "a": {"name": "Wipe", "command": "wipe", "fingerprint": true, "everyTime": true},
              "b": {"name": "backup", "command": "backup"},
              "c": {"name": "Trash", "command": "gio trash --empty", "fingerprint": true},
              "d": {"name": "Odd", "command": "odd", "everyTime": true},
              "e": "not a command"
            }
            """.trimIndent(),
        ).jsonObject
        assertEquals(
            listOf(
                RemoteCommand("b", "backup", "backup"),
                // Every time alone, from a hand-edited file, means nothing without the fingerprint.
                RemoteCommand("d", "Odd", "odd"),
                RemoteCommand("c", "Trash", "gio trash --empty", fingerprint = true),
                RemoteCommand("a", "Wipe", "wipe", fingerprint = true, everyTime = true),
            ),
            RunCommandPlugin.parseCommands(list),
        )
    }
}
