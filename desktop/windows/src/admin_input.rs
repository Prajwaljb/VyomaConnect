//! Control admin windows (This PC, off until set up): the phone's touchpad and keyboard reaching
//! windows of programs running as administrator (Task Manager, an admin terminal, an installer),
//! which Windows keeps a normal program's input out of (UIPI).
//!
//! Set up asks Windows for an administrator once (`--install-admin-input <user SID>`), which copies
//! this program under Program Files as `vyoma-input.exe` (a folder only administrators can change:
//! a task that ran the per-user copy would let any program swap it and run as administrator at the
//! next sign-in) and adds a scheduled task that starts it at this user's sign-in "with highest
//! privileges", as `--input-helper`. The helper does one thing: it replays mouse and keyboard
//! events (SendInput's records, checked to be nothing else) that this user's processes send it over
//! `\\.\pipe\vyoma-input-<SID>`. The agent itself stays a normal program; it sends its input there
//! while the helper answers, else through SendInput as before. The UAC prompt and the lock screen
//! (the secure desktop) stay out of reach, as for every program that isn't a system service.
//!
//! Only for an account that is an administrator: for a standard account, or with Windows 11's
//! Administrator protection, the task runs without administrator rights, and This PC says so.
//! The trade-off, said on This PC: while it's on, any program this user runs could also type into
//! admin windows through the helper (never into the UAC prompt).
//! Built and tested on CI and under Wine only (Wine has no integrity levels); not yet run on a real Windows PC.

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_KEYBOARD, INPUT_MOUSE, SendInput};

use crate::win::{self, wide};

pub const CHECK: &str = "Control admin windows";
pub const INSTALL_ARG: &str = "--install-admin-input";
pub const REMOVE_ARG: &str = "--remove-admin-input";
pub const HELPER_ARG: &str = "--input-helper";
/// The helper's first line: its protocol, and whether Windows gave it administrator rights.
const HELLO: &str = "vyoma-input 1";
/// Events in one batch at most (a typed paragraph is a few hundred).
const MAX_BATCH: u32 = 1024;
/// How long the agent waits before looking for a helper again that wasn't there.
const RETRY: Duration = Duration::from_secs(5);
/// The helper's copy under Program Files, and the marker in Vyoma's folder that it was set up.
const HELPER_EXE: &str = "vyoma-input.exe";
const MARKER: &str = "admin-input";
/// Owners a real helper's pipe can have: this user, or (an elevated token's default) Administrators.
const ADMINISTRATORS: &str = "S-1-5-32-544";

fn pipe_name(sid: &str) -> String {
    format!(r"\\.\pipe\vyoma-input-{sid}")
}

fn task_name(sid: &str) -> String {
    format!(r"\Vyoma\Admin input {sid}")
}

fn helper_path() -> PathBuf {
    crate::webcam::program_files().join("Vyoma").join(HELPER_EXE)
}

fn marker() -> PathBuf {
    win::data_dir().join(MARKER)
}

