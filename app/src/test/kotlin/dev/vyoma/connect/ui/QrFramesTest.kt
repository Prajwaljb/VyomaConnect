package dev.vyoma.connect.ui

import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The PC's code as a camera sees it: grey-on-grey, softened, in a padded full-HD brightness plane. */
class QrFramesTest {
    private val code = "https://github.com/Prajwaljb/VyomaConnect/releases/latest#pair?v=1&id=0123456789abcdef0123456789abcdef" +
        "&name=Desk%20PC&fp=1b9f3d973c838fe866cb806b72ef20448ccb55c320a514b35e9e03808d00069b" +
        "&s=00112233445566778899aabbccddeeff&a=192.168.1.20,10.0.0.5&p=1716"

    private fun frame(pxPerModule: Double, width: Int = 1920, height: Int = 1080, stride: Int = width + 64): ByteArray {
        val m = QRCodeWriter().encode(code, BarcodeFormat.QR_CODE, 0, 0, mapOf(EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.L, EncodeHintType.MARGIN to 2))
        val modules = m.width
        val side = modules * pxPerModule
        val x0 = width / 2 - side / 2 + 37
        val y0 = height / 2 - side / 2 - 21
        val y = ByteArray(stride * height) { 90 }
        for (row in 0 until height) for (col in 0 until width) {
            // Average a 2x2 subsample per pixel: the lens and sensor soften edges.
            var sum = 0.0
            for (dy in 0..1) for (dx in 0..1) {
                val mx = ((col + dx * 0.5 - x0) / pxPerModule).toInt()
                val my = ((row + dy * 0.5 - y0) / pxPerModule).toInt()
                val inside = col + dx * 0.5 >= x0 && row + dy * 0.5 >= y0 && mx in 0 until modules && my in 0 until modules
                sum += if (!inside) 90.0 else if (m.get(mx, my)) 45.0 else 200.0
            }
            y[row * stride + col] = (sum / 4).toInt().toByte()
        }
        return y
    }

    private fun readWithin(frames: Int, y: ByteArray, stride: Int = 1984): String? {
        val r = QrFrames()
        repeat(frames) { r.read(y, stride, 1920, 1080)?.let { return it } }
        return null
    }

    @Test
    fun readsTheCodeAtThreePixelsASquare() = assertEquals(code, readWithin(3, frame(3.0)))

    @Test
    fun readsItBigToo() = assertEquals(code, readWithin(3, frame(12.0)))

    @Test
    fun oneSquareAPixelIsTooSmall() = assertNull(readWithin(3, frame(1.1)))
}
