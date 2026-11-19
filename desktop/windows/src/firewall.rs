//! Windows Defender Firewall and Vyoma: phones reach this PC on TCP 1714-1764, announce
//! themselves on UDP 1716 and send the controller's states on UDP 1750-1764. The window's This PC page checks that the firewall lets them in
//! ([check]) and offers Let phones through ([open]), which asks for an administrator once (Windows'
//! own prompt) and runs this program as `--open-firewall` ([allow]): it takes out every rule for
//! Vyoma (including the blocks Windows makes when its first-run question is cancelled) and adds two
//! that let in only Vyoma, only on its ports.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, VARIANT_TRUE, WAIT_OBJECT_0};
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, NET_FW_ACTION_ALLOW, NET_FW_ACTION_BLOCK, NET_FW_IP_PROTOCOL_ANY, NET_FW_IP_PROTOCOL_TCP, NET_FW_IP_PROTOCOL_UDP,
    NET_FW_PROFILE2_ALL, NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_RULE_DIR_IN, NetFwPolicy2,
    NetFwRule,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Ole::IEnumVARIANT;
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{BSTR, HSTRING, IUnknown, Interface, PCWSTR};

pub const CHECK: &str = "Firewall";
pub const OPEN_ARG: &str = "--open-firewall";
/// Where phones knock: the TCP ports, the UDP announcement port and the controller's.
const TCP_PORTS: &str = "1714-1764";
const UDP_PORT: &str = "1716,1750-1764";
/// The port that must be open of each, for the check.
const PORT: u16 = 1716;
const TCP_RULE: &str = "Vyoma (TCP in)";
const UDP_RULE: &str = "Vyoma (UDP in)";

/// One inbound rule as the check needs it.
#[derive(Clone, Debug, Default)]
pub struct Rule {
    pub name: String,
    pub app: String,
    pub inbound: bool,
    pub allow: bool,
    pub block: bool,
    pub enabled: bool,
    pub profiles: i32,
    pub protocol: i32,
    pub ports: String,
}

#[derive(Debug, PartialEq)]
pub enum State {
    /// The firewall is off for the network this PC is on.
    Off,
    Open,
    /// Nothing lets phones in (Windows' question not answered yet, or answered for another network).
    Closed,
    /// A rule blocks Vyoma (its question was cancelled once).
    Blocked,
}

/// Whether [ports] ("*", "1714-1764", "1716,1739-1764") takes [port].
fn takes_port(ports: &str, port: u16) -> bool {
    ports.split(',').map(str::trim).any(|p| match p {
        "*" | "" => true,
        p => match p.split_once('-') {
            Some((a, b)) => a.trim().parse::<u16>().is_ok_and(|a| a <= port) && b.trim().parse::<u16>().is_ok_and(|b| port <= b),
            None => p.parse::<u16>() == Ok(port),
        },
    })
}

/// What the rules mean for phones on [current] (the networks' profile bits), with the firewall on
/// for [enabled] of them.
pub fn judge(rules: &[Rule], exe: &str, current: i32, enabled: i32) -> State {
    if current & enabled == 0 {
        return State::Off;
    }
    let live = |r: &&Rule| r.inbound && r.enabled && r.profiles & current != 0;
    let ours = |r: &Rule| r.app.eq_ignore_ascii_case(exe);
    if rules.iter().filter(live).any(|r| r.block && ours(r)) {
        return State::Blocked;
    }
    let lets_in = |protocol: i32| {
        rules.iter().filter(live).any(|r| {
            r.allow && (ours(r) || r.app.is_empty() || r.app == "*") && (r.protocol == protocol || r.protocol == NET_FW_IP_PROTOCOL_ANY.0) && takes_port(&r.ports, PORT)
        })
    };
    if lets_in(NET_FW_IP_PROTOCOL_TCP.0) && lets_in(NET_FW_IP_PROTOCOL_UDP.0) { State::Open } else { State::Closed }
}

fn policy() -> Result<INetFwPolicy2> {
    // SAFETY: this thread joins the multithreaded apartment (fine if it already has), then the
    // firewall's policy object.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER).context("no Windows Firewall here")
    }
}

