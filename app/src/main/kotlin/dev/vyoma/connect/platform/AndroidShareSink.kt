package dev.vyoma.connect.platform

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.plugin.builtin.IncomingFile
import dev.vyoma.connect.protocol.plugin.builtin.ShareSink
import dev.vyoma.connect.service.Notifications
import java.io.File
import java.io.OutputStream

/** Files from the PC land in Downloads/Vyoma (a sent folder inside it, as it was) through MediaStore, so no storage permission is needed. */
class AndroidShareSink(private val context: Context, private val clipboard: AndroidClipboard) : ShareSink {

    override fun onText(from: Device, text: String) {
        clipboard.applyRemote(text, from.id)
        Notifications.textReceived(context, from.name, text)
    }

    override fun onUrl(from: Device, url: String) = Notifications.linkReceived(context, from.name, url)

    override fun createFile(from: Device, name: String, size: Long, lastModified: Long?, folder: String?): IncomingFile? {
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, mimeOf(name))
            put(MediaStore.Downloads.RELATIVE_PATH, listOfNotNull("${Environment.DIRECTORY_DOWNLOADS}/Vyoma", folder).joinToString("/"))
            put(MediaStore.Downloads.IS_PENDING, 1)
            if (size >= 0) put(MediaStore.Downloads.SIZE, size)
        }
        return open(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
    }

    /** Screenshots go with the user's pictures, so Gallery shows them in a "Vyoma" album. */
    override fun createPicture(from: Device, name: String, size: Long): IncomingFile? {
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, name)
            put(MediaStore.Images.Media.MIME_TYPE, mimeOf(name))
            put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/Vyoma")
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }
        return open(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
    }

    /** Null when it can't be made (storage full, too many copies of the name): these calls can throw. */
    private fun open(collection: Uri, values: ContentValues): IncomingFile? {
        val resolver = context.contentResolver
        return runCatching {
            val uri = resolver.insert(collection, values) ?: return null
            val out = runCatching { resolver.openOutputStream(uri) }.getOrNull()
            if (out == null) {
                resolver.delete(uri, null, null)
                return null
            }
            MediaStoreFile(uri, out.buffered(256 * 1024))
        }.onFailure { VLog.w("ShareSink", "couldn't create a file: ${it.message}") }.getOrNull()
    }

    /**
     * PC files opened only to look at: the app's cache (never the user's Downloads), shared with the
     * viewer through a FileProvider. Anything a day old is cleared each time, and Android clears the
     * cache itself when storage runs low.
     */
    override fun createTemporary(from: Device, name: String, size: Long): IncomingFile? {
        sweepViewing(context)
        // Its own folder, so the viewer shows the real name even if two files share it.
        val dir = File(File(context.cacheDir, VIEWING), System.nanoTime().toString()).apply { mkdirs() }
        val file = File(dir, name)
        val out = runCatching { file.outputStream() }.getOrNull() ?: return null
        return object : IncomingFile {
            override val output: OutputStream = out.buffered(256 * 1024)
            override fun complete(): String = FileProvider.getUriForFile(context, "${context.packageName}.files", file).toString()
            override fun abort() {
                runCatching { output.close() }
                dir.deleteRecursively()
            }
        }
    }

    private inner class MediaStoreFile(private val uri: Uri, override val output: OutputStream) : IncomingFile {
        override fun complete(): String {
            context.contentResolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            return uri.toString()
        }

        override fun abort() {
            runCatching { output.close() }
            runCatching { context.contentResolver.delete(uri, null, null) }
        }
    }

    companion object {
        private const val VIEWING = "viewing"
        private const val KEEP_MS = 24 * 60 * 60 * 1000L

        /** Clears looked-at PC files older than a day. */
        fun sweepViewing(context: Context) {
            val cutoff = System.currentTimeMillis() - KEEP_MS
            File(context.cacheDir, VIEWING).listFiles()?.forEach { if (it.lastModified() < cutoff) it.deleteRecursively() }
        }

        fun mimeOf(name: String): String =
            MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase()) ?: "application/octet-stream"
    }
}
