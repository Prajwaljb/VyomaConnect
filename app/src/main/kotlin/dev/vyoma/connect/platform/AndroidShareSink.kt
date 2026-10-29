package dev.vyoma.connect.platform

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.plugin.builtin.IncomingFile
import dev.vyoma.connect.protocol.plugin.builtin.ShareSink
import dev.vyoma.connect.service.Notifications
import java.io.OutputStream

/** Files from the PC land in Downloads/Vyoma through MediaStore, so no storage permission is needed. */
class AndroidShareSink(private val context: Context, private val clipboard: AndroidClipboard) : ShareSink {

    override fun onText(from: Device, text: String) {
        clipboard.applyRemote(text, from.id)
        Notifications.textReceived(context, from.name, text)
    }

    override fun onUrl(from: Device, url: String) = Notifications.linkReceived(context, from.name, url)

    override fun createFile(from: Device, name: String, size: Long, lastModified: Long?): IncomingFile? {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, mimeOf(name))
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Vyoma")
            put(MediaStore.Downloads.IS_PENDING, 1)
            if (size >= 0) put(MediaStore.Downloads.SIZE, size)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: return null
        val out = resolver.openOutputStream(uri) ?: run {
            resolver.delete(uri, null, null)
            return null
        }
        return MediaStoreFile(uri, out.buffered(256 * 1024))
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
        fun mimeOf(name: String): String =
            MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase()) ?: "application/octet-stream"
    }
}
