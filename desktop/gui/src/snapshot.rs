//! The window rendered offscreen to a PNG, with made-up phones: for checking the look without a
//! display (or anyone's screen). Slint's software renderer on a window of our own.

use std::rc::Rc;

use anyhow::{Context, Result};
use slint::Rgb8Pixel;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize};

use crate::daemon::PhoneData;
use crate::{MainWindow, Theme};

const WIDTH: u32 = 920;
const HEIGHT: u32 = 620;
/// Rendered at twice the size, as on a HiDPI screen.
const SCALE: f32 = 2.0;

struct Offscreen(Rc<MinimalSoftwareWindow>);

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

pub fn run(path: &str, dark: bool, scene: &str) -> Result<()> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let ui = MainWindow::new()?;
    ui.global::<Theme>().set_dark(dark);
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: SCALE });
    let (w, h) = ((WIDTH as f32 * SCALE) as u32, (HEIGHT as f32 * SCALE) as u32);
    window.set_size(PhysicalSize::new(w, h));
    match scene {
        "down" => crate::apply(&ui, None),
        "empty" => crate::apply(&ui, Some(vec![])),
        _ => crate::apply(&ui, Some(phones(scene))),
    }
    if scene == "nearby" {
        ui.set_selected("c".into());
        crate::apply(&ui, Some(phones(scene)));
    }
    if scene == "unpair" {
        ui.set_confirm_unpair(true);
    }
    if scene == "notice" {
        ui.set_notice("Opening the phone's files…".into());
    }
    ui.show()?;
    slint::platform::update_timers_and_animations();
    let mut pixels = vec![Rgb8Pixel::new(0, 0, 0); (w * h) as usize];
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, w as usize);
    });
    let file = std::fs::File::create(path).with_context(|| format!("creating {path}"))?;
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    png.write_header()?.write_image_data(&bytes)?;
    Ok(())
}

fn phones(scene: &str) -> Vec<PhoneData> {
    let phone = |id: &str, name: &str| PhoneData { id: id.into(), name: name.into(), can_ring: true, can_browse: true, can_share: true, ..Default::default() };
    let mut main = PhoneData { paired: true, linked: true, via: "wifi".into(), battery: Some(82), charging: true, ..phone("a", "Pixel 9 Pro") };
    let away = PhoneData { paired: true, ..phone("b", "Work phone") };
    let mut near = PhoneData { linked: true, ..phone("c", "OnePlus 12") };
    if scene == "pairing" {
        near.code = "4F9A2C1B".into();
        near.asking = true;
    }
    if scene == "cable" {
        main.via = "cable".into();
    }
    vec![main, away, near]
}
