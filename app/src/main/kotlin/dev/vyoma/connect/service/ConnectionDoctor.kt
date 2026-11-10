package dev.vyoma.connect.service

import android.content.Context
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Build
import android.os.SystemClock
import dev.vyoma.connect.container
import dev.vyoma.connect.protocol.VLog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.ConnectException
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NoRouteToHostException
import java.net.Socket

/**
 * Why a paired PC is away, found by trying things rather than guessing: is the phone on a network
 * at all, behind a VPN, on the network the PC was last on, and does the PC answer at its last
 * address. Each cause comes in plain words with the one thing that fixes it.
 */
object ConnectionDoctor {
    enum class Fix { NONE, ALLOW_LOCAL_NETWORK, WIFI_SETTINGS, VPN_SETTINGS, USE_CABLE }

    data class Finding(val title: String, val detail: String, val fix: Fix = Fix.NONE)

    /** What knocking on the PC's last address said. */
    enum class Probe {
        /** Something took the connection: the PC is there, so it's dialled. */
        ANSWERS,
        /** Turned away at once: the PC is on, with nothing listening for Vyoma. */
        REFUSED,
        /** Refused by the PC's firewall: an instant "not allowed" rather than silence. */
        BLOCKED,
        /** No answer: asleep, off, or kept apart by the Wi-Fi. */
        SILENT,
        /** No address to try (never reached over a network), or no network to try it on. */
        NOT_TRIED,
    }

    data class Facts(
        val pcName: String,
        val onLan: Boolean,
        val vpn: Boolean,
        val localNetworkAllowed: Boolean,
        /** Whether the PC's last address is on the phone's network now; null when there's nothing to compare. */
        val sameNetwork: Boolean?,
        val probe: Probe,
    )

    /** The causes, likeliest first: the first is what a card leads with. */
    fun findings(f: Facts): List<Finding> = buildList {
        if (!f.localNetworkAllowed) add(
            Finding("Vyoma can't look on this network", "Android asks first. Allow it, and Vyoma finds ${f.pcName} by itself.", Fix.ALLOW_LOCAL_NETWORK),
        )
        if (!f.onLan) add(
            Finding("This phone isn't on Wi-Fi", "Join the same Wi-Fi as ${f.pcName}, or plug the phone into it with a USB cable.", Fix.WIFI_SETTINGS),
        )
        when (f.probe) {
            Probe.ANSWERS -> add(Finding("${f.pcName} answers: connecting", "Not connected in a few seconds? Open Vyoma on the PC."))
            Probe.REFUSED -> add(Finding("Vyoma isn't running on ${f.pcName}", "The PC is on and on this Wi-Fi, but Vyoma isn't answering there. Open Vyoma on the PC to start it again."))
            else -> Unit
        }
        if (f.sameNetwork == false && f.probe != Probe.ANSWERS && f.probe != Probe.REFUSED) add(
            Finding("Your phone and ${f.pcName} are on different networks", "Put both on the same Wi-Fi. A PC plugged into the router counts as the same network.", Fix.WIFI_SETTINGS),
        )
        if (f.probe == Probe.BLOCKED) add(
            Finding("${f.pcName}'s firewall is blocking Vyoma", "On the PC, open Vyoma's window, choose This PC, and click Open ports next to Firewall."),
        )
        if (f.vpn && f.probe != Probe.ANSWERS && f.probe != Probe.REFUSED) add(
            Finding("A VPN on this phone may be hiding ${f.pcName}", "Some VPNs block the local network. Turn it off for a moment, or allow local network access in its settings.", Fix.VPN_SETTINGS),
        )
        if (isEmpty() || f.probe == Probe.SILENT) add(
            Finding(
                "${f.pcName} doesn't answer",
                "It may be asleep or off. Some Wi-Fi, often guest networks, keeps devices apart: a USB cable gets round that.",
                Fix.USE_CABLE,
            ),
        )
    }

