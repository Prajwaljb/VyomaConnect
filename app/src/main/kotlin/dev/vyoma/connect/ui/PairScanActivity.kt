package dev.vyoma.connect.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import dev.vyoma.connect.container
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.SmoothCornerShape
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.designsystem.VyomaTheme
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.protocol.crypto.QrInvite
import dev.vyoma.connect.service.CodePairing
import kotlinx.coroutines.launch
import java.util.concurrent.Executors

/**
 * Pairs by the code Vyoma's PC window shows: the camera reads it, then [CodePairing] dials the PC
 * and pairs on the code's proof, with nothing to compare or accept. Also opened by a
 * `vyoma://pair?...` link, which skips the camera.
 */
class PairScanActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Always over the dark camera picture, whatever the theme: light bar icons.
        val dark = androidx.activity.SystemBarStyle.dark(android.graphics.Color.TRANSPARENT)
        enableEdgeToEdge(statusBarStyle = dark, navigationBarStyle = dark)
        val linked = intent?.dataString?.let(QrInvite::parse)
        setContent {
            val theme by application.container.settings.theme.collectAsStateWithLifecycle(ThemeSettings())
            VyomaTheme(theme) { PairScan(linked, onClose = ::finish, onPaired = ::openHome) }
        }
    }

    private fun openHome() {
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        finish()
    }

    companion object {
        fun open(context: Context) = context.startActivity(Intent(context, PairScanActivity::class.java))
    }
}

private sealed interface ScanState {
    data object Looking : ScanState
    data class Confirm(val invite: QrInvite) : ScanState
    data class Working(val name: String, val step: CodePairing.Step) : ScanState
}

@Composable
private fun PairScan(linked: QrInvite?, onClose: () -> Unit, onPaired: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf<ScanState>(ScanState.Looking) }
    var cameraOn by remember { mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    var asked by remember { mutableStateOf(false) }
    // A link brings its own code: the camera only if that fails and they scan instead.
    var fromLink by remember { mutableStateOf(linked != null) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { cameraOn = it; asked = true }

    fun start(invite: QrInvite) {
        if (state != ScanState.Looking && state !is ScanState.Confirm) return
        haptics.perform(Haptic.Tap)
        state = ScanState.Working(invite.name, CodePairing.Step.Connecting)
        scope.launch {
            val end = CodePairing.pair(context, invite) { step -> state = ScanState.Working(invite.name, step) }
            when (end) {
                CodePairing.Step.Paired -> {
                    haptics.perform(Haptic.Success)
                    kotlinx.coroutines.delay(900)
                    onPaired()
                }
                is CodePairing.Step.Failed -> haptics.perform(Haptic.Failure)
                else -> Unit
            }
        }
    }
    LaunchedEffect(linked) { if (linked != null) state = ScanState.Confirm(linked) }
    LaunchedEffect(Unit) { if (!cameraOn && linked == null) ask.launch(Manifest.permission.CAMERA) }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        if (cameraOn && !fromLink) {
            CameraFeed(scanning = state == ScanState.Looking, onCode = ::start)
        }
        Viewfinder(Modifier.fillMaxSize())
        Row(Modifier.fillMaxWidth().statusBarsPadding().padding(16.dp)) {
            Box(Modifier.size(48.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.4f)), contentAlignment = Alignment.Center) {
                BounceIconButton(onClick = onClose) { Icon(Icons.Rounded.Close, "Close", tint = Color.White) }
            }
        }
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().navigationBarsPadding().padding(16.dp)
                .widthIn(max = 560.dp)
                .clip(SmoothCornerShape(28.dp)).background(Color(0xE6101114)).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            AnimatedContent(state, label = "scan") { s ->
                Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    when {
                        s is ScanState.Looking && !cameraOn -> {
                            Heading("Vyoma needs the camera")
                            Body("Only to read the code on your PC's screen. Nothing is recorded.")
                            Spacer(Modifier.height(8.dp))
                            PillButton("Allow camera", onClick = {
                                if (asked) runCatching { context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))) }
                                else ask.launch(Manifest.permission.CAMERA)
                            })
                        }
                        s is ScanState.Looking -> {
                            Heading("Scan the code on your PC")
                            Body("Open Vyoma on the PC: its window shows a code. Point the camera at it and your phone pairs by itself.")
                        }
                        s is ScanState.Confirm -> {
                            Heading("Pair with ${s.invite.name}?")
                            Body("Only if Vyoma on your PC sent you here. It would see your notifications, clipboard and files.")
                            Spacer(Modifier.height(8.dp))
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                PillButton("Pair", onClick = { start(s.invite) })
                                PillButton("Cancel", onClick = onClose, prominent = false)
                            }
                        }
                        s is ScanState.Working -> when (val step = s.step) {
                            CodePairing.Step.Connecting, CodePairing.Step.Pairing -> {
                                CircularProgressIndicator(Modifier.size(32.dp), color = Color.White, strokeWidth = 3.dp)
                                Heading(if (step == CodePairing.Step.Connecting) "Connecting to ${s.name}" else "Pairing with ${s.name}")
                                Body("Keep the phone on the same Wi-Fi as the PC.")
                            }
                            CodePairing.Step.Paired -> {
                                Icon(Icons.Rounded.CheckCircle, null, tint = Color(0xFF34C759), modifier = Modifier.size(40.dp))
                                Heading("Paired with ${s.name}")
                            }
                            is CodePairing.Step.Failed -> {
                                Icon(Icons.Rounded.ErrorOutline, null, tint = Color(0xFFFF9F0A), modifier = Modifier.size(40.dp))
                                Heading("Not paired")
                                Body(step.why)
                                Spacer(Modifier.height(8.dp))
                                PillButton("Scan again", onClick = {
                                    state = ScanState.Looking
                                    if (fromLink) {
                                        fromLink = false
                                        if (!cameraOn) ask.launch(Manifest.permission.CAMERA)
                                    }
                                })
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Heading(text: String) = Text(
    text, style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.SemiBold),
    color = Color.White, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth(),
)

@Composable
private fun Body(text: String) = Text(
    text, style = MaterialTheme.typography.bodyMedium,
    color = Color.White.copy(alpha = 0.85f), textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth(),
)

/** The frame to aim with: the picture dimmed around a clear rounded square, white corners on it. */
@Composable
private fun Viewfinder(modifier: Modifier) {
    Canvas(modifier.graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }) {
        val side = size.minDimension * 0.68f
        val topLeft = Offset((size.width - side) / 2, size.height * 0.40f - side / 2)
        val r = 28.dp.toPx()
        drawRect(Color.Black.copy(alpha = 0.45f))
        drawRoundRect(Color.Transparent, topLeft, Size(side, side), CornerRadius(r), blendMode = BlendMode.Clear)
        val arm = side * 0.16f
        val stroke = Stroke(width = 4.dp.toPx(), cap = StrokeCap.Round)
        val path = androidx.compose.ui.graphics.Path()
        fun corner(x: Float, y: Float, dx: Float, dy: Float) {
            path.moveTo(x, y + dy * (r + arm))
            path.lineTo(x, y + dy * r)
            path.quadraticTo(x, y, x + dx * r, y)
            path.lineTo(x + dx * (r + arm), y)
        }
        corner(topLeft.x, topLeft.y, 1f, 1f)
        corner(topLeft.x + side, topLeft.y, -1f, 1f)
        corner(topLeft.x, topLeft.y + side, 1f, -1f)
        corner(topLeft.x + side, topLeft.y + side, -1f, -1f)
        drawPath(path, Color.White, style = stroke)
    }
}