fn rules(p: &INetFwPolicy2) -> Result<Vec<(Rule, INetFwRule)>> {
    let mut out = Vec::new();
    // SAFETY: walking the rules' enumerator one VARIANT at a time; each holds a rule object.
    unsafe {
        let all = p.Rules()?;
        let walk: IEnumVARIANT = all._NewEnum()?.cast()?;
        loop {
            let mut v = [VARIANT::default()];
            let mut got = 0u32;
            if walk.Next(&mut v, &mut got).is_err() || got == 0 {
                break;
            }
            let Ok(unknown) = IUnknown::try_from(&v[0]) else { continue };
            let Ok(r) = unknown.cast::<INetFwRule>() else { continue };
            let text = |b: windows::core::Result<BSTR>| b.map(|b| b.to_string()).unwrap_or_default();
            let action = r.Action().map(|a| a.0).unwrap_or(-1);
            let rule = Rule {
                name: text(r.Name()),
                app: text(r.ApplicationName()),
                inbound: r.Direction().is_ok_and(|d| d == NET_FW_RULE_DIR_IN),
                allow: action == NET_FW_ACTION_ALLOW.0,
                block: action == NET_FW_ACTION_BLOCK.0,
                enabled: r.Enabled().is_ok_and(|e| e.as_bool()),
                profiles: r.Profiles().unwrap_or(0),
                protocol: r.Protocol().unwrap_or(0),
                ports: text(r.LocalPorts()),
            };
            out.push((rule, r));
        }
    }
    Ok(out)
}

/// The firewall as it stands for phones (blocking: a few hundred rules are read).
pub fn state() -> Result<State> {
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let p = policy()?;
    // SAFETY: plain reads of the policy.
    let (current, enabled) = unsafe {
        let current = p.CurrentProfileTypes()?;
        let mut enabled = 0;
        for profile in [NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC] {
            if p.get_FirewallEnabled(profile).is_ok_and(|e| e.as_bool()) {
                enabled |= profile.0;
            }
        }
        (current, enabled)
    };
    let rules: Vec<Rule> = rules(&p)?.into_iter().map(|(r, _)| r).collect();
    Ok(judge(&rules, &exe, current, enabled))
}

/// The This PC check, or None where there's no firewall to ask (Wine).
pub fn check() -> Option<serde_json::Value> {
    let state = match state() {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("no firewall check: {e:#}");
            return None;
        }
    };
    let (ok, detail) = match state {
        State::Off => (true, "Windows Firewall is off on this network"),
        State::Open => (true, "Phones can reach this PC"),
        State::Closed => (false, "Windows Firewall may keep phones out. Let them through (Windows asks for an administrator)"),
        State::Blocked => (false, "Windows Firewall blocks Vyoma. Let phones through (Windows asks for an administrator)"),
    };
    Some(serde_json::json!({ "name": CHECK, "state": if ok { "ok" } else { "warn" }, "detail": detail, "action": if ok { "" } else { "Let phones through" } }))
}

/// Let phones through: this program again as an administrator (Windows asks), waiting for it.
pub fn open() -> Result<()> {
    let exe = HSTRING::from(std::env::current_exe()?.as_os_str());
    let (verb, args) = (HSTRING::from("runas"), HSTRING::from(OPEN_ARG));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the strings outlive the call; the process handle is waited on and closed.
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            if e.code() == ERROR_CANCELLED.to_hresult() {
                bail!("Windows asked for an administrator and was told no");
            }
            return Err(e).context("couldn't ask Windows for an administrator");
        }
        if info.hProcess.is_invalid() {
            bail!("the firewall change didn't start");
        }
        let waited = WaitForSingleObject(info.hProcess, 120_000);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if waited != WAIT_OBJECT_0 || code != 0 {
            bail!("the firewall wasn't changed");
        }
    }
    Ok(())
}

