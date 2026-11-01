package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.Device
import kotlinx.coroutines.delay
import java.util.UUID

/** One thing to send: a file, at [folder] inside a sent folder ("Photos/2024"), or loose (null). */
class SendItem(val file: OutgoingFile, val folder: String? = null)

/**
 * Sends files and folders to [pc] through its companion, carrying on after drops: when a send
 * breaks off (Wi-Fi hiccup, the PC asleep for a moment, the cable pulled) it waits for the
 * companion to come back and the PC resumes from what it already has. Gives up only after
 * [STALL_MS] without a byte of progress, or when the PC refuses outright.
 *
 * Loose files show as a transfer each; a folder shows as one transfer of all its bytes.
 */
class CompanionUploads(private val pc: Device, private val companion: () -> CompanionPlugin?) {

    /** Returns how many of [items] arrived. [folderName] names the transfer when they're a folder. */
    suspend fun send(items: List<SendItem>, folderName: String? = null): Int {
        if (folderName == null) return items.count { item ->
            val id = Transfers.start(pc, item.file.name, incoming = false, total = item.file.size)
            val ok = sendOne(item, batch = null) { Transfers.progress(id, it) } != null
            Transfers.finish(id, ok)
            ok
        }
        val batch = UUID.randomUUID().toString()
        val total = items.sumOf { it.file.size }
        val id = Transfers.start(pc, folderName, incoming = false, total = total)
        var before = 0L
        var arrived = 0
        for (item in items) {
            if (sendOne(item, batch) { Transfers.progress(id, before + it) } != null) arrived++
            before += item.file.size
        }
        companion()?.endBatch(batch, arrived)
        Transfers.finish(id, ok = arrived == items.size)
        return arrived
    }

    /** The saved name, or null if it couldn't be sent. */
    private suspend fun sendOne(item: SendItem, batch: String?, onProgress: (Long) -> Unit): String? {
        val key = UUID.randomUUID().toString()
        var best = 0L
        var lastProgress = System.currentTimeMillis()
        while (System.currentTimeMillis() - lastProgress < STALL_MS) {
            val plugin = companion()
            if (plugin == null) {
                delay(WAIT_MS) // the companion is away; it reconnects by itself
                continue
            }
            val result = plugin.sendResumable(item.file, key, item.folder, batch) { have ->
                if (have > best) {
                    best = have
                    lastProgress = System.currentTimeMillis()
                }
                onProgress(have)
            }
            when (result) {
                is XferResult.Saved -> return result.name
                is XferResult.Failed -> {
                    VLog.w(TAG, "${item.file.name}: ${result.reason}${if (result.retry) "; resuming" else ""}")
                    if (!result.retry) return null
                    delay(WAIT_MS)
                }
            }
        }
        return null
    }

    private companion object {
        const val TAG = "Uploads"
        const val WAIT_MS = 2_000L
        /** No progress for this long and the send is given up (the PC is off, or gone for good). */
        const val STALL_MS = 5 * 60_000L
    }
}
