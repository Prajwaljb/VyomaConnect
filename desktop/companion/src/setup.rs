//! Installs what the companion needs, so nobody has to read a list. It looks for what's missing at
//! start and offers one Install button; `--check` and `--setup` do the same from a terminal.
//!
//! The service can't gain privileges (NoNewPrivileges in its unit), so the button asks the user
//! manager to run pkexec outside it. The script run as root is built only from the constants here.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// Package managers this knows, in the order they're looked for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Distro {
    Pacman,
    Apt,
    Dnf,
}

impl Distro {
    fn detect() -> Option<Self> {
        [(Self::Pacman, "pacman"), (Self::Apt, "apt-get"), (Self::Dnf, "dnf")].into_iter().find(|(_, tool)| on_path(tool)).map(|(d, _)| d)
    }

    fn install_command(self, packages: &str) -> String {
        match self {
            Self::Pacman => format!("pacman -S --needed --noconfirm {packages}"),
            Self::Apt => format!("apt-get update && apt-get install -y {packages}"),
            Self::Dnf => format!("dnf install -y {packages}"),
        }
    }
}

enum Probe {
    /// A program on PATH.
    Tool(&'static str),
    /// A GStreamer plugin. By plugin, not element: the elements of `va` only exist with a GPU that
    /// can do them, and no package would fix that.
    Gst(&'static str),
    /// A kernel module that's either built in or can be loaded.
    Module(&'static str),
    /// GVfs's SFTP, which GTK file managers browse the phone through (Dolphin uses the mount).
    GvfsSftp,
    /// Nautilus's Python extensions, for "Send to" in its menu itself rather than its Scripts.
    NautilusPython,
}

struct Need {
    /// What goes without it, for the prompt.
    feature: &'static str,
    probe: Probe,
    pacman: &'static str,
    apt: &'static str,
    dnf: &'static str,
}

impl Need {
    /// Whether this session uses it: X11 has its own clipboard and screen capture, Wayland the
    /// data-control clipboard and the portal's PipeWire streams.
    fn applies(&self) -> bool {
        let x11 = crate::x11::active();
        match self.probe {
            Probe::Tool("wl-copy") | Probe::Gst("pipewire") => !x11,
            Probe::Gst("ximagesrc") => x11,
            _ => true,
        }
    }

    fn packages(&self, distro: Distro) -> &'static str {
        match distro {
            Distro::Pacman => self.pacman,
            Distro::Apt => self.apt,
            Distro::Dnf => self.dnf,
        }
    }

    fn met(&self) -> bool {
        match self.probe {
            Probe::Tool(tool) => on_path(tool),
            Probe::Gst(plugin) => quiet("gst-inspect-1.0", &[plugin]),
            Probe::Module(module) => quiet("modinfo", &["-F", "filename", module]),
            Probe::GvfsSftp => !gtk_file_manager() || gvfs_sftp(),
            Probe::NautilusPython => !on_path("nautilus") || nautilus_python(),
        }
    }
}

/// Nautilus can load Python extensions (python-nautilus), wherever the distro puts its libraries.
pub fn nautilus_python() -> bool {
    ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu"]
        .iter()
        .any(|d| std::path::Path::new(d).join("nautilus/extensions-4/libnautilus-python.so").exists())
}

const fn need(feature: &'static str, probe: Probe, pacman: &'static str, apt: &'static str, dnf: &'static str) -> Need {
    Need { feature, probe, pacman, apt, dnf }
}

/// Everything the companion runs, whichever build of a distro it's on. An empty package means the
/// distro has none we can name: it's reported, not installed.
const NEEDS: &[Need] = &[
    need("pairing", Probe::Tool("kdeconnect-cli"), "kdeconnect", "kdeconnect", "kde-connect"),
    need("sound", Probe::Tool("pactl"), "libpulse", "pulseaudio-utils", "pulseaudio-utils"),
    need("volume", Probe::Tool("wpctl"), "wireplumber", "wireplumber", "wireplumber"),
    need("phone's sound on this PC", Probe::Tool("pw-dump"), "pipewire", "pipewire-bin", "pipewire-utils"),
    need("clipboard", Probe::Tool("wl-copy"), "wl-clipboard", "wl-clipboard", "wl-clipboard"),
    need("notifications", Probe::Tool("notify-send"), "libnotify", "libnotify-bin", "libnotify"),
    need("opening files", Probe::Tool("xdg-open"), "xdg-utils", "xdg-utils", "xdg-utils"),
    need("updates", Probe::Tool("curl"), "curl", "curl", "curl"),
    need("monitor brightness", Probe::Tool("ddcutil"), "ddcutil", "ddcutil", "ddcutil"),
    need("USB", Probe::Tool("adb"), "android-tools", "adb", "android-tools"),
    need("wallpaper and camera", Probe::Tool("ffmpeg"), "ffmpeg", "ffmpeg", "ffmpeg-free"),
    need("phone files", Probe::Tool("sshfs"), "sshfs", "sshfs", "fuse-sshfs"),
    need("phone files", Probe::GvfsSftp, "gvfs", "gvfs-backends", "gvfs"),
    need("Send to in Files", Probe::NautilusPython, "python-nautilus", "python3-nautilus", "nautilus-python"),
    need("PC screen on the phone", Probe::Tool("gst-launch-1.0"), "gstreamer", "gstreamer1.0-tools", "gstreamer1"),
    need("PC screen on the phone", Probe::Gst("pipewire"), "gst-plugin-pipewire", "gstreamer1.0-pipewire", "pipewire-gstreamer"),
    need("PC screen on the phone", Probe::Gst("ximagesrc"), "gst-plugins-good", "gstreamer1.0-plugins-good", "gstreamer1-plugins-good"),
    need("PC screen on the phone", Probe::Gst("coreelements"), "gstreamer", "gstreamer1.0-tools", "gstreamer1"),
    need("PC screen on the phone", Probe::Gst("videoconvertscale"), "gst-plugins-base", "gstreamer1.0-plugins-base", "gstreamer1-plugins-base"),
    need("phone's screen on this PC", Probe::Gst("opengl"), "gst-plugins-base", "gstreamer1.0-gl", "gstreamer1-plugins-base"),
    need("video", Probe::Gst("videoparsersbad"), "gst-plugins-bad", "gstreamer1.0-plugins-bad", "gstreamer1-plugins-bad-free"),
    need("video", Probe::Gst("va"), "gst-plugin-va", "gstreamer1.0-plugins-bad", "gstreamer1-plugins-bad-free"),
    need("phone's screen on this PC", Probe::Gst("libav"), "gst-libav", "gstreamer1.0-libav", "gstreamer1-plugin-libav"),
    need("phone's sound on this PC", Probe::Gst("opus"), "gst-plugins-base", "gstreamer1.0-plugins-base", "gstreamer1-plugins-base"),
    need("phone's sound on this PC", Probe::Gst("app"), "gst-plugins-base", "gstreamer1.0-plugins-base", "gstreamer1-plugins-base"),
    // Comes with some kernels (CachyOS); built for the others, which needs their headers.
    need("phone as webcam", Probe::Module("v4l2loopback"), "v4l2loopback-dkms", "v4l2loopback-dkms", ""),
];

/// Whether folders open in a file manager built on GIO (Thunar, Nautilus, Nemo, Caja, PCManFM,
/// elementary Files), which browses the phone best through GVfs: they treat a FUSE mount as a
/// local disk and wait on it in their window. Dolphin and the rest get the mount.
pub fn gtk_file_manager() -> bool {
    const GIO: [&str; 6] = ["thunar", "nautilus", "nemo", "caja", "pcmanfm", "io.elementary.files"];
    let Ok(out) = std::process::Command::new("xdg-mime").args(["query", "default", "inode/directory"]).output() else { return false };
    let app = String::from_utf8_lossy(&out.stdout).to_lowercase();
    GIO.iter().any(|name| app.contains(name))
}

/// Whether GVfs can do SFTP here (its mount description, wherever the distro puts the daemons).
pub fn gvfs_sftp() -> bool {
    std::path::Path::new("/usr/share/gvfs/mounts/sftp.mount").exists()
}

/// Set by the desktop app, which pairs by itself and so needs no kdeconnectd.
static STANDALONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn standalone() {
    STANDALONE.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Whether [standalone] was called: the desktop app, not the companion beside kdeconnectd.
pub fn is_standalone() -> bool {
    STANDALONE.load(std::sync::atomic::Ordering::Relaxed)
}

/// The program the user runs: the companion, or the desktop app.
fn program() -> &'static str {
    if STANDALONE.load(std::sync::atomic::Ordering::Relaxed) { "vyoma-desktop" } else { "vyoma-companion" }
}

const UDEV_RULE: &str = include_str!("../udev/70-vyoma-usb.rules");
const UDEV_PATH: &str = "/etc/udev/rules.d/70-vyoma-usb.rules";
const WEBCAM_CONF: &str = include_str!("../webcam/vyoma-webcam.conf");
const WEBCAM_PATH: &str = "/etc/modprobe.d/vyoma-webcam.conf";
const WEBCAM_LOAD_PATH: &str = "/etc/modules-load.d/vyoma-webcam.conf";

/// What this PC lacks.
#[derive(Default, Debug, PartialEq)]
pub struct Missing {
    /// Package names, in order, once each.
    packages: Vec<&'static str>,
    /// Features nothing here can install: an unknown distro, or no package on this one.
    unavailable: Vec<&'static str>,
    features: Vec<&'static str>,
    udev: bool,
    webcam: bool,
}

impl Missing {
    pub fn nothing(&self) -> bool {
        self.packages.is_empty() && self.unavailable.is_empty() && !self.udev && !self.webcam
    }

    /// Whether the Install button could do anything.
    fn installable(&self) -> bool {
        !self.packages.is_empty() || self.udev || self.webcam
    }

    fn summary(&self) -> String {
        let mut all = self.features.clone();
        if self.udev {
            all.push("game controller over USB");
        }
        if self.webcam {
            all.push("phone as webcam");
        }
        let mut seen = Vec::new();
        all.retain(|f| {
            let new = !seen.contains(f);
            seen.push(*f);
            new
        });
        all.join(", ")
    }

    /// Remembered when declined: a new lack asks again, the same one doesn't.
    fn fingerprint(&self) -> String {
        format!("{:?} {} {}", self.packages, self.udev, self.webcam)
    }
}

pub fn check() -> Missing {
    let distro = Distro::detect();
    let mut missing = Missing::default();
    let standalone = STANDALONE.load(std::sync::atomic::Ordering::Relaxed);
    for need in NEEDS.iter().filter(|n| !(standalone && n.feature == "pairing") && n.applies()) {
        if need.met() {
            continue;
        }
        missing.features.push(need.feature);
        match distro.map(|d| need.packages(d)).filter(|p| !p.is_empty()) {
            Some(package) if !missing.packages.contains(&package) => missing.packages.push(package),
            Some(_) => {}
            None => missing.unavailable.push(need.feature),
        }
    }
    missing.udev = !Path::new(UDEV_PATH).exists() && !Path::new("/usr/lib/udev/rules.d/70-vyoma-usb.rules").exists();
    missing.webcam = !Path::new(WEBCAM_PATH).exists();
    missing
}

/// One feature as the desktop window shows it.
pub struct FeatureState {
    pub feature: &'static str,
    pub ready: bool,
    /// What would make it ready: packages, or a file the install writes.
    pub needs: Vec<&'static str>,
    /// Nothing here can install it (an unknown distro, no package on this one).
    pub unavailable: bool,
}

/// Every feature that needs something on this PC, in [NEEDS]' order, and whether Install can help.
pub struct Report {
    pub features: Vec<FeatureState>,
    pub installable: bool,
    /// Things no package fixes.
    pub notes: Vec<String>,
}

pub fn report() -> Report {
    let distro = Distro::detect();
    let standalone = STANDALONE.load(std::sync::atomic::Ordering::Relaxed);
    let mut features: Vec<FeatureState> = Vec::new();
    for need in NEEDS.iter().filter(|n| !(standalone && n.feature == "pairing") && n.applies()) {
        let i = match features.iter().position(|f| f.feature == need.feature) {
            Some(i) => i,
            None => {
                features.push(FeatureState { feature: need.feature, ready: true, needs: Vec::new(), unavailable: false });
                features.len() - 1
            }
        };
        if need.met() {
            continue;
        }
        let f = &mut features[i];
        f.ready = false;
        match distro.map(|d| need.packages(d)).filter(|p| !p.is_empty()) {
            Some(package) if !f.needs.contains(&package) => f.needs.push(package),
            Some(_) => {}
            None => f.unavailable = true,
        }
    }
    let missing = check();
    let udev = FeatureState { feature: "game controller over USB", ready: !missing.udev, needs: if missing.udev { vec!["a udev rule"] } else { vec![] }, unavailable: false };
    features.push(udev);
    if missing.webcam {
        match features.iter_mut().find(|f| f.feature == "phone as webcam") {
            Some(f) => {
                f.ready = false;
                f.needs.push("a module setting");
            }
            None => features.push(FeatureState { feature: "phone as webcam", ready: false, needs: vec!["a module setting"], unavailable: false }),
        }
    }
    Report { features, installable: missing.installable(), notes: notes() }
}

/// Advice: what a package can't fix.
fn notes() -> Vec<String> {
    let mut out = Vec::new();
    if on_path("gst-inspect-1.0") {
        for (element, what, work) in [("vah264enc", "PC screen on the phone", "encoding"), ("vah264dec", "Phone as webcam", "decoding")] {
            if !quiet("gst-inspect-1.0", &["--exists", element]) {
                // The part's name is for the log; the page says what to do in plain words.
                tracing::debug!("no {element} (GStreamer VA-API) for {what}");
                out.push(format!("{what} needs your graphics card's video driver for hardware {work}. Your system's driver settings can add it."));
            }
        }
    }
    out
}

/// Installs what's missing (one password prompt); whether it finished.
pub async fn install() -> bool {
    let Ok(missing) = tokio::task::spawn_blocking(check).await else { return false };
    if !missing.installable() {
        return true;
    }
    install_missing(&missing).await
}

/// The script run as root. Packages first, and it goes on without the webcam's module, whose build
/// can fail on its own. Files already there are left: someone may have merged the webcam's options
/// with OBS's.
fn script(missing: &Missing, distro: Option<Distro>) -> String {
    let mut out = String::from("set -u\nstatus=0\n");
    let (module, packages): (Vec<&str>, Vec<&str>) = missing.packages.iter().partition(|p| p.contains("v4l2loopback"));
    if let (Some(distro), false) = (distro, packages.is_empty()) {
        out += &format!("{} || status=1\n", distro.install_command(&packages.join(" ")));
    }
    if let (Some(distro), false) = (distro, module.is_empty()) {
        out += &format!("{} || echo 'The webcam module could not be installed; the rest still works.'\n", distro.install_command(&module.join(" ")));
    }
    if missing.udev {
        out += &write_file(UDEV_PATH, UDEV_RULE);
        out += "udevadm control --reload\nudevadm trigger --subsystem-match=usb --action=change\n";
    }
    if missing.webcam {
        out += &write_file(WEBCAM_PATH, WEBCAM_CONF);
        out += &write_file(WEBCAM_LOAD_PATH, "v4l2loopback\n");
        // Already loaded for something else, it keeps its options until it next loads.
        out += "modprobe v4l2loopback || true\n";
    }
    out += "exit $status\n";
    out
}

fn write_file(path: &str, content: &str) -> String {
    format!("[ -e {path} ] || cat > {path} <<'VYOMA_END'\n{content}VYOMA_END\n")
}

fn run_privileged(script: &str) -> Result<bool> {
    let status = if on_path("pkexec") {
        Command::new("pkexec").args(["sh", "-c", script]).status()
    } else {
        Command::new("sudo").args(["sh", "-c", script]).status()
    }
    .context("couldn't ask for the password (neither pkexec nor sudo works)")?;
    Ok(status.success())
}

/// `--check`: what's there and what isn't. Exit status says whether anything is missing.
pub fn print_check() -> bool {
    let missing = check();
    match Distro::detect() {
        Some(d) => println!("package manager: {d:?}"),
        None => println!("package manager: none known (pacman, apt-get, dnf)"),
    }
    if missing.nothing() {
        println!("nothing missing");
    } else {
        println!("missing for: {}", missing.summary());
        if !missing.packages.is_empty() {
            println!("packages: {}", missing.packages.join(" "));
        }
        if !missing.unavailable.is_empty() {
            println!("no package known here for: {}", missing.unavailable.join(", "));
        }
        if missing.udev {
            println!("file: {UDEV_PATH}");
        }
        if missing.webcam {
            println!("file: {WEBCAM_PATH}");
        }
    }
    // A package can't fix these, so they're advice rather than something missing.
    for note in notes() {
        println!("note: {note}");
    }
    missing.nothing()
}

/// `--setup`, in a terminal; `--dry-run` shows the script instead of running it.
pub fn run(dry_run: bool) -> Result<()> {
    let missing = check();
    if dry_run {
        print!("{}", script(&missing, Distro::detect()));
        return Ok(());
    }
    if !missing.installable() {
        print_check();
        return Ok(());
    }
    println!("Installing for: {}", missing.summary());
    if !run_privileged(&script(&missing, Distro::detect()))? {
        bail!("the install didn't finish");
    }
    print_check();
    Ok(())
}

/// At start: if something's missing, offer the button. Declining is remembered until the list changes.
pub async fn offer() {
    tokio::time::sleep(std::time::Duration::from_secs(20)).await; // past the login rush
    let (missing, declined_file) = match tokio::task::spawn_blocking(|| (check(), crate::identity::dir().join("setup-declined"))).await {
        Ok(found) => found,
        Err(_) => return,
    };
    if !missing.installable() {
        return;
    }
    let print = missing.fingerprint();
    if tokio::fs::read_to_string(&declined_file).await.is_ok_and(|said| said == print) {
        return;
    }
    let answer = tokio::process::Command::new("notify-send")
        .args(["--app-name=Vyoma", "--hint=string:desktop-entry:vyoma", "--icon=system-software-install", "--wait", "--action=install=Install", "--action=no=Not now", "--"])
        .arg("Vyoma needs a few things on this PC")
        .arg(crate::notice::escape(&format!("For: {}. Installing asks for your password once.", missing.summary())))
        .kill_on_drop(true)
        .output()
        .await;
    let Ok(answer) = answer else {
        tracing::warn!("can't ask to install ({}): run {} --setup", missing.summary(), program());
        return;
    };
    match String::from_utf8_lossy(&answer.stdout).trim() {
        "install" => {
            if install_missing(&missing).await {
                crate::notice::tell("emblem-success", "Vyoma is set up", "Everything it needs is installed.").await;
            } else {
                tracing::warn!("install didn't finish: {} --setup shows why", program());
                let next = if is_standalone() { "Open Vyoma and choose Install on This PC to try again." } else { "Vyoma offers it again the next time you log in." };
                crate::notice::tell("dialog-warning", "Vyoma's install didn't finish", next).await;
            }
        }
        "no" => {
            let _ = tokio::fs::write(&declined_file, print).await;
        }
        _ => {} // dismissed: asks again next start
    }
}

async fn install_missing(missing: &Missing) -> bool {
    let script = script(missing, Distro::detect());
    // Out of this service's sandbox: NoNewPrivileges would stop pkexec working at all.
    let status = tokio::process::Command::new("systemd-run")
        .args(["--user", "--wait", "--collect", "--quiet", &format!("--unit={}-setup", program()), "pkexec", "sh", "-c"])
        .arg(script)
        .status()
        .await;
    status.is_ok_and(|s| s.success())
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

fn quiet(program: &str, args: &[&str]) -> bool {
    Command::new(program).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing() -> Missing {
        Missing { packages: vec!["ffmpeg", "v4l2loopback-dkms"], features: vec!["camera"], udev: true, webcam: true, ..Default::default() }
    }

    #[test]
    fn the_webcam_module_failing_doesnt_fail_the_rest() {
        let script = script(&missing(), Some(Distro::Pacman));
        assert!(script.contains("pacman -S --needed --noconfirm ffmpeg || status=1"));
        assert!(script.contains("pacman -S --needed --noconfirm v4l2loopback-dkms || echo"));
    }

    #[test]
    fn existing_config_is_left_alone() {
        let script = script(&missing(), None);
        assert!(script.contains("[ -e /etc/modprobe.d/vyoma-webcam.conf ] || cat >"));
        assert!(script.contains("[ -e /etc/udev/rules.d/70-vyoma-usb.rules ] || cat >"));
        assert!(!script.contains("pacman"), "no package manager, no packages");
    }

    #[test]
    fn the_script_parses() {
        for distro in [Distro::Pacman, Distro::Apt, Distro::Dnf] {
            let ok = Command::new("sh").args(["-n", "-c", &script(&missing(), Some(distro))]).status().unwrap().success();
            assert!(ok, "{distro:?}");
        }
    }

    #[test]
    fn every_need_names_a_package_where_it_can() {
        for need in NEEDS {
            assert!(!need.pacman.is_empty() && !need.apt.is_empty(), "{}", need.feature);
            assert!(!need.pacman.contains(' ') && !need.apt.contains(' ') && !need.dnf.contains(' '), "{}", need.feature);
        }
    }

    #[test]
    fn installed_files_end_in_a_newline() {
        // The heredoc's terminator has to start its own line.
        assert!(UDEV_RULE.ends_with('\n') && WEBCAM_CONF.ends_with('\n'));
    }
}
