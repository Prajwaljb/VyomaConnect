package dev.vyoma.connect.ui

import dev.vyoma.connect.designsystem.CanvasWallpaper
import dev.vyoma.connect.designsystem.SpecularLight
import dev.vyoma.connect.designsystem.lightAngle
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class WallpaperLightTest {
    private fun wallpaper(vararg columns: Int) = CanvasWallpaper(columns.size, 2, IntArray(columns.size * 2) { columns[it % columns.size] })

    @Test
    fun `the light comes from the bright side`() {
        assertTrue(wallpaper(0xFFFFFF, 0x202020, 0x000000).lightAngle() < SpecularLight.REST - 10f)
        assertTrue(wallpaper(0x000000, 0x202020, 0xFFFFFF).lightAngle() > SpecularLight.REST + 10f)
    }

    @Test
    fun `an even wallpaper leaves it overhead`() {
        assertEquals(SpecularLight.REST, wallpaper(0x808080, 0x808080, 0x808080).lightAngle(), 0.01f)
        assertEquals(SpecularLight.REST, wallpaper(0xFFFFFF, 0x808080, 0xFFFFFF).lightAngle(), 0.01f)
    }

    @Test
    fun `a faint difference barely moves it`() {
        assertEquals(SpecularLight.REST, wallpaper(0x202020, 0x1C1C1C, 0x181818).lightAngle(), 2f)
    }
}
