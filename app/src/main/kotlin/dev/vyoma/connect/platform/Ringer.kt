package dev.vyoma.connect.platform

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.VibrationEffect
import android.os.VibratorManager
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * "Find my phone": loud alarm-stream ringing that cuts through silent mode, at full volume,
 * with vibration. The previous alarm volume comes back when it stops.
 */
object Ringer {
    private var player: MediaPlayer? = null
    private var savedVolume = -1

    private val _ringing = MutableStateFlow<String?>(null)
    /** Name of the device that asked, while ringing. */
    val ringing: StateFlow<String?> = _ringing.asStateFlow()

    @Synchronized
    fun start(context: Context, from: String) {
        if (player != null) return
        val app = context.applicationContext
        val audio = app.getSystemService(AudioManager::class.java)
        savedVolume = audio.getStreamVolume(AudioManager.STREAM_ALARM)
        audio.setStreamVolume(AudioManager.STREAM_ALARM, audio.getStreamMaxVolume(AudioManager.STREAM_ALARM), 0)
        val uri = RingtoneManager.getActualDefaultRingtoneUri(app, RingtoneManager.TYPE_RINGTONE)
            ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM)
        player = runCatching {
            MediaPlayer().apply {
                setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ALARM).setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build())
                setDataSource(app, uri)
                isLooping = true
                prepare()
                start()
            }
        }.getOrNull()
        app.getSystemService(VibratorManager::class.java)?.defaultVibrator?.vibrate(
            VibrationEffect.createWaveform(longArrayOf(0, 600, 400), 0),
        )
        _ringing.value = from
    }

    @Synchronized
    fun stop(context: Context) {
        val app = context.applicationContext
        player?.runCatching { stop(); release() }
        player = null
        app.getSystemService(VibratorManager::class.java)?.defaultVibrator?.cancel()
        if (savedVolume >= 0) {
            runCatching { app.getSystemService(AudioManager::class.java).setStreamVolume(AudioManager.STREAM_ALARM, savedVolume, 0) }
            savedVolume = -1
        }
        _ringing.value = null
    }
}
