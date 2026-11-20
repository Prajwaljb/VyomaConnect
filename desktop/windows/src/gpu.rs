//! The live screen's colour conversion on the graphics card: Direct3D 11's video processor scales
//! the duplicated desktop and turns it into NV12 (BT.601, video range, as [crate::cast::to_nv12]),
//! so only the small NV12 picture comes back to memory for the encoder, instead of the whole
//! desktop in BGRA and a pass over every pixel on the CPU. The pointer is drawn with GDI on a copy
//! of the picture on the card. Where the card's driver has no video processor, or one that won't
//! make NV12, cast.rs converts on the CPU as before.

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_RESOURCE_MISC_GDI_COMPATIBLE, D3D11_TEX2D_VPIV,
    D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor,
    ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGISurface1;
use windows::Win32::Graphics::Gdi::HDC;
use windows::core::Interface;

/// The output's colour space: BT.601 (matrix bit clear), nominal range 16-235 (bits 4-5 = 1).
const STUDIO_601: u32 = 0x10;
/// The input's: full-range RGB (all bits clear).
const FULL_RGB: u32 = 0;

/// One size of desktop to one size of NV12, on one Direct3D device.
pub struct Converter {
    context: ID3D11DeviceContext,
    video: ID3D11VideoContext,
    processor: ID3D11VideoProcessor,
    /// The last picture as it came.
    clean: ID3D11Texture2D,
    /// The picture with the pointer drawn on: GDI can draw on it, the video processor reads it.
    canvas: ID3D11Texture2D,
    input: ID3D11VideoProcessorInputView,
    nv12: ID3D11Texture2D,
    output: ID3D11VideoProcessorOutputView,
    staging: ID3D11Texture2D,
    pub input_size: (u32, u32),
    pub output_size: (u32, u32),
}

/// A texture of [w]×[h] in [format] as [desc] says the rest.
fn texture(device: &ID3D11Device, desc: D3D11_TEXTURE2D_DESC) -> Result<ID3D11Texture2D> {
    let mut t = None;
    // SAFETY: a plain description, no initial data.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut t))? };
    t.context("no texture")
}

fn desc(format: DXGI_FORMAT, (w, h): (u32, u32)) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC { Width: w, Height: h, MipLevels: 1, ArraySize: 1, Format: format, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_DEFAULT, ..Default::default() }
}

impl Converter {
    /// For BGRA pictures of [input_size] to NV12 at [output_size] (both even), or why the card
    /// can't.
    pub fn new(device: &ID3D11Device, context: &ID3D11DeviceContext, input_size: (u32, u32), output_size: (u32, u32), fps: u32) -> Result<Converter> {
        if !output_size.0.is_multiple_of(2) || !output_size.1.is_multiple_of(2) {
            bail!("NV12 needs an even size");
        }
        // SAFETY: Direct3D calls on owned COM pointers; every view is made from a texture made here.
        unsafe {
            let vdevice: ID3D11VideoDevice = device.cast().context("this graphics driver has no video processing")?;
            let video: ID3D11VideoContext = context.cast().context("this graphics driver has no video processing")?;
            let rate = DXGI_RATIONAL { Numerator: fps, Denominator: 1 };
            let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: rate,
                InputWidth: input_size.0,
                InputHeight: input_size.1,
                OutputFrameRate: rate,
                OutputWidth: output_size.0,
                OutputHeight: output_size.1,
                Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
            };
            let enumerator: ID3D11VideoProcessorEnumerator = vdevice.CreateVideoProcessorEnumerator(&content).context("no video processor for this size")?;
            let takes = enumerator.CheckVideoProcessorFormat(DXGI_FORMAT_B8G8R8A8_UNORM).unwrap_or(0);
            let makes = enumerator.CheckVideoProcessorFormat(DXGI_FORMAT_NV12).unwrap_or(0);
            if takes & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32 == 0 || makes & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32 == 0 {
                bail!("the video processor doesn't turn BGRA into NV12");
            }
            let processor = vdevice.CreateVideoProcessor(&enumerator, 0).context("no video processor")?;

            let clean = texture(device, desc(DXGI_FORMAT_B8G8R8A8_UNORM, input_size))?;
            let canvas = texture(
                device,
                D3D11_TEXTURE2D_DESC { BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32, MiscFlags: D3D11_RESOURCE_MISC_GDI_COMPATIBLE.0 as u32, ..desc(DXGI_FORMAT_B8G8R8A8_UNORM, input_size) },
            )?;
            let nv12 = texture(device, D3D11_TEXTURE2D_DESC { BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32, ..desc(DXGI_FORMAT_NV12, output_size) })?;
            let staging = texture(
                device,
                D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, ..desc(DXGI_FORMAT_NV12, output_size) },
            )?;