/** The back camera's picture, read for Vyoma's codes while [scanning]. */
@Composable
private fun CameraFeed(scanning: Boolean, onCode: (QrInvite) -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val owner = LocalLifecycleOwner.current
    val current = androidx.compose.runtime.rememberUpdatedState(onCode)
    val active = androidx.compose.runtime.rememberUpdatedState(scanning)
    val previewView = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }
    DisposableEffect(owner) {
        val worker = Executors.newSingleThreadExecutor()
        val reader = QrReader()
        val future = ProcessCameraProvider.getInstance(context)
        var provider: ProcessCameraProvider? = null
        future.addListener({
            val p = runCatching { future.get() }.getOrNull() ?: return@addListener
            provider = p
            val preview = Preview.Builder().build().also { it.setSurfaceProvider(previewView.surfaceProvider) }
            val analysis = ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
            analysis.setAnalyzer(worker) { image ->
                val invite = if (active.value) reader.read(image) else null
                image.close()
                if (invite != null) ContextCompat.getMainExecutor(context).execute { if (active.value) current.value(invite) }
            }
            runCatching {
                p.unbindAll()
                p.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
            }
        }, ContextCompat.getMainExecutor(context))
        onDispose {
            provider?.unbindAll()
            worker.shutdown()
        }
    }
    AndroidView({ previewView }, Modifier.fillMaxSize())
}

/** Finds a QR code in a camera frame's brightness plane and keeps it only if it's one of Vyoma's. */
private class QrReader {
    private val reader = MultiFormatReader().apply {
        setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE), DecodeHintType.TRY_HARDER to true))
    }

    fun read(image: ImageProxy): QrInvite? {
        val plane = image.planes.firstOrNull() ?: return null
        val buffer = plane.buffer.duplicate().apply { rewind() }
        val width = image.width
        val height = image.height
        val stride = plane.rowStride
        val bytes = ByteArray(buffer.remaining()).also(buffer::get)
        val source = runCatching { PlanarYUVLuminanceSource(bytes, stride, height, 0, 0, width, height, false) }.getOrNull() ?: return null
        val text = try {
            reader.decodeWithState(BinaryBitmap(HybridBinarizer(source))).text
        } catch (_: NotFoundException) {
            null
        } catch (_: Exception) {
            null
        } finally {
            reader.reset()
        }
        return text?.let(QrInvite::parse)
    }
}
