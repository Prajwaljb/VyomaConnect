package dev.vyoma.connect.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import android.graphics.SurfaceTexture
import android.view.TextureView
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.material.icons.rounded.FlashlightOn
import androidx.compose.material.icons.rounded.ZoomIn
import androidx.compose.ui.Alignment
import androidx.compose.ui.viewinterop.AndroidView
import dev.vyoma.connect.designsystem.LiquidSlider
import kotlin.math.exp
import kotlin.math.ln
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Cameraswitch
import androidx.compose.material.icons.rounded.HighQuality
import androidx.compose.material.icons.rounded.Mic
import androidx.compose.material.icons.rounded.VideocamOff
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.container
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.SegmentedControl
import dev.vyoma.connect.designsystem.ThemeSettings
import dev.vyoma.connect.designsystem.VyomaSwitch
import dev.vyoma.connect.designsystem.VyomaTheme
import dev.vyoma.connect.webcam.PhoneWebcam

/**
 * The phone as the PC's webcam (and mic): pick the camera, mic and quality, then Start. It keeps
 * streaming after this screen closes (the link service holds the camera); Stop here ends it.
 * Opened from the Camera tile's long-press menu on Home.
 */
class WebcamActivity : ComponentActivity() {
    private val prefs by lazy { getSharedPreferences("webcam", MODE_PRIVATE) }

    private fun saved() = PhoneWebcam.Options(
        front = prefs.getBoolean("front", true),
        mic = prefs.getBoolean("mic", true),
        hd = prefs.getBoolean("hd", false),
    )

    private fun save(o: PhoneWebcam.Options) {
        prefs.edit().putBoolean("front", o.front).putBoolean("mic", o.mic).putBoolean("hd", o.hd).apply()
        PhoneWebcam.change(this, o)
    }

    private fun granted(p: String) = checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (!PhoneWebcam.status.value.on) PhoneWebcam.change(this, saved())
        setContent {
            val theme by application.container.settings.theme.collectAsStateWithLifecycle(ThemeSettings())
            val status by PhoneWebcam.status.collectAsStateWithLifecycle()
            val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
                if (granted(Manifest.permission.CAMERA)) {
                    val o = status.options.let { o -> if (o.mic && !granted(Manifest.permission.RECORD_AUDIO)) o.copy(mic = false) else o }
                    if (o != status.options) save(o)
                    PhoneWebcam.start(this, o)
                }
            }
            fun start() {
                val need = buildList {
                    if (!granted(Manifest.permission.CAMERA)) add(Manifest.permission.CAMERA)
                    if (status.options.mic && !granted(Manifest.permission.RECORD_AUDIO)) add(Manifest.permission.RECORD_AUDIO)
                }
                if (need.isEmpty()) PhoneWebcam.start(this, status.options) else ask.launch(need.toTypedArray())
            }
            VyomaTheme(theme) {
                CollapsingHeaderScaffold(
                    title = "Phone as webcam",
                    subtitle = subtitle(status),
                    navigation = { BounceIconButton(onClick = ::finish) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
                ) {
                    item(key = "go") {
                        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            PillButton(
                                if (status.on) "Stop" else "Start",
                                onClick = { if (status.on) PhoneWebcam.stop(this@WebcamActivity) else start() },
                                modifier = Modifier.fillMaxWidth(),
                                prominent = !status.on,
                                large = true,
                            )
                        }
                    }
                    if (status.on && status.size != null) {
                        item(key = "preview") { Preview(status.size!!) }
                        item(key = "zoom") {
                            SectionCard(Modifier.padding(top = 8.dp)) {
                                val range = status.zoomRange
                                val zoom by PhoneWebcam.zoom.collectAsStateWithLifecycle()
                                if (range.endInclusive > range.start) {
                                    ListRow(title = "Zoom", subtitle = "%.1f×".format(zoom) + if (range.start < 1f) " · below 1× is the ultra-wide" else "", icon = Icons.Rounded.ZoomIn)
                                    // Even steps feel even: the slider runs on the zoom's logarithm.
                                    val lo = ln(range.start); val hi = ln(range.endInclusive)
                                    LiquidSlider(
                                        value = { ((ln(PhoneWebcam.zoom.value) - lo) / (hi - lo)).coerceIn(0f, 1f) },
                                        onChange = { v -> PhoneWebcam.zoom.value = exp(lo + v * (hi - lo)) },
                                        onDone = {},
                                        fill = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, bottom = 14.dp),
                                    )
                                }
                                if (status.torchable) {
                                    val torch by PhoneWebcam.torch.collectAsStateWithLifecycle()
                                    ListRow(
                                        title = "Torch",
                                        subtitle = "Lights the room from the back of the phone",
                                        icon = Icons.Rounded.FlashlightOn,
                                        trailing = { VyomaSwitch(torch, { PhoneWebcam.torch.value = it }, contentDescription = "Torch") },
                                        onClick = { PhoneWebcam.torch.value = !torch },
                                    )
                                }
                            }
                        }
                    }
                    status.error?.let { err ->
                        item(key = "error") {
                            SectionCard(Modifier.padding(top = 8.dp)) {
                                ListRow(title = "The PC can't show the camera yet", subtitle = err, icon = Icons.Rounded.VideocamOff, iconTint = MaterialTheme.colorScheme.error)
                                if ("one-time setup" in err) SetupSteps()
                            }
                        }
                    }
                    item(key = "camera-label") { SectionLabel("Camera") }
                    item(key = "camera") {
                        SectionCard(Modifier.padding(top = 8.dp)) {
                            ListRow(
                                title = "Which camera",
                                subtitle = "The back camera is sharper; turn the phone round to face you",
                                icon = Icons.Rounded.Cameraswitch,
                            )
                            SegmentedControl(
                                listOf("Front", "Back"), if (status.options.front) 0 else 1,
                                { i -> save(status.options.copy(front = i == 0)) },
                                Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, bottom = 12.dp),
                            )
                            ListRow(
                                title = "Quality",
                                subtitle = "1080p looks better and needs a steadier connection",
                                icon = Icons.Rounded.HighQuality,
                            )
                            SegmentedControl(
                                listOf("720p", "1080p"), if (status.options.hd) 1 else 0,
                                { i -> save(status.options.copy(hd = i == 1)) },
                                Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, bottom = 12.dp),
                            )
                        }
                    }
                    item(key = "mic-label") { SectionLabel("Microphone") }
                    item(key = "mic") {
                        SectionCard(Modifier.padding(top = 8.dp)) {
                            ListRow(
                                title = "Send the microphone too",
                                subtitle = "Appears on the PC as \"Vyoma Phone Mic\"",
                                icon = Icons.Rounded.Mic,
                                trailing = {
                                    VyomaSwitch(status.options.mic, { on -> save(status.options.copy(mic = on)) }, contentDescription = "Microphone")
                                },
                                onClick = { save(status.options.copy(mic = !status.options.mic)) },
                            )
                        }
                    }
                    item(key = "tip") {
                        Text(
                            "In your video app, pick \"Vyoma Phone Camera\". Stand the phone sideways for a wide picture; " +
                                "upright works too, with bars at the sides. It keeps going with this screen closed.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp),
                        )
                    }
                }
            }
        }
    }

    companion object {
        fun open(context: Context) = context.startActivity(Intent(context, WebcamActivity::class.java))
    }
}