    /** Looks at the phone's network and knocks on the PC's last address (a second or two). */
    suspend fun examine(context: Context, pcId: String, pcName: String): Facts = withContext(Dispatchers.IO) {
        val app = context.applicationContext
        val connectivity = app.getSystemService(ConnectivityManager::class.java)
        @Suppress("DEPRECATION")
        val networks = connectivity.allNetworks.mapNotNull { n -> connectivity.getNetworkCapabilities(n)?.let { n to it } }
        val lan = networks.filter { (_, c) ->
            !c.hasTransport(NetworkCapabilities.TRANSPORT_VPN) &&
                (c.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || c.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET))
        }.map { it.first }
        val vpn = networks.any { (_, c) -> c.hasTransport(NetworkCapabilities.TRANSPORT_VPN) }
        val allowed = Build.VERSION.SDK_INT < 37 ||
            app.checkSelfPermission("android.permission.ACCESS_LOCAL_NETWORK") == PackageManager.PERMISSION_GRANTED
        val hub = (app as android.app.Application).container.obtainHub()
        val trusted = hub.trustStore.all().firstOrNull { it.deviceId == pcId }
        val last = trusted?.lastAddress?.let { a -> runCatching { InetAddress.getByName(a) }.getOrNull() }
            ?.takeUnless { it.isLoopbackAddress || it.isAnyLocalAddress || it.isMulticastAddress }
        val port = trusted?.lastPort ?: 1716
        val sameNetwork = last?.let { address -> lan.takeIf { it.isNotEmpty() }?.any { onSubnet(connectivity, it, address) } }
        val network = lan.firstOrNull { last != null && onSubnet(connectivity, it, last) } ?: lan.firstOrNull()
        val probe = if (last == null || network == null) Probe.NOT_TRIED else knock(network, last, port)
        if (probe == Probe.ANSWERS) runCatching { hub.dial(last!!, port) }
        VLog.i(TAG, "away: lan=${lan.isNotEmpty()} vpn=$vpn allowed=$allowed same=$sameNetwork probe=$probe")
        Facts(pcName, lan.isNotEmpty(), vpn, allowed, sameNetwork, probe)
    }

    private fun onSubnet(connectivity: ConnectivityManager, network: Network, address: InetAddress): Boolean =
        connectivity.getLinkProperties(network)?.linkAddresses.orEmpty().any { la ->
            val mine = la.address
            when {
                mine is Inet4Address && address is Inet4Address -> samePrefix(mine.address, address.address, la.prefixLength)
                // IPv6: the /64 the router hands out; link-local addresses are on every link.
                mine is Inet6Address && address is Inet6Address && !address.isLinkLocalAddress && !mine.isLinkLocalAddress ->
                    samePrefix(mine.address, address.address, minOf(la.prefixLength, 64))
                else -> false
            }
        }

    internal fun samePrefix(a: ByteArray, b: ByteArray, bits: Int): Boolean {
        if (a.size != b.size) return false
        for (i in 0 until bits.coerceIn(0, a.size * 8)) {
            val mask = 0x80 ushr (i % 8)
            if ((a[i / 8].toInt() and mask) != (b[i / 8].toInt() and mask)) return false
        }
        return true
    }

    /**
     * A plain TCP connection to the PC's port, over Wi-Fi even with a VPN up. A firewall that
     * refuses sends "not allowed" back at once; a PC that isn't there leaves the phone asking for
     * it for a few seconds first, so a quick "no route" is the firewall.
     */
    private fun knock(network: Network, address: InetAddress, port: Int): Probe {
        val started = SystemClock.elapsedRealtime()
        return try {
            Socket().use { s ->
                network.bindSocket(s)
                s.connect(InetSocketAddress(address, port), KNOCK_MS)
            }
            Probe.ANSWERS
        } catch (e: NoRouteToHostException) {
            if (SystemClock.elapsedRealtime() - started < QUICK_MS) Probe.BLOCKED else Probe.SILENT
        } catch (e: ConnectException) {
            val why = e.message.orEmpty()
            when {
                "ECONNREFUSED" in why -> Probe.REFUSED
                "EHOSTUNREACH" in why && SystemClock.elapsedRealtime() - started < QUICK_MS -> Probe.BLOCKED
                else -> Probe.SILENT
            }
        } catch (e: Exception) {
            Probe.SILENT
        }
    }

    private const val TAG = "Doctor"
    private const val KNOCK_MS = 2_500
    private const val QUICK_MS = 1_000L
}