            let mut input = None;
            let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
            };
            vdevice.CreateVideoProcessorInputView(&canvas, &enumerator, &in_desc, Some(&mut input)).context("the video processor won't read the picture")?;
            let mut output = None;
            let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            };
            vdevice.CreateVideoProcessorOutputView(&nv12, &enumerator, &out_desc, Some(&mut output)).context("the video processor won't write NV12")?;

            // The whole picture onto the whole output; no "enhancements" of the driver's own.
            let whole_in = RECT { left: 0, top: 0, right: input_size.0 as i32, bottom: input_size.1 as i32 };
            let whole_out = RECT { left: 0, top: 0, right: output_size.0 as i32, bottom: output_size.1 as i32 };
            video.VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            video.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&whole_in));
            video.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&whole_out));
            video.VideoProcessorSetOutputTargetRect(&processor, true, Some(&whole_out));
            video.VideoProcessorSetStreamColorSpace(&processor, 0, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: FULL_RGB });
            video.VideoProcessorSetOutputColorSpace(&processor, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: STUDIO_601 });
            video.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);

            Ok(Converter {
                context: context.clone(),
                video,
                processor,
                clean,
                canvas,
                input: input.context("no input view")?,
                nv12,
                output: output.context("no output view")?,
                staging,
                input_size,
                output_size,
            })
        }
    }

    /// Keeps [picture] (BGRA, the input size) as the latest.
    pub fn load(&self, picture: &ID3D11Texture2D) {
        // SAFETY: both textures are the input size in BGRA on this device.
        unsafe { self.context.CopyResource(&self.clean, picture) };
    }

    /// The latest picture with whatever [draw] puts on it through GDI, scaled and converted, into
    /// [out] as NV12 (the Y plane, then the interleaved UV plane, rows packed).
    pub fn convert(&self, draw: impl FnOnce(HDC), out: &mut Vec<u8>) -> Result<()> {
        // SAFETY: the canvas is GDI compatible, its DC given back before Direct3D uses it again;
        // the staging texture is mapped for reading and each row copied within its pitch.
        unsafe {
            self.context.CopyResource(&self.canvas, &self.clean);
            let surface: IDXGISurface1 = self.canvas.cast()?;
            let dc = surface.GetDC(false).context("no GDI on the picture")?;
            draw(dc);
            surface.ReleaseDC(None)?;

            let stream = D3D11_VIDEO_PROCESSOR_STREAM { Enable: true.into(), pInputSurface: std::mem::ManuallyDrop::new(Some(self.input.clone())), ..Default::default() };
            let streams = [stream];
            let blt = self.video.VideoProcessorBlt(&self.processor, &self.output, 0, &streams);
            let [mut stream] = streams;
            std::mem::ManuallyDrop::drop(&mut stream.pInputSurface);
            blt.context("the video processor failed")?;

            self.context.CopyResource(&self.staging, &self.nv12);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let (w, h) = (self.output_size.0 as usize, self.output_size.1 as usize);
            let pitch = mapped.RowPitch as usize;
            // NV12 mapped: h rows of Y, then h/2 rows of UV, each `pitch` apart.
            let src = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * (h + h / 2));
            out.clear();
            out.reserve(w * h * 3 / 2);
            for row in 0..h + h / 2 {
                out.extend_from_slice(&src[row * pitch..row * pitch + w]);
            }
            self.context.Unmap(&self.staging, 0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
    use windows::Win32::Graphics::Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11CreateDevice};
    use windows::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, FillRect};

    fn device(kind: D3D_DRIVER_TYPE) -> Option<(ID3D11Device, ID3D11DeviceContext)> {
        let (mut device, mut context) = (None, None);
        // SAFETY: a device of that kind, or nothing.
        unsafe { D3D11CreateDevice(None, kind, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context)).ok()? };
        Some((device?, context?))
    }

    fn under_wine() -> bool {
        // SAFETY: looking up an export by name.
        unsafe {
            use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
            let ntdll = GetModuleHandleW(windows_sys::w!("ntdll.dll"));
            !ntdll.is_null() && GetProcAddress(ntdll, windows_sys::s!("wine_get_version")).is_some()
        }
    }

    /// A picture of top white, bottom red, with a black square GDI draws, through the card: the
    /// same video-range colours the CPU makes, at a quarter of the size. Windows' own software
    /// card (WARP) has a video processor, so this runs on CI too; Wine has none.
    #[test]
    fn the_card_converts_as_the_cpu_does() {
        let devices = [D3D_DRIVER_TYPE_WARP, D3D_DRIVER_TYPE_HARDWARE].into_iter().filter_map(device).collect::<Vec<_>>();
        let (iw, ih, ow, oh) = (64u32, 64u32, 32u32, 32u32);
        let mut pixels = Vec::new();
        for y in 0..ih {
            for _ in 0..iw {
                pixels.extend_from_slice(if y < ih / 2 { &[255, 255, 255, 255] } else { &[0, 0, 255, 255] });
            }
        }
        let mut cpu = Vec::new();
        crate::cast::to_nv12(&pixels, iw as usize * 4, iw as usize, ih as usize, ow as usize, oh as usize, &mut cpu);
        let mut tested = 0;
        for (device, context) in &devices {
            let converter = match Converter::new(device, context, (iw, ih), (ow, oh), 30) {
                Ok(c) => c,
                Err(e) if under_wine() => {
                    eprintln!("no conversion on the card under Wine: {e:#}");
                    continue;
                }
                Err(e) => panic!("Windows' card can't convert: {e:#}"),
            };
            let data = D3D11_SUBRESOURCE_DATA { pSysMem: pixels.as_ptr().cast(), SysMemPitch: iw * 4, SysMemSlicePitch: 0 };
            let mut picture = None;
            // SAFETY: initial data of exactly the texture's size.
            unsafe { device.CreateTexture2D(&desc(DXGI_FORMAT_B8G8R8A8_UNORM, (iw, ih)), Some(&data), Some(&mut picture)).unwrap() };
            converter.load(&picture.unwrap());
            let mut out = Vec::new();
            converter
                .convert(
                    |dc| unsafe {
                        // SAFETY: a brush made, used and deleted on this DC.
                        let black = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0));
                        FillRect(dc, &RECT { left: 0, top: 0, right: 16, bottom: 16 }, black);
                        let _ = DeleteObject(black.into());
                    },
                    &mut out,
                )
                .unwrap();
            assert_eq!(out.len(), (ow * oh * 3 / 2) as usize);
            let y = |x: u32, y: u32| out[(y * ow + x) as usize] as i32;
            let uv = |x: u32, y: u32| {
                let i = (ow * oh + (y / 2) * ow + (x / 2) * 2) as usize;
                (out[i] as i32, out[i + 1] as i32)
            };
            let cpu_y = |x: u32, y: u32| cpu[(y * ow + x) as usize] as i32;
            let near = |a: i32, b: i32, what: &str| assert!((a - b).abs() <= 3, "{what}: card {a}, cpu {b}");
            near(y(4, 4), 16, "the square drawn with GDI is black");
            near(y(24, 4), cpu_y(24, 4), "white");
            near(y(24, 28), cpu_y(24, 28), "red");
            let (u, v) = uv(24, 28);
            let i = (ow * oh + 14 * ow + 24) as usize;
            near(u, cpu[i] as i32, "red's U");
            near(v, cpu[i + 1] as i32, "red's V");
            tested += 1;
        }
        assert!(tested > 0 || under_wine(), "no Direct3D device at all");
    }
}
