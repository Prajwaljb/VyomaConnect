//! The window rendered offscreen to a PNG, with made-up phones: for checking the look without a
//! display (or anyone's screen). Slint's software renderer on a window of our own.

use std::rc::Rc;

use anyhow::{Context, Result};
use slint::Rgb8Pixel;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize};

use crate::daemon::{ActivityEntry, Check, Feature, Health, Permission, PhoneData};
use crate::{MainWindow, Theme};

/// The window's size, unless `--size WxH` says otherwise (the smallest it goes is 780x540).
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

pub fn run(path: &str, dark: bool, plain: bool, scene: &str, size: Option<(u32, u32)>, scroll: f32) -> Result<()> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let ui = MainWindow::new()?;
    ui.global::<Theme>().set_dark(dark);
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: SCALE });
    let (width, height) = size.unwrap_or((WIDTH, HEIGHT));
    let (w, h) = ((width as f32 * SCALE) as u32, (height as f32 * SCALE) as u32);
    window.set_size(PhysicalSize::new(w, h));
    match scene {
        "down" => crate::apply(&ui, None),
        "empty" | "setup-search" => crate::apply(&ui, Some(vec![])),
        _ => crate::apply(&ui, Some(phones(scene))),
    }
    if scene == "nearby" {
        ui.set_selected("c".into());
        crate::apply(&ui, Some(phones(scene)));
    }
    if scene == "unpair" {
        ui.set_confirm_unpair(true);
    }
    if scene == "add" {
        ui.set_selected("add".into());
    }
    // A made-up pairing code wherever one shows.
    let made_up = "https://github.com/Prajwaljb/VyomaConnect/releases/latest#pair?v=1&id=0123456789abcdef0123456789abcdef&name=Desk&fp=1b9f3d973c838fe866cb806b72ef20448ccb55c320a514b35e9e03808d00069b&s=00112233445566778899aabbccddeeff&a=192.168.1.20,10.0.0.5&p=1716";
    if let Some(code) = crate::qr::image(made_up) {
        ui.set_pair_code(code);
        ui.set_pair_code_ready(true);
    }
    if scene.starts_with("pc") {
        ui.set_selected(crate::THIS_PC.into());
        crate::apply(&ui, Some(phones(scene)));
        crate::show_health(&ui, &health(scene == "pc-setup"));
    }
    if scene.starts_with("setup") {
        ui.set_setup(true);
        crate::show_health(&ui, &health(scene == "setup-ready" || scene == "setup-moment"));
        if scene == "setup-moment" {
            crate::moment(&ui, "Pixel 9 Pro".into());
        }
    }
    if scene.starts_with("commands") {
        ui.set_selected(crate::COMMANDS.into());
        crate::apply(&ui, Some(phones(scene)));
        let c = |key: &str, name: &str, command: &str, fingerprint: bool, every_time: bool| crate::daemon::CommandData { key: key.into(), name: name.into(), command: command.into(), fingerprint, every_time };
        let list = if scene == "commands-empty" {
            vec![]
        } else {
            vec![
                c("1", "Start the backup", "~/bin/backup --quiet", false, false),
                c("2", "Empty the trash", "gio trash --empty", true, false),
                c("4", "Wipe the browser history", "~/bin/wipe-history", true, true),
                c("3", "Turn the screens off", "kscreen-doctor --dpms off", false, false),
            ]
        };
        crate::show_commands(&ui, &list);
        ui.on_check_command(move |key, name, command| crate::command_warning(&key, &name, &command, &list, &|p| p != "backupp").into());
        if scene == "commands-warn" {
            ui.set_edit_key("".into());
            ui.set_edit_name("Start the backup".into());
            ui.set_edit_command("backupp --quiet".into());
            ui.set_editing(true);
            ui.set_notice("Deleted \u{201c}Turn the screens off\u{201d}".into());
            ui.set_notice_action("Undo".into());
            ui.set_notice_on(true);
        }
        if scene == "commands-edit" {
            ui.set_edit_key("2".into());
            ui.set_edit_name("Empty the trash".into());
            ui.set_edit_command("gio trash --empty".into());
            ui.set_edit_fingerprint(true);
            ui.set_edit_every(true);
            ui.set_editing(true);
        }
    }
    if scene.starts_with("activity") {
        ui.set_selected(crate::ACTIVITY.into());
        crate::apply(&ui, Some(phones(scene)));
    }
    // Made up, around a fixed noon so the days read the same every time.
    let noon = chrono::TimeZone::with_ymd_and_hms(&chrono::Local, 2026, 11, 8, 12, 0, 0).single().context("no such time")?;
    let at = |mins_ago: i64| (noon - chrono::Duration::minutes(mins_ago)).timestamp();
    let ev = |mins_ago: i64, name: &str, kind: &str, text: &str| ActivityEntry { time: at(mins_ago), name: name.into(), kind: kind.into(), text: text.into() };
    // activity-late: the list arrives after the page opened, as it does from the daemon.
    let late = scene == "activity-late";
    let list = if scene == "activity" || late {
        vec![
            ev(4, "Pixel 9 Pro", "screenshot", "Took a screenshot"),
            ev(12, "Pixel 9 Pro", "files", "Copied \u{201c}Quarterly report for the whole team, final version two.pdf\u{201d} from this PC"),
            ev(38, "Work phone", "refused", "Refused: \u{201c}See and control this screen\u{201d} is off"),
            ev(95, "Pixel 9 Pro", "touchpad", "Was the touchpad"),
            ev(97, "Pixel 9 Pro", "files", "Copied \u{201c}Screen_Recording_20261108_103512_Chrome.mp4\u{201d} from this PC"),
            ev(60 * 20, "Pixel 9 Pro", "power", "Put the PC to sleep"),
            ev(60 * 21, "Pixel 9 Pro", "share", "Shared 12 files with this PC"),
            ev(60 * 22, "Pixel 9 Pro", "photo", "Took a photo with the camera"),
            ev(60 * 24 * 4, "Work phone", "webcam", "Was the webcam"),
            ev(60 * 24 * 4 + 5, "Work phone", "command", "Ran \u{201c}Lock screen\u{201d}"),
        ]
    } else {
        vec![]
    };
    crate::show_activity(&ui, if late { &[] } else { &list }, noon);
    if scene == "notice" {
        ui.set_notice("Opening the phone's files…".into());
        ui.set_notice_on(true);
    }
    // A made-up wallpaper (a dusk over water) unless [--plain], for the accent's glow instead.
    if !plain {
        let colors = [
            [0x1B, 0x2A, 0x4A], [0x2E, 0x3F, 0x6E], [0x5A, 0x4E, 0x7A], [0xD9, 0x7A, 0x4A],
            [0x22, 0x3A, 0x5C], [0x3B, 0x5C, 0x8A], [0x8A, 0x6A, 0x7E], [0xF0, 0xA0, 0x5A],
            [0x14, 0x30, 0x48], [0x1E, 0x4A, 0x66], [0x3A, 0x5E, 0x78], [0x6A, 0x70, 0x78],
            [0x0C, 0x1C, 0x2C], [0x10, 0x2A, 0x3C], [0x18, 0x34, 0x46], [0x22, 0x3A, 0x4A],
        ];
        crate::look::apply(&ui.global::<Theme>(), Some(&crate::look::Wallpaper { cols: 4, rows: 4, colors: colors.to_vec() }));
    }
    ui.show()?;
    // Draw once so pages start rising in, then again once every enter animation has settled.
    let mut pixels = vec![Rgb8Pixel::new(0, 0, 0); (w * h) as usize];
    slint::platform::update_timers_and_animations();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, w as usize);
    });
    if late {
        crate::show_activity(&ui, &list, noon);
        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, w as usize);
        });
    }
    // Laid out now: scroll, if asked (pages follow Snap.scroll as it changes).
    ui.global::<crate::Snap>().set_scroll(scroll);
    // A frame at the new scroll, so what follows it (the docked title) starts moving now.
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, w as usize);
    });
    std::thread::sleep(std::time::Duration::from_millis(1100));
    slint::platform::update_timers_and_animations();
    window.request_redraw();
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
    let names = ["Sound", "Volume", "Clipboard", "Notifications", "Phone files", "PC screen on the phone", "Phone on PC", "Phone as webcam", "Game controller over USB"];
    let features = names
        .iter()
        .map(|n| {
            let needs: Vec<String> = match (*n, missing) {
                ("Phone files", true) => vec!["sshfs".into()],
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
        notes.push("PC screen on the phone needs vah264enc, which comes from your graphics card's VA-API driver.".into());
    }
    let update = crate::daemon::Update { current: "0.3.0".into(), latest: missing.then(|| "0.4.0".into()), checks: true, can_update: true, ..Default::default() };
    Health { features, installable: missing, installing: false, checks, notes, update }
}

fn phones(scene: &str) -> Vec<PhoneData> {
    let phone = |id: &str, name: &str| PhoneData { id: id.into(), name: name.into(), can_ring: true, can_browse: true, can_share: true, ..Default::default() };
    let mut main = PhoneData { paired: true, linked: true, via: "wifi".into(), battery: Some(82), charging: true, ..phone("a", "Pixel 9 Pro") };
    let permit = |key: &str, name: &str, detail: &str, on: bool| Permission { key: key.into(), name: name.into(), detail: detail.into(), on };
    main.permits = vec![
        permit("screen", "See and control this screen", "Watch the screen live, use it by touch, take screenshots", true),
        permit("control", "Touchpad, keyboard and controller", "Move the pointer, type, paste, play games", true),
        permit("files", "Use this PC's files", "Browse your home folder from the phone, and save to it", true),
        permit("camera", "Take photos with this PC's camera", "Photos land on the phone", false),
        permit("webcam", "Be this PC's webcam and mic", "The phone's camera and microphone in calls here", true),
        permit("power", "Wake, unlock, sleep and shut down", "Also log out and restart", false),
    ];
    (main.rtt, main.since) = (Some(6), Some(chrono::Local::now().timestamp_millis() - (2 * 60 + 14) * 60_000));
    let away = PhoneData { paired: true, clip: "plug".into(), ..phone("b", "Work phone") };
    let mut near = PhoneData { linked: true, ..phone("c", "OnePlus 12") };
    if scene == "pairing" {
        near.code = "4F9A2C1B".into();
        near.asking = true;
    }
    main.clip = "on".into();
    if scene == "cable" {
        // Plugged in, waiting for the one-time clipboard grant.
        main.via = "cable".into();
        main.clip = "ask".into();
    }
    // Setup before pairing: nothing paired yet, one phone nearby (asking, in setup-pairing).
    if scene == "setup" || scene == "setup-pairing" {
        let mut first = PhoneData { linked: true, via: "wifi".into(), ..phone("a", "Pixel 9 Pro") };
        if scene == "setup-pairing" {
            first.code = "4F9A2C1B".into();
            first.asking = true;
        }
        return vec![first];
    }
    if scene == "setup-search" {
        return vec![];
    }
    vec![main, away, near]
}
