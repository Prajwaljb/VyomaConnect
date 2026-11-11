package dev.vyoma.connect.ui

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Share
import dev.vyoma.connect.designsystem.StatusController
import dev.vyoma.connect.platform.AndroidShareSink

/**
 * Opens a file in whatever app views its kind. With none (a .db, say), the share sheet takes its
 * place, so a tap always leads somewhere: a file manager, a cloud drive, another app. A file that's
 * gone from the phone says so instead of opening a viewer that fails.
 */
internal fun openFile(context: Context, uri: Uri, name: String, status: StatusController) {
    val gone = runCatching { context.contentResolver.openFileDescriptor(uri, "r")?.close() ?: error("none") }.isFailure
    if (gone) return status.show("$name is no longer on this phone", failure = true)
    val mime = AndroidShareSink.mimeOf(name)
    val view = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, mime)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    if (runCatching { context.startActivity(view) }.isSuccess) return
    val send = Intent(Intent.ACTION_SEND)
        .setType(mime)
        .putExtra(Intent.EXTRA_STREAM, uri)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        // The read grant reaches the chosen app through the clip.
        .apply { clipData = ClipData.newRawUri(name, uri) }
    val sheet = Intent.createChooser(send, name).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    if (runCatching { context.startActivity(sheet) }.isSuccess) {
        val ext = name.substringAfterLast('.', "").lowercase()
        status.show(if (ext.isEmpty()) "No app here opens it. Share it instead" else "No app here opens .$ext files. Share it instead", icon = Icons.Rounded.Share)
    } else {
        status.show("No app here opens $name", failure = true)
    }
}