private fun subtitle(s: PhoneWebcam.Status): String = when {
    s.error != null && !s.on -> "Off"
    !s.on -> "Off"
    s.pc == null -> "Waiting for Vyoma's companion on the PC"
    s.camera -> "Live on ${s.pc}" + (if (s.mic) " · camera and mic" else " · camera")
    else -> "Starting on ${s.pc}…"
}

/**
 * What the camera sees, as the phone holds it (the PC turns the picture upright itself). A
 * TextureView: the camera's own transform turns its landscape frames for this portrait screen (and
 * mirrors the front camera), and unlike a SurfaceView it clips to the rounded shape and scrolls with
 * the list (a SurfaceView here showed a thin strip of picture, the rest black).
 */
@Composable
private fun Preview(size: android.util.Size) {
    Box(Modifier.fillMaxWidth().padding(vertical = 12.dp), contentAlignment = Alignment.Center) {
        AndroidView(
            factory = { context ->
                TextureView(context).apply {
                    surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                        private var surface: android.view.Surface? = null
                        override fun onSurfaceTextureAvailable(st: SurfaceTexture, w: Int, h: Int) {
                            st.setDefaultBufferSize(size.width, size.height)
                            surface = android.view.Surface(st).also(PhoneWebcam::attachPreview)
                        }
                        override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, w: Int, h: Int) {}
                        override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
                            PhoneWebcam.detachPreview()
                            surface?.release()
                            return true
                        }
                        override fun onSurfaceTextureUpdated(st: SurfaceTexture) {}
                    }
                }
            },
            modifier = Modifier.fillMaxWidth(0.56f).aspectRatio(size.height.toFloat() / size.width).clip(RoundedCornerShape(28.dp)),
        )
    }
}

@Composable
private fun SetupSteps() {
    Text(
        "Run once on the PC, in a terminal:\n\n" +
            "sudo install -Dm644 desktop/companion/webcam/vyoma-webcam.conf /etc/modprobe.d/\n" +
            "echo v4l2loopback | sudo tee /etc/modules-load.d/vyoma-webcam.conf\n" +
            "sudo modprobe v4l2loopback",
        style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
        color = MaterialTheme.colorScheme.onSurface,
        modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 14.dp),
    )
}
