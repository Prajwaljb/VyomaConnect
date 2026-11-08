//! The window rendered offscreen to a PNG, with made-up phones: for checking the look without a
//! display (or anyone's screen). Slint's software renderer on a window of our own.

use std::rc::Rc;

use anyhow::{Context, Result};
use slint::Rgb8Pixel;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize};

use crate::daemon::{Check, Feature, Health, PhoneData};
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
    if scene.starts_with("pc") {
        ui.set_selected(crate::THIS_PC.into());
        crate::apply(&ui, Some(phones(scene)));
        crate::show_health(&ui, &health(scene == "pc-setup"));
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

fn health(missing: bool) -> Health {
    let names = ["Sound", "Volume", "Clipboard", "Notifications", "Browsing the phone", "Live screen", "Phone screen", "Phone as webcam", "Game controller over USB"];
    let features = names
        .iter()
        .map(|n| {
            let needs: Vec<String> = match (*n, missing) {
                ("Browsing the phone", true) => vec!["sshfs".into()],
                ("Phone as webcam", true) => vec!["v4l2loopback-dkms".into(), "a module setting".into()],
                _ => vec![],
            };
            Feature { name: (*n).into(), ready: needs.is_empty(), needs, unavailable: false }
        })
        .collect();
    let mut checks = vec![Check { name: "Firewall".into(), state: "ok".into(), detail: "On, and your phones get through on Wi-Fi".into() }];
    let mut notes = vec![];
    if missing {
        checks[0] = Check { name: "Firewall".into(), state: "info".into(), detail: "ufw is on. If phones don't show up on Wi-Fi, allow ports 1714 to 1764, TCP and UDP.".into() };
        checks.push(Check { name: "KDE Connect".into(), state: "warn".into(), detail: "Also running, so the USB cable, tethering and Bluetooth stay with it. Quit it to use them here.".into() });
        notes.push("Live screen needs vah264enc, which comes from your graphics card's VA-API driver.".into());
    }
    Health { features, installable: missing, installing: false, checks, notes }
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