/// A SID as Windows writes it ("S-1-5-21-…"), and nothing else (it goes into a task's XML).
fn plain_sid(s: &str) -> bool {
    s.starts_with("S-1-") && s.len() < 200 && s[4..].split('-').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

// The agent's side.

struct Helper {
    pipe: File,
    elevated: bool,
}

struct Client {
    helper: Option<Helper>,
    next_try: Option<Instant>,
}

static CLIENT: Mutex<Client> = Mutex::new(Client { helper: None, next_try: None });

/// Sends [all] through the helper when it's there with administrator rights: Some(events Windows
/// took). None: no helper (the caller uses SendInput itself).
pub fn send(all: &[INPUT]) -> Option<u32> {
    let mut c = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    if c.helper.is_none() {
        if c.next_try.is_some_and(|t| Instant::now() < t) || !marker().is_file() {
            return None;
        }
        c.helper = connect().ok();
        if c.helper.is_none() {
            c.next_try = Some(Instant::now() + RETRY);
            return None;
        }
    }
    let helper = c.helper.as_mut()?;
    if !helper.elevated {
        return None;
    }
    match exchange(&mut helper.pipe, all) {
        Ok(n) => Some(n),
        Err(e) => {
            tracing::info!("admin input helper gone: {e}");
            c.helper = None;
            c.next_try = Some(Instant::now() + RETRY);
            None
        }
    }
}

fn exchange(pipe: &mut File, all: &[INPUT]) -> std::io::Result<u32> {
    for batch in all.chunks(MAX_BATCH as usize) {
        let mut frame = Vec::with_capacity(4 + std::mem::size_of_val(batch));
        frame.extend_from_slice(&(batch.len() as u32).to_le_bytes());
        // SAFETY: INPUT is plain data; its bytes are what the helper reads back into INPUTs.
        frame.extend_from_slice(unsafe { std::slice::from_raw_parts(batch.as_ptr().cast::<u8>(), std::mem::size_of_val(batch)) });
        pipe.write_all(&frame)?;
    }
    let mut took = 0;
    for _ in all.chunks(MAX_BATCH as usize) {
        let mut n = [0u8; 4];
        pipe.read_exact(&mut n)?;
        took += u32::from_le_bytes(n);
    }
    Ok(took)
}

/// The helper's pipe, if this user's helper (not another account's look-alike) holds it.
fn connect() -> Result<Helper> {
    let sid = crate::control::user_sid()?;
    let mut pipe = std::fs::OpenOptions::new().read(true).write(true).open(pipe_name(&sid)).context("no helper")?;
    let owner = pipe_owner(&pipe).context("the helper's pipe has no owner")?;
    if owner != sid && owner != ADMINISTRATORS {
        bail!("another account holds the helper's pipe ({owner}): not using it");
    }
    let mut hello = [0u8; 64];
    let mut got = 0;
    while got < hello.len() {
        let n = pipe.read(&mut hello[got..got + 1])?;
        if n == 0 {
            bail!("the helper closed");
        }
        got += 1;
        if hello[got - 1] == b'\n' {
            break;
        }
    }
    let line = String::from_utf8_lossy(&hello[..got]).trim().to_owned();
    let elevated = match line.strip_prefix(HELLO) {
        Some(" high") => true,
        Some(" normal") => false,
        _ => bail!("not a helper this Vyoma knows ({line}): Set up again"),
    };
    if !elevated {
        tracing::info!("admin input helper runs without administrator rights");
    }
    Ok(Helper { pipe, elevated })
}

/// The owner of the object behind [f], as "S-1-…".
fn pipe_owner(f: &File) -> Option<String> {
    use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, GetSecurityInfo, SE_KERNEL_OBJECT};
    use windows_sys::Win32::Security::OWNER_SECURITY_INFORMATION;
    // SAFETY: the owner points into the descriptor Windows allocates, freed after copying it out.
    unsafe {
        let (mut owner, mut sd): (*mut core::ffi::c_void, PSECURITY_DESCRIPTOR) = (std::ptr::null_mut(), std::ptr::null_mut());
        let err = GetSecurityInfo(f.as_raw_handle() as HANDLE, SE_KERNEL_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut sd);
        if err != 0 || owner.is_null() {
            return None;
        }
        let mut s: *mut u16 = std::ptr::null_mut();
        let text = (ConvertSidToStringSidW(owner, &mut s) != 0).then(|| win::from_wide(s));
        LocalFree(s.cast());
        LocalFree(sd);
        text
    }
}

/// What This PC's row says.
pub fn check() -> Value {
    let set_up = marker().is_file();
    // None: no helper answering; Some(whether it has administrator rights).
    let helper = set_up
        .then(|| {
            let mut c = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
            if c.helper.is_none() {
                c.helper = connect().ok();
            }
            c.helper.as_ref().map(|h| h.elevated)
        })
        .flatten();
    let (state, detail, action) = match (set_up, helper) {
        (false, _) => (
            "off",
            "The phone's touchpad and keyboard can't reach windows running as administrator (Task Manager, an admin terminal). Set up asks Windows for an administrator once. While it's on, other programs you run could type into those windows too",
            "Set up",
        ),
        (true, Some(true)) => ("ok", "The touchpad and keyboard reach windows running as administrator (not the UAC prompt or the lock screen)", "Turn off"),
        (true, Some(false)) => ("warn", "Windows runs the helper without administrator rights: this account isn't an administrator, or Administrator protection is on", "Turn off"),
        (true, None) => ("warn", "Set up, but its helper isn't running. Sign out and in again, or set it up again", "Set up"),
    };
    json!({ "name": CHECK, "state": state, "detail": detail, "action": action })
}

