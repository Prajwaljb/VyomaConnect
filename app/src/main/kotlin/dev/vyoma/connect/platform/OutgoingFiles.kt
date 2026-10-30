package dev.vyoma.connect.platform

import android.content.ContentResolver
import android.net.Uri
import android.provider.OpenableColumns
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.OutgoingFile

/** A picked or shared document as something to upload: its display name, size, and a way to read it. */
fun ContentResolver.outgoingFile(uri: Uri): OutgoingFile {
    var name: String? = null
    var size = -1L
    runCatching {
        query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
            if (c.moveToFirst()) {
                name = c.getString(0)
                if (!c.isNull(1)) size = c.getLong(1)
            }
        }
    }.onFailure { VLog.w("Share", "can't read $uri: ${it.message}") }
    if (size < 0) size = runCatching { openAssetFileDescriptor(uri, "r")?.use { it.length } }.getOrNull() ?: -1
    return OutgoingFile(name ?: uri.lastPathSegment ?: "file", size, null) {
        openInputStream(uri) ?: error("can't open $uri")
    }
}
