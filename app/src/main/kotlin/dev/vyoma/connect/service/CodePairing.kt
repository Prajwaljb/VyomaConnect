package dev.vyoma.connect.service

import android.content.Context
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.crypto.QrInvite
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.device.PairState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.net.InetAddress

/**
 * Pairs with the PC whose code was scanned ([QrInvite]): dials the addresses in it, checks the PC
 * shows the certificate the code names, and sends the pair request with the code's proof, which the
 * PC accepts with nothing to compare. Works where the network hides the PC from discovery too.
 */
object CodePairing {
    sealed interface Step {
        data object Connecting : Step
        data object Pairing : Step
        data object Paired : Step
        data class Failed(val why: String) : Step
    }

    private const val REACH_MS = 12_000L
    private const val ANSWER_MS = 15_000L

    @OptIn(ExperimentalCoroutinesApi::class)
    suspend fun pair(context: Context, invite: QrInvite, onStep: (Step) -> Unit): Step {
        val app = context.applicationContext as android.app.Application
        LinkService.start(app, visible = true)
        val hub = app.container.obtainHub()
        val state = hub.devices.flatMapLatest { it[invite.deviceId]?.state ?: flowOf(null) }
        fun done(step: Step) = step.also(onStep)

        if (state.first()?.isPaired == true) return done(Step.Paired)
        onStep(Step.Connecting)
        withContext(Dispatchers.IO) {
            // IP literals only (QrInvite.parse), so this looks nothing up.
            invite.addresses.forEach { a -> runCatching { hub.dial(InetAddress.getByName(a), invite.port) } }
        }
        val reached = withTimeoutOrNull(REACH_MS) { state.first { it?.isReachable == true } }
            ?: return done(Step.Failed("Couldn't reach ${invite.name}. Check the phone and the PC are on the same Wi-Fi, then scan again."))
        if (reached.isPaired) return done(Step.Paired)

        onStep(Step.Pairing)
        val device = hub.device(invite.deviceId) ?: return done(Step.Failed("Lost ${invite.name}. Scan the code again."))
        if (!device.requestPairingByCode(invite)) {
            return done(Step.Failed("That isn't the PC that showed the code, so Vyoma didn't pair with it."))
        }
        val answer: DeviceState? = withTimeoutOrNull(ANSWER_MS) {
            state.first { it == null || it.isPaired || it.pairState == PairState.UNPAIRED }
        }
        return done(
            when {
                answer?.isPaired == true -> Step.Paired
                answer == null && state.first() != null -> Step.Failed("${invite.name} didn't take the code. If it asks, accept there, or show a new code and scan it.")
                else -> Step.Failed("The code ran out or was already used. Show a new one on the PC and scan it.")
            },
        )
    }
}
