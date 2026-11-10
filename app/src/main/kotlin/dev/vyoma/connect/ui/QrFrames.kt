package dev.vyoma.connect.ui

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.detector.Detector

/**
 * Reads QR codes from camera frames' brightness planes. Most frames decode only the middle square,
 * where the viewfinder is: the code then has the frame's full resolution at a fraction of the work.
 * Every third frame looks at all of it, for a code held off centre.
 */
internal class QrFrames {
    private val reader = MultiFormatReader().apply {
        setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE), DecodeHintType.TRY_HARDER to true))
    }
    private var frame = 0
    /** How far the last frame got, for the log: no corner squares, found but unreadable, or read. */
    var lastStage = "none"
        private set

    /** The text of a code in [y] (rows [stride] bytes apart, [width] x [height] used), or null. */
    fun read(y: ByteArray, stride: Int, width: Int, height: Int): String? {
        val whole = frame++ % 3 == 2
        val side = minOf(width, height)
        val (left, top, w, h) = if (whole) listOf(0, 0, width, height) else listOf((width - side) / 2, (height - side) / 2, side, side)
        return try {
            val bitmap = BinaryBitmap(HybridBinarizer(PlanarYUVLuminanceSource(y, stride, height, left, top, w, h, false)))
            reader.decodeWithState(bitmap).text.also { lastStage = "read" }
        } catch (_: Exception) {
            lastStage = runCatching {
                Detector(BinaryBitmap(HybridBinarizer(PlanarYUVLuminanceSource(y, stride, height, left, top, w, h, false))).blackMatrix).detect()
                "corners found, unreadable"
            }.getOrDefault("no corners")
            null
        } finally {
            reader.reset()
        }
    }

    companion object {
        /** The middle third's mean brightness and share of near-white and near-black pixels. */
        fun brightness(y: ByteArray, stride: Int, width: Int, height: Int): String {
            var sum = 0L
            var white = 0
            var black = 0
            var n = 0
            for (row in height / 3 until 2 * height / 3 step 4) for (col in width / 3 until 2 * width / 3 step 4) {
                val v = y[row * stride + col].toInt() and 0xff
                sum += v
                if (v >= 245) white++
                if (v <= 40) black++
                n++
            }
            return "mean ${sum / n}, white ${white * 100 / n}%, black ${black * 100 / n}%"
        }
    }
}