/// `--open-firewall` (as an administrator): Vyoma's rules afresh. Exit status 0 when done.
pub fn allow() -> Result<()> {
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let p = policy()?;
    // SAFETY: COM calls on the policy and new rule objects made here.
    unsafe {
        let all = p.Rules()?;
        // Every rule for this program, and our own by name (a moved program leaves old ones).
        let names: Vec<String> = rules(&p)?.into_iter().map(|(r, _)| r).filter(|r| r.app.eq_ignore_ascii_case(&exe) || r.name == TCP_RULE || r.name == UDP_RULE).map(|r| r.name).collect();
        for name in names {
            let _ = all.Remove(&BSTR::from(name.as_str()));
        }
        for (name, protocol, ports) in [(TCP_RULE, NET_FW_IP_PROTOCOL_TCP, TCP_PORTS), (UDP_RULE, NET_FW_IP_PROTOCOL_UDP, UDP_PORT)] {
            let rule: INetFwRule = CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER)?;
            rule.SetName(&BSTR::from(name))?;
            rule.SetDescription(&BSTR::from("Lets phones with Vyoma reach this PC"))?;
            rule.SetApplicationName(&BSTR::from(exe.as_str()))?;
            rule.SetProtocol(protocol.0)?;
            rule.SetLocalPorts(&BSTR::from(ports))?;
            rule.SetDirection(NET_FW_RULE_DIR_IN)?;
            rule.SetAction(NET_FW_ACTION_ALLOW)?;
            rule.SetProfiles(NET_FW_PROFILE2_ALL.0)?;
            rule.SetEnabled(VARIANT_TRUE)?;
            all.Add(&rule)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\sam\AppData\Local\Programs\Vyoma\vyoma-windows.exe";
    const PRIVATE: i32 = 2;
    const PUBLIC: i32 = 4;

    fn rule(app: &str, allow: bool, protocol: i32, ports: &str, profiles: i32) -> Rule {
        Rule { name: "r".into(), app: app.into(), inbound: true, allow, block: !allow, enabled: true, profiles, protocol, ports: ports.into() }
    }

    #[test]
    fn ports_are_read_as_windows_writes_them() {
        assert!(takes_port("*", 1716));
        assert!(takes_port("1714-1764", 1716));
        assert!(takes_port("80, 1716", 1716));
        assert!(!takes_port("1739-1764", 1716));
        assert!(!takes_port("RPC", 1716));
    }

    #[test]
    fn the_check_reads_the_rules_as_windows_applies_them() {
        let tcp = NET_FW_IP_PROTOCOL_TCP.0;
        let udp = NET_FW_IP_PROTOCOL_UDP.0;
        let ours = [rule(EXE, true, tcp, TCP_PORTS, 0x7fff_ffff), rule(EXE, true, udp, UDP_PORT, 0x7fff_ffff)];
        assert_eq!(judge(&ours, EXE, PRIVATE, PRIVATE | PUBLIC), State::Open);
        assert_eq!(judge(&[], EXE, PRIVATE, PUBLIC), State::Off, "off on this network");
        assert_eq!(judge(&ours[..1], EXE, PRIVATE, PRIVATE), State::Closed, "UDP missing");
        // Windows' own question answered for private networks only, on a public one.
        let asked = [rule(&EXE.to_uppercase(), true, NET_FW_IP_PROTOCOL_ANY.0, "*", PRIVATE)];
        assert_eq!(judge(&asked, EXE, PRIVATE, PRIVATE | PUBLIC), State::Open, "any protocol, any port, case aside");
        assert_eq!(judge(&asked, EXE, PUBLIC, PRIVATE | PUBLIC), State::Closed);
        // Cancelling Windows' question leaves blocks, which win over allows.
        let blocked = [ours[0].clone(), ours[1].clone(), rule(EXE, false, tcp, "*", PUBLIC | PRIVATE)];
        assert_eq!(judge(&blocked, EXE, PRIVATE, PRIVATE), State::Blocked);
        let disabled = [ours[0].clone(), ours[1].clone(), Rule { enabled: false, ..rule(EXE, false, tcp, "*", PRIVATE) }];
        assert_eq!(judge(&disabled, EXE, PRIVATE, PRIVATE), State::Open, "a disabled block doesn't count");
        let other = [rule(r"C:\other.exe", true, tcp, "*", PRIVATE), rule(r"C:\other.exe", true, udp, "*", PRIVATE)];
        assert_eq!(judge(&other, EXE, PRIVATE, PRIVATE), State::Closed, "another program's rule");
    }

    /// Whatever this PC has: Wine has no real firewall; it answers or says there's none.
    #[test]
    fn reads_the_firewall_or_says_why() {
        eprintln!("firewall: {:?}", state().map_err(|e| format!("{e:#}")));
    }
}
