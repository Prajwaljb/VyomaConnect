package dev.vyoma.connect.demo

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.RadialGradient
import android.graphics.RectF
import android.graphics.Shader
import java.io.ByteArrayOutputStream
import kotlin.random.Random

/** Pictures the simulated PC sends (album covers, screenshots, webcam shots), drawn on the spot: no bundled assets. */
internal object DemoArt {
    /** A square cover: two-hue gradient with soft orbs, different for every [seed]. */
    fun cover(seed: Int, hue: Float, size: Int = 640): ByteArray {
        val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        val c = Canvas(bmp)
        val s = size.toFloat()
        val a = hsv(hue, 0.75f, 0.55f)
        val b = hsv((hue + 40f) % 360f, 0.85f, 0.18f)
        c.drawRect(0f, 0f, s, s, Paint().apply { shader = LinearGradient(0f, 0f, s, s, a, b, Shader.TileMode.CLAMP) })
        val rnd = Random(seed)
        repeat(5) {
            val x = rnd.nextFloat() * s
            val y = rnd.nextFloat() * s
            val r = s * (0.15f + rnd.nextFloat() * 0.35f)
            val glow = hsv((hue + rnd.nextFloat() * 80f - 40f + 360f) % 360f, 0.6f, 0.95f)
            c.drawCircle(x, y, r, Paint(Paint.ANTI_ALIAS_FLAG).apply {
                shader = RadialGradient(x, y, r, Color.argb(150, Color.red(glow), Color.green(glow), Color.blue(glow)), Color.TRANSPARENT, Shader.TileMode.CLAMP)
            })
        }
        // A thin ring, like a record label.
        c.drawCircle(s * 0.5f, s * 0.5f, s * 0.28f, Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE; strokeWidth = s * 0.012f; color = Color.argb(90, 255, 255, 255)
        })
        return bmp.encode(Bitmap.CompressFormat.PNG)
    }

    /** A desktop: wallpaper, two windows and a panel. */
    fun screenshot(): ByteArray {
        val w = 1920f; val h = 1080f
        val bmp = Bitmap.createBitmap(w.toInt(), h.toInt(), Bitmap.Config.ARGB_8888)
        val c = Canvas(bmp)
        c.drawRect(0f, 0f, w, h, Paint().apply { shader = LinearGradient(0f, 0f, w, h, hsv(215f, 0.7f, 0.45f), hsv(275f, 0.6f, 0.2f), Shader.TileMode.CLAMP) })
        val window = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(235, 32, 35, 42) }
        val bar = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(255, 48, 52, 62) }
        val line = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(120, 200, 205, 215) }
        for ((x, y, ww, hh) in listOf(listOf(180f, 140f, 900f, 600f), listOf(980f, 320f, 760f, 520f))) {
            val r = RectF(x, y, x + ww, y + hh)
            c.drawRoundRect(r, 18f, 18f, window)
            c.drawRoundRect(RectF(x, y, x + ww, y + 52f), 18f, 18f, bar)
            for (i in 0 until 9) c.drawRoundRect(RectF(x + 40f, y + 90f + i * 48f, x + 40f + ww * (0.35f + (i * 37 % 50) / 100f), y + 108f + i * 48f), 8f, 8f, line)
        }
        c.drawRect(0f, h - 56f, w, h, Paint().apply { color = Color.argb(230, 20, 22, 28) })
        return bmp.encode(Bitmap.CompressFormat.PNG)
    }

    /** A soft, out-of-focus "webcam" frame. */
    fun photo(): ByteArray {
        val w = 1280f; val h = 720f
        val bmp = Bitmap.createBitmap(w.toInt(), h.toInt(), Bitmap.Config.ARGB_8888)
        val c = Canvas(bmp)
        c.drawRect(0f, 0f, w, h, Paint().apply { shader = LinearGradient(0f, 0f, 0f, h, hsv(30f, 0.35f, 0.75f), hsv(20f, 0.5f, 0.3f), Shader.TileMode.CLAMP) })
        c.drawCircle(w * 0.5f, h * 0.62f, h * 0.42f, Paint(Paint.ANTI_ALIAS_FLAG).apply {
            shader = RadialGradient(w * 0.5f, h * 0.62f, h * 0.42f, Color.argb(200, 70, 55, 50), Color.TRANSPARENT, Shader.TileMode.CLAMP)
        })
        return bmp.encode(Bitmap.CompressFormat.JPEG)
    }

    private fun Bitmap.encode(format: Bitmap.CompressFormat): ByteArray =
        ByteArrayOutputStream().also { compress(format, 88, it); recycle() }.toByteArray()

    private fun hsv(h: Float, s: Float, v: Float) = Color.HSVToColor(floatArrayOf(h, s, v))
}
