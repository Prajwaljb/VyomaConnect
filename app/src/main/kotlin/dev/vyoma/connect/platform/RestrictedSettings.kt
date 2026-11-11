package dev.vyoma.connect.platform

import android.app.AppOpsManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings

/**
 * Android 13+ "restricted settings": an app installed from a file (a download, a chat, a file
 * manager), not a store, can't be given notification access or accessibility until its owner
 * allows it in App info > ⋮ > Allow restricted settings. Until then the switch only says "For your
 * security, this setting is currently unavailable" (OnePlus, Samsung and Pixel all word it a little
 * differently), with nothing to say how to get past it.
 */
object RestrictedSettings {
    /** The hidden app op behind that menu entry: allowed once the owner has allowed it. */
    private const val OP = "android:access_restricted_settings"

    /** Whether Android may be holding those settings back from Vyoma. */
    fun mayBlock(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return false
        val source = runCatching { context.packageManager.getInstallSourceInfo(context.packageName).packageSource }.getOrNull()
        if (source == PackageInstaller.PACKAGE_SOURCE_STORE) return false
        val mode = runCatching {
            context.getSystemService(AppOpsManager::class.java).unsafeCheckOpNoThrow(OP, android.os.Process.myUid(), context.packageName)
        }.getOrNull()
        return mode != AppOpsManager.MODE_ALLOWED
    }

    /** Vyoma's App info page, where ⋮ holds Allow restricted settings. */
    fun appInfo(context: Context): Intent =
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
}