/// Whether the helper is doing the input now (This PC's "How this PC does it").
pub fn in_use() -> bool {
    CLIENT.lock().unwrap_or_else(|e| e.into_inner()).helper.as_ref().is_some_and(|h| h.elevated)
}

/// This PC's Set up / Turn off: as an administrator (Windows asks), then remembered for this user.
pub fn toggle() -> Result<()> {
    let running = CLIENT.lock().unwrap_or_else(|e| e.into_inner()).helper.is_some();
    if marker().is_file() && running {
        return turn_off();
    }
    let sid = crate::control::user_sid()?;
    win::as_admin(&format!("{INSTALL_ARG} {sid}"), "admin windows control wasn't set up")?;
    std::fs::create_dir_all(win::data_dir())?;
    std::fs::write(marker(), HELLO)?;
    let mut c = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    (c.helper, c.next_try) = (None, None);
    Ok(())
}

/// Turn off, and the uninstall (`--uninstall`): this user's task and helper gone (Windows asks for
/// an administrator), if it was set up.
pub fn turn_off() -> Result<()> {
    if !marker().is_file() {
        return Ok(());
    }
    let sid = crate::control::user_sid()?;
    CLIENT.lock().unwrap_or_else(|e| e.into_inner()).helper = None;
    win::as_admin(&format!("{REMOVE_ARG} {sid}"), "admin windows control wasn't turned off")?;
    let _ = std::fs::remove_file(marker());
    Ok(())
}

// As an administrator.

