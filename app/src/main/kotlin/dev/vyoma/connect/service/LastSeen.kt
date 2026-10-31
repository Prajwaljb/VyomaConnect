package dev.vyoma.connect.service

import android.content.Context
import androidx.core.content.edit

/** When each PC was last connected, for Home's "Away · seen 12 min. ago". Written by [LinkService] as links drop. */
object LastSeen {
    private fun prefs(context: Context) = context.getSharedPreferences("pc_seen", Context.MODE_PRIVATE)

    fun mark(context: Context, ids: Collection<String>, at: Long = System.currentTimeMillis()) =
        prefs(context).edit { ids.forEach { putLong(it, at) } }

    fun of(context: Context, id: String): Long? = prefs(context).getLong(id, 0L).takeIf { it > 0 }
}
