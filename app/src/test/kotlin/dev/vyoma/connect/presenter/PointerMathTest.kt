package dev.vyoma.connect.presenter

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class PointerMathTest {
    private val flat = floatArrayOf(0f, 0f, 1f)     // lying on its back, top edge pointing at the screen
    private val upright = floatArrayOf(0f, 1f, 0f)  // held up like a window

    @Test fun turningRightMovesRight() {
        // Clockwise seen from above is minus the spin around "up".
        assertTrue(PointerMath.step(0f, 0f, -1f, flat, 0.02f).first > 0f)
        assertTrue(PointerMath.step(0f, -1f, 0f, upright, 0.02f).first > 0f)
    }

    @Test fun turnFeelsTheSameHoweverItIsTilted() =
        assertEquals(PointerMath.step(0f, 0f, -1f, flat, 0.02f).first, PointerMath.step(0f, -1f, 0f, upright, 0.02f).first, 1e-6f)

    @Test fun raisingTheTopEdgeMovesUp() {
        assertTrue(PointerMath.step(1f, 0f, 0f, flat, 0.02f).second < 0f)
        assertTrue(PointerMath.step(1f, 0f, 0f, upright, 0.02f).second < 0f)
    }

    @Test fun aboutThirtyEightDegreesCrossesTheScreen() {
        val radians = Math.toRadians(38.2).toFloat()
        val (dx, _) = PointerMath.step(0f, 0f, -radians, flat, 1f)
        assertEquals(2f, dx, 0.01f)
    }

    @Test fun aSteadyHandBarelyMoves() {
        // 0.009 rad/s is what the S24 FE's gyro reads when still (see GyroAim).
        val (dx, _) = PointerMath.step(0f, 0f, -0.009f, flat, 1f)
        assertTrue(dx < 0.006f, "$dx")
    }

    @Test fun airMouseIsPreciseSlowAndQuickFast() {
        // A slow 10°/s pan: about 19 px a degree. A quick flick: three times that.
        val slow = AirMouseMath.step(0f, 0f, -Math.toRadians(10.0).toFloat(), flat, 1f).first
        assertEquals(Math.toRadians(10.0).toFloat() * AirMouseMath.SLOW_PX_RAD, slow, 0.5f)
        val fast = AirMouseMath.step(0f, 0f, -3f, flat, 0.01f).first
        assertEquals(3f * 0.01f * AirMouseMath.FAST_PX_RAD, fast, 0.5f)
        assertTrue(AirMouseMath.step(1f, 0f, 0f, upright, 0.01f).second < 0f)
    }

    @Test fun airMouseHoldsStillInASteadyHand() =
        assertTrue(AirMouseMath.step(0f, 0f, -0.009f, flat, 0.02f).first < 0.05f)
}