/// `--install-admin-input <SID>`: the helper's copy under Program Files and this user's task, started now.
pub fn install(sid: &str) -> Result<()> {
    if !plain_sid(sid) {
        bail!("not a user: {sid}");
    }
    let exe = helper_path();
    std::fs::create_dir_all(exe.parent().context("no folder")?)?;
    // A running helper holds its file: ended first (it starts again below).
    let _ = schtasks(&["/End", "/TN", &task_name(sid)]);
    let mut copied = std::fs::copy(std::env::current_exe()?, &exe);
    for _ in 0..10 {
        if copied.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
        copied = std::fs::copy(std::env::current_exe()?, &exe);
    }
    copied.with_context(|| format!("couldn't copy the helper to {}", exe.display()))?;
    let xml = task_xml(sid, &exe.to_string_lossy());
    let file = std::env::temp_dir().join(format!("vyoma-input-{}.xml", std::process::id()));
    // Task Scheduler reads its XML as UTF-16 with a byte order mark.
    let bytes: Vec<u8> = std::iter::once(0xFEFFu16).chain(xml.encode_utf16()).flat_map(|u| u.to_le_bytes()).collect();
    std::fs::write(&file, bytes)?;
    let made = schtasks(&["/Create", "/TN", &task_name(sid), "/XML", &file.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(&file);
    made.context("Windows didn't add the task")?;
    schtasks(&["/Run", "/TN", &task_name(sid)]).context("Windows didn't start the helper")?;
    Ok(())
}

/// `--remove-admin-input <SID>`: this user's task gone, and the helper's copy once no user has one.
pub fn remove(sid: &str) -> Result<()> {
    if !plain_sid(sid) {
        bail!("not a user: {sid}");
    }
    let _ = schtasks(&["/End", "/TN", &task_name(sid)]);
    let _ = schtasks(&["/Delete", "/TN", &task_name(sid), "/F"]);
    let others = std::process::Command::new("schtasks").args(["/Query", "/FO", "CSV", "/NH"]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains(r"\Vyoma\Admin input ")).unwrap_or(true);
    if !others {
        let exe = helper_path();
        if std::fs::remove_file(&exe).is_err() && exe.exists() {
            use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
            // SAFETY: a NUL-terminated path; no new name: delete at restart.
            unsafe { MoveFileExW(wide(&exe.to_string_lossy()).as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
        }
    }
    Ok(())
}

fn schtasks(args: &[&str]) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("schtasks").args(args).creation_flags(CREATE_NO_WINDOW).output().context("no schtasks")?;
    if !out.status.success() {
        bail!("schtasks {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The task: at [sid]'s sign-in, [exe] `--input-helper` with that user's highest rights, for as long
/// as the session lasts, on battery too.
fn task_xml(sid: &str, exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>Vyoma</Author>
    <Description>Lets the phone's touchpad and keyboard in Vyoma reach windows running as administrator. Turned on and off on Vyoma's This PC page.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{sid}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>{HELPER_ARG}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        sid = xml_escape(sid),
        exe = xml_escape(exe),
    )
}

// The helper.

/// `--input-helper`: serves this user's processes until the session ends. One per session.
pub fn helper() -> Result<()> {
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
    // SAFETY: once at start; a named mutex kept for the process's life.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let m = CreateMutexW(std::ptr::null(), 1, wide(r"Local\VyomaInputHelper").as_ptr());
        if m.is_null() || windows_sys::Win32::Foundation::GetLastError() == windows_sys::Win32::Foundation::ERROR_ALREADY_EXISTS {
            return Ok(());
        }
    }
    let sid = crate::control::user_sid()?;
    let hello = format!("{HELLO} {}\n", if elevated() { "high" } else { "normal" });
    loop {
        let pipe = listen(&pipe_name(&sid), &sid)?;
        let hello = hello.clone();
        std::thread::spawn(move || {
            let _ = serve(pipe, &hello);
        });
    }
}

/// One instance of the pipe, once a client connects: owned by and open to this user only, from this
/// PC only, labelled so a normal (medium) program may write to it.
fn listen(name: &str, sid: &str) -> Result<File> {
    use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};
    let sddl = wide(&format!("O:{sid}D:P(A;;GA;;;{sid})S:(ML;;NW;;;ME)"));
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: a NUL-terminated SDDL; the descriptor is freed once the pipe has it.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) == 0 {
            bail!("no security descriptor: {}", std::io::Error::last_os_error());
        }
        let attrs = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
        let h = CreateNamedPipeW(
            wide(name).as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            64 * 1024,
            0,
            &attrs,
        );
        LocalFree(sd);
        if h == INVALID_HANDLE_VALUE {
            bail!("couldn't make the pipe: {}", std::io::Error::last_os_error());
        }
        if ConnectNamedPipe(h, std::ptr::null_mut()) == 0 && windows_sys::Win32::Foundation::GetLastError() != windows_sys::Win32::Foundation::ERROR_PIPE_CONNECTED {
            CloseHandle(h);
            bail!("no client: {}", std::io::Error::last_os_error());
        }
        Ok(File::from_raw_handle(h as _))
    }
}

/// One client: the hello, then batches of input events, each answered with how many Windows took.
fn serve(mut pipe: File, hello: &str) -> std::io::Result<()> {
    pipe.write_all(hello.as_bytes())?;
    loop {
        let mut n = [0u8; 4];
        pipe.read_exact(&mut n)?;
        let n = u32::from_le_bytes(n);
        if n == 0 || n > MAX_BATCH {
            return Ok(());
        }
        let mut bytes = vec![0u8; n as usize * size_of::<INPUT>()];
        pipe.read_exact(&mut bytes)?;
        let inputs = checked(&bytes);
        let took = match inputs {
            // SAFETY: an array of checked INPUTs and its length.
            Some(all) => unsafe { SendInput(all.len() as u32, all.as_ptr(), size_of::<INPUT>() as i32) },
            None => return Ok(()),
        };
        pipe.write_all(&took.to_le_bytes())?;
    }
}

/// The records as INPUTs if every one is a mouse or keyboard event (never a raw hardware one),
/// with the extra info cleared.
fn checked(bytes: &[u8]) -> Option<Vec<INPUT>> {
    let n = bytes.len() / size_of::<INPUT>();
    let mut all: Vec<INPUT> = Vec::with_capacity(n);
    for chunk in bytes.chunks_exact(size_of::<INPUT>()) {
        // SAFETY: INPUT is plain data of exactly this many bytes; unaligned read from the buffer.
        let mut input: INPUT = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast()) };
        match input.r#type {
            INPUT_MOUSE => input.Anonymous.mi.dwExtraInfo = 0,
            INPUT_KEYBOARD => input.Anonymous.ki.dwExtraInfo = 0,
            _ => return None,
        }
        all.push(input);
    }
    Some(all)
}

