//! Tells phones when this PC goes away and comes back, so they don't spend their battery looking
//! for a PC that's off: `vyoma.pc.leaving {why}` just before it shuts down or sleeps (logind holds
//! the shutdown or sleep a moment for it, through a delay inhibitor), and an announcement when it
//! wakes, to the network and to each phone's last address (a sleeping phone's Wi-Fi often drops
//! broadcasts, never a packet sent to it). Quitting says `stop` from main's signal handling.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::{MatchRule, MessageStream};

/// The packet: PC → phone, `why` = shutdown | sleep | stop.
pub const LEAVING: &str = "vyoma.pc.leaving";

const LOGIN: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
/// How long the packet gets to go out before the inhibitor is let go (logind waits up to its
/// InhibitDelayMaxSec, 5 s by default, for us).
pub const FLUSH: Duration = Duration::from_millis(400);

pub enum Change {
    Leaving(&'static str),
    Back,
}

/// Follows logind's PrepareForSleep and PrepareForShutdown until the system bus goes.
pub async fn watch(out: mpsc::UnboundedSender<Change>) {
    if let Err(e) = try_watch(&out).await {
        tracing::warn!("can't tell phones when this PC sleeps or shuts down: {e:#}");
    }
}

async fn try_watch(out: &mpsc::UnboundedSender<Change>) -> Result<()> {
    let conn = zbus::Connection::system().await?;
    let rule = |member| MatchRule::builder().msg_type(zbus::message::Type::Signal).sender(LOGIN).interface(MANAGER).member(member).path(PATH).build();
    let mut sleep = MessageStream::for_match_rule(rule("PrepareForSleep")?, &conn, None).await?;
    let mut shutdown = MessageStream::for_match_rule(rule("PrepareForShutdown")?, &conn, None).await?;
    let mut held = inhibit(&conn).await;
    loop {
        let (why, going) = tokio::select! {
            Some(m) = sleep.next() => ("sleep", m?.body().deserialize::<bool>()?),
            Some(m) = shutdown.next() => ("shutdown", m?.body().deserialize::<bool>()?),
            else => anyhow::bail!("the system bus closed"),
        };
        if going {
            tracing::info!("this PC is going to {why}: telling the phones");
            let _ = out.send(Change::Leaving(why));
            tokio::time::sleep(FLUSH).await;
            held = None;
        } else {
            if held.is_none() {
                held = inhibit(&conn).await;
            }
            if why == "sleep" {
                tracing::info!("awake again: telling the phones");
                let _ = out.send(Change::Back);
            }
        }
    }
}

/// A delay inhibitor: logind holds sleep and shutdown until it's closed (or a few seconds pass).
async fn inhibit(conn: &zbus::Connection) -> Option<zbus::OwnedFd> {
    let reply = conn
        .call_method(Some(LOGIN), PATH, Some(MANAGER), "Inhibit", &("sleep:shutdown", "Vyoma", "Telling your phones this PC is going away", "delay"))
        .await
        .map_err(|e| tracing::debug!("no inhibitor: {e}"))
        .ok()?;
    reply.body().deserialize().ok()
}

/// Each paired phone's last Wi-Fi address, kept so a PC that starts (or wakes) can tell it directly.
pub struct Addresses {
    file: PathBuf,
    all: BTreeMap<String, IpAddr>,
}

impl Addresses {
    pub fn load(dir: &Path) -> Self {
        let file = dir.join("addresses.json");
        let all = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Addresses { file, all }
    }

    /// [id] was last on [ip]; loopback (the cable, Bluetooth) isn't an address to call.
    pub fn seen(&mut self, id: &str, ip: IpAddr) {
        if ip.is_loopback() || ip.is_unspecified() || self.all.get(id) == Some(&ip) {
            return;
        }
        self.all.insert(id.to_owned(), ip);
        self.save();
    }

    pub fn forget(&mut self, id: &str) {
        if self.all.remove(id).is_some() {
            self.save();
        }
    }

    /// Where to announce besides the broadcast: each phone's last address, at the phones' UDP port.
    pub fn targets(&self, port: u16) -> Vec<SocketAddr> {
        self.all.values().map(|ip| SocketAddr::new(*ip, port)).collect()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(&self.all)
            && let Err(e) = vyoma_core::identity::write_private(&self.file, &json)
        {
            tracing::warn!("can't keep the phones' addresses: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_wifi_addresses_only() {
        let dir = std::env::temp_dir().join(format!("vyoma-addresses-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = Addresses::load(&dir);
        a.seen("phone", "127.0.0.1".parse().unwrap());
        assert!(a.targets(1716).is_empty());
        a.seen("phone", "192.168.1.20".parse().unwrap());
        assert_eq!(Addresses::load(&dir).targets(1716), vec!["192.168.1.20:1716".parse().unwrap()]);
        a.forget("phone");
        assert!(Addresses::load(&dir).targets(1716).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
