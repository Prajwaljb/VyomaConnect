//! The pairing QR code as a picture for the window: dark modules on white with a quiet zone, as
//! cameras read best whatever the window's theme. Each module is whole pixels, so it stays sharp.

use std::time::Duration;

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::MainWindow;
use crate::daemon::{Daemon, DbusDaemon};

/// How often the code on screen is renewed (the daemon's last ten minutes).
const RENEW: Duration = Duration::from_secs(5 * 60);

thread_local! {
    static RENEWING: slint::Timer = slint::Timer::default();
}

/// Keeps a fresh code on screen while a page shows one ([MainWindow]'s wants-code), and takes it
/// down after, so a code left in a screenshot can't pair anyone.
pub fn wire(ui: &MainWindow) {
    ui.on_wants_code_changed({
        let ui = ui.as_weak();
        move |on| {
            if on {
                fetch(ui.clone());
                let again = ui.clone();
                RENEWING.with(|t| t.start(slint::TimerMode::Repeated, RENEW, move || fetch(again.clone())));
            } else {
                RENEWING.with(|t| t.stop());
                if let Some(ui) = ui.upgrade() {
                    ui.set_pair_code_ready(false);
                }
                std::thread::spawn(|| {
                    let _ = DbusDaemon::connect().and_then(|d| d.end_pairing_code());
                });
            }
        }
    });
    // It may open on such a page.
    if ui.get_wants_code() {
        ui.invoke_wants_code_changed(true);
    }
}

fn fetch(ui: slint::Weak<MainWindow>) {
    std::thread::spawn(move || {
        let text = DbusDaemon::connect().and_then(|d| d.pairing_code());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(win) = ui.upgrade() else { return };
            match text {
                Ok(text) => match image(&text) {
                    Some(code) => {
                        win.set_pair_code(code);
                        win.set_pair_code_note("".into());
                        win.set_pair_code_ready(true);
                    }
                    None => {
                        win.set_pair_code_note("Couldn't draw the code. Pair from the list of nearby phones instead.".into());
                        win.set_pair_code_ready(false);
                    }
                },
                Err(e) => {
                    // Say why there's no code, and try again shortly (the daemon restarting, the
                    // network coming back) rather than waiting for the next renewal.
                    let offline = format!("{e:#}").contains("offline");
                    win.set_pair_code_note(
                        if offline { "This PC isn't on a network. Connect it to the same Wi-Fi as your phone." } else { "Couldn't make a code. Trying again…" }.into(),
                    );
                    win.set_pair_code_ready(false);
                    slint::Timer::single_shot(RETRY, move || {
                        if ui.upgrade().is_some_and(|w| w.get_wants_code()) {
                            fetch(ui.clone());
                        }
                    });
                }
            }
        });
    });
}

/// How soon a code that couldn't be made is tried again.
const RETRY: Duration = Duration::from_secs(4);

/// Pixels per module: a ~60-module code comes out near 520 px, drawn at a third of that.
const SCALE: usize = 8;
/// Modules of white around it, which scanners need to find it (the tile under it adds more).
const QUIET: usize = 2;
const DARK: Rgba8Pixel = Rgba8Pixel { r: 0x1c, g: 0x1c, b: 0x1e, a: 0xff };
const LIGHT: Rgba8Pixel = Rgba8Pixel { r: 0xff, g: 0xff, b: 0xff, a: 0xff };

pub fn image(text: &str) -> Option<Image> {
    // Low error correction: a screen doesn't smudge, and fewer, bigger squares read from farther away.
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L).ok()?;
    let modules = code.width();
    let colors = code.to_colors();
    let side = (modules + 2 * QUIET) * SCALE;
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(side as u32, side as u32);
    let pixels = buffer.make_mut_slice();
    for (i, pixel) in pixels.iter_mut().enumerate() {
        let (x, y) = ((i % side) / SCALE, (i / side) / SCALE);
        let dark = (QUIET..QUIET + modules).contains(&x)
            && (QUIET..QUIET + modules).contains(&y)
            && colors[(y - QUIET) * modules + (x - QUIET)] == qrcode::Color::Dark;
        *pixel = if dark { DARK } else { LIGHT };
    }
    Some(Image::from_rgba8(buffer))
}

#[cfg(test)]
mod tests {
    #[test]
    fn draws_a_pairing_code() {
        let text = "https://github.com/Prajwaljb/VyomaConnect/releases/latest#pair?v=1&id=0123456789abcdef0123456789abcdef&name=My%20PC&fp=1b9f3d973c838fe866cb806b72ef20448ccb55c320a514b35e9e03808d00069b&s=00112233445566778899aabbccddeeff&a=192.168.1.20,10.0.0.5&p=1716";
        let image = super::image(text).expect("fits a QR code");
        let size = image.size();
        assert_eq!(size.width, size.height);
        assert!(size.width >= 400, "big enough to scale down sharply: {}", size.width);
    }
}
