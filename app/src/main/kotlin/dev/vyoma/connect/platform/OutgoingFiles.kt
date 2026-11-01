package dev.vyoma.connect.platform

import android.content.ContentResolver
import android.net.Uri
import android.provider.OpenableColumns
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.OutgoingFile
import dev.vyoma.connect.protocol.plugin.builtin.SendItem
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document

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

/**
 * Everything in a picked folder ([tree] from the system's folder picker), each at its path inside
 * it ("Photos/2024"), and the folder's name. Empty folders don't travel; neither do files whose
 * size the provider won't say (the PC needs it to resume).
 */
fun ContentResolver.folderItems(tree: Uri): Pair<String, List<SendItem>> {
    val rootId = DocumentsContract.getTreeDocumentId(tree)
    val rootName = runCatching {
        query(DocumentsContract.buildDocumentUriUsingTree(tree, rootId), arrayOf(Document.COLUMN_DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    }.getOrNull() ?: "Folder"
    val items = mutableListOf<SendItem>()
    fun walk(docId: String, path: String) {
        val cols = arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED)
        val children = runCatching { query(DocumentsContract.buildChildDocumentsUriUsingTree(tree, docId), cols, null, null, null) }
            .onFailure { VLog.w("Share", "can't list $path: ${it.message}") }.getOrNull() ?: return
        children.use { c ->
            while (c.moveToNext()) {
                val id = c.getString(0)
                val name = c.getString(1) ?: continue
                if (c.getString(2) == Document.MIME_TYPE_DIR) {
                    walk(id, "$path/$name")
                    continue
                }
                if (c.isNull(3)) continue
                val uri = DocumentsContract.buildDocumentUriUsingTree(tree, id)
                val modified = if (c.isNull(4)) null else c.getLong(4)
                items += SendItem(OutgoingFile(name, c.getLong(3), modified) { openInputStream(uri) ?: error("can't open $name") }, path)
            }
        }
    }
    walk(rootId, rootName)
    return rootName to items
}