/// Whether Windows gave this process administrator rights.
fn elevated() -> bool {
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: the token opened for query and closed; a fixed-size answer.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenElevation, (&mut e as *mut TOKEN_ELEVATION).cast(), size_of::<TOKEN_ELEVATION>() as u32, &mut len) != 0;
        CloseHandle(token);
        ok && e.TokenIsElevated != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT_0, INPUT_HARDWARE, KEYBDINPUT, MOUSEINPUT};

    #[test]
    fn only_plain_sids_go_into_the_task() {
        assert!(plain_sid("S-1-5-21-3623811015-3361044348-30300820-1013"));
        for bad in ["S-1-5-21-1\"<x>", "S-1-", "S-1-5--1", "1-5-21", "S-1-5-21-1 /RU SYSTEM"] {
            assert!(!plain_sid(bad), "{bad}");
        }
        let xml = task_xml("S-1-5-21-1-2-3-1001", r"C:\Program Files\Vyoma\vyoma-input.exe");
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>") && xml.contains("<UserId>S-1-5-21-1-2-3-1001</UserId>"));
        assert!(xml.contains(r#"<Command>"C:\Program Files\Vyoma\vyoma-input.exe"</Command>"#) && xml.contains("<Arguments>--input-helper</Arguments>"));
        assert!(helper_path().ends_with(r"Vyoma\vyoma-input.exe"));
    }

    fn bytes_of(all: &[INPUT]) -> Vec<u8> {
        // SAFETY: plain data, read as its bytes.
        unsafe { std::slice::from_raw_parts(all.as_ptr().cast::<u8>(), std::mem::size_of_val(all)) }.to_vec()
    }

    #[test]
    fn the_helper_replays_only_mouse_and_keyboard() {
        let mouse = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 5, dy: -3, mouseData: 0, dwFlags: 1, time: 0, dwExtraInfo: 77 } } };
        let key = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0x41, wScan: 0, dwFlags: 0, time: 0, dwExtraInfo: 9 } } };
        let back = checked(&bytes_of(&[mouse, key])).unwrap();
        assert_eq!(back.len(), 2);
        // SAFETY: the members the types name.
        unsafe {
            assert_eq!((back[0].Anonymous.mi.dx, back[0].Anonymous.mi.dy, back[0].Anonymous.mi.dwExtraInfo), (5, -3, 0));
            assert_eq!((back[1].Anonymous.ki.wVk, back[1].Anonymous.ki.dwExtraInfo), (0x41, 0));
        }
        let hardware = INPUT { r#type: INPUT_HARDWARE, ..mouse };
        assert!(checked(&bytes_of(&[mouse, hardware])).is_none(), "nothing but mouse and keyboard");
    }

    /// The helper and the agent over a real pipe (on a test name): the hello, a batch, its answer.
    #[test]
    fn the_agent_talks_to_the_helper() {
        let sid = crate::control::user_sid().unwrap();
        let test_sid = format!("{sid}-test{}", std::process::id());
        let name = pipe_name(&test_sid);
        // The helper's own pipe (owner, access and label as it makes them) on a test name.
        let server = std::thread::spawn({
            let (sid, name) = (sid.clone(), name.clone());
            move || {
                let pipe = listen(&name, &sid).unwrap();
                // Said as an elevated helper would, so the agent sends.
                let _ = serve(pipe, &format!("{HELLO} high\n"));
            }
        });
        std::thread::sleep(Duration::from_millis(300));
        let mut pipe = std::fs::OpenOptions::new().read(true).write(true).open(pipe_name(&test_sid)).unwrap();
        let owner = pipe_owner(&pipe).unwrap();
        assert!(owner == sid || owner == ADMINISTRATORS, "{owner}");
        let mut hello = [0u8; 19];
        pipe.read_exact(&mut hello).unwrap();
        assert_eq!(&hello, b"vyoma-input 1 high\n");
        // A key-up of no key (vk 0): Windows takes it and nothing happens.
        let nothing = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: 0, dwFlags: 2, time: 0, dwExtraInfo: 0 } } };
        let took = exchange(&mut pipe, &[nothing]).unwrap();
        assert!(took <= 1);
        drop(pipe);
        server.join().unwrap();
    }
}
