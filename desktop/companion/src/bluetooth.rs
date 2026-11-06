//! Bluetooth fallback: a tunnel to kdeconnectd, as the USB cable is. BlueZ hands us each RFCOMM
//! connection a bonded phone opens to our service, and we pipe it to kdeconnectd's TCP port, so the
//! phone's usual TLS link runs end to end and stock KDE Connect's own (beta, unencrypted past the
//! radio) Bluetooth backend can stay off. Used by the phone only while it has no other way to the PC.
//!
//! A second service carries the companion's own link the same way: the phone opens it once
//! kdeconnectd is up, and main.rs dials the phone through a local port we bridge to the radio. A
//! third takes the controller's frames directly, whatever the link is on: it's steadier than Wi-Fi.
//! A fourth does the same for the touchpad (pointer.rs).

use std::collections::HashMap;
use std::os::fd::OwnedFd as StdFd;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info, warn};
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value};
use zbus::{fdo, interface, Connection};

/// Must match BluetoothTunnel.SERVICE on the phone.
const SERVICE: &str = "6f1d3e52-8a47-4c1b-9d06-2be7a5c4f801";
/// Must match BluetoothTunnel.COMPANION_SERVICE on the phone.
const COMPANION_SERVICE: &str = "6f1d3e52-8a47-4c1b-9d06-2be7a5c4f802";
/// Must match BluetoothPadChannel.SERVICE on the phone.
const PAD_SERVICE: &str = "6f1d3e52-8a47-4c1b-9d06-2be7a5c4f803";
/// Must match BluetoothPointerChannel.SERVICE on the phone.
const POINTER_SERVICE: &str = "6f1d3e52-8a47-4c1b-9d06-2be7a5c4f804";
/// RFCOMM channels; the phone finds them from the service record. High, clear of headsets' and OBEX's usual ones.
const CHANNEL: u16 = 26;
const COMPANION_CHANNEL: u16 = 27;
const PAD_CHANNEL: u16 = 28;
const POINTER_CHANNEL: u16 = 29;
const PROFILE_PATH: &str = "/dev/vyoma/companion/bluetooth";
const COMPANION_PATH: &str = "/dev/vyoma/companion/bluetooth_companion";
const PAD_PATH: &str = "/dev/vyoma/companion/bluetooth_pad";
const POINTER_PATH: &str = "/dev/vyoma/companion/bluetooth_pointer";
const KDECONNECTD: &str = "127.0.0.1:1716";
/// A failed or dropped registration (BlueZ restarted, adapter came back) is retried this often.
const REGISTER_EVERY: Duration = Duration::from_secs(30);
/// How long main.rs gets to dial a port we bridged to the radio.
const BRIDGE_WAIT: Duration = Duration::from_secs(10);

struct Profile;

#[interface(name = "org.bluez.Profile1")]
impl Profile {
    fn release(&self) {
        debug!("bluetooth profile released");
    }

    async fn new_connection(&self, device: OwnedObjectPath, fd: OwnedFd, _properties: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        info!("bluetooth tunnel opened by {}", device.as_str());
        let fd: StdFd = fd.into();
        tokio::spawn(async move {
            match tunnel(fd).await {
                Ok(()) => info!("bluetooth tunnel closed"),
                Err(e) => warn!("bluetooth tunnel failed: {e:#}"),
            }
        });
        Ok(())
    }

    fn request_disconnection(&self, _device: OwnedObjectPath) {}
}

/// Tells main.rs the local port each phone's companion tunnel can be dialled on.
struct CompanionProfile(UnboundedSender<u16>);

#[interface(name = "org.bluez.Profile1")]
impl CompanionProfile {
    fn release(&self) {
        debug!("bluetooth companion profile released");
    }

    async fn new_connection(&self, device: OwnedObjectPath, fd: OwnedFd, _properties: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        info!("bluetooth companion tunnel opened by {}", device.as_str());
        let port = bridge(fd.into()).await.map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        let _ = self.0.send(port);
        Ok(())
    }

    fn request_disconnection(&self, _device: OwnedObjectPath) {}
}

/// The controller's frames, straight to the pad (see gamepad.rs).
struct PadProfile;

#[interface(name = "org.bluez.Profile1")]
impl PadProfile {
    fn release(&self) {
        debug!("bluetooth controller profile released");
    }

    async fn new_connection(&self, _device: OwnedObjectPath, fd: OwnedFd, _properties: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        let radio = radio_stream(fd.into()).map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        tokio::spawn(crate::gamepad::read_frames(radio, "Bluetooth", crate::gamepad::ECHO_EVERY));
        Ok(())
    }

    fn request_disconnection(&self, _device: OwnedObjectPath) {}
}

/// The touchpad's frames, straight to its portal session (see pointer.rs).
struct PointerProfile;

#[interface(name = "org.bluez.Profile1")]
impl PointerProfile {
    fn release(&self) {
        debug!("bluetooth touchpad profile released");
    }

    async fn new_connection(&self, _device: OwnedObjectPath, fd: OwnedFd, _properties: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        let radio = radio_stream(fd.into()).map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        tokio::spawn(crate::pointer::read_frames(radio));
        Ok(())
    }

    fn request_disconnection(&self, _device: OwnedObjectPath) {}
}

fn radio_stream(fd: StdFd) -> Result<UnixStream> {
    // Any stream socket works here; only the address calls care that it isn't really a Unix one.
    let radio = std::os::unix::net::UnixStream::from(fd);
    radio.set_nonblocking(true)?;
    Ok(UnixStream::from_std(radio)?)
}

async fn tunnel(fd: StdFd) -> Result<()> {
    let mut radio = radio_stream(fd)?;
    let mut daemon = TcpStream::connect(KDECONNECTD).await.context("kdeconnectd isn't listening")?;
    daemon.set_nodelay(true)?;
    tokio::io::copy_bidirectional(&mut radio, &mut daemon).await?;
    Ok(())
}

/// A one-time local port piped to the radio, so a link runs over it like any TCP one. Whoever dials
/// still has to show a certificate kdeconnectd trusts.
async fn bridge(fd: StdFd) -> Result<u16> {
    let mut radio = radio_stream(fd)?;
    let local = TcpListener::bind("127.0.0.1:0").await?;
    let port = local.local_addr()?.port();
    tokio::spawn(async move {
        let piped = async {
            let (mut tcp, _) = tokio::time::timeout(BRIDGE_WAIT, local.accept()).await.context("nobody dialled")??;
            tcp.set_nodelay(true)?;
            tokio::io::copy_bidirectional(&mut radio, &mut tcp).await?;
            anyhow::Ok(())
        };
        match piped.await {
            Ok(()) => info!("bluetooth companion tunnel closed"),
            Err(e) => warn!("bluetooth companion tunnel failed: {e:#}"),
        }
    });
    Ok(port)
}

async fn register(system: &Connection, path: &str, service: &str, channel: u16) -> Result<()> {
    let options: HashMap<&str, Value> = HashMap::from([
        ("Name", Value::from("Vyoma")),
        ("Role", Value::from("server")),
        // Without a channel BlueZ advertises the service but never listens: the phone's connect just fails.
        ("Channel", Value::from(channel)),
        // Bonded phones only, and no extra prompt: TLS decides who gets further.
        ("RequireAuthentication", Value::from(true)),
        ("RequireAuthorization", Value::from(false)),
        ("AutoConnect", Value::from(false)),
    ]);
    let done = system
        .call_method(
            Some("org.bluez"),
            "/org/bluez",
            Some("org.bluez.ProfileManager1"),
            "RegisterProfile",
            &(ObjectPath::try_from(path)?, service, options),
        )
        .await;
    match done {
        // Still registered from last time: nothing to do.
        Err(e) if e.to_string().contains("AlreadyExists") => Ok(()),
        other => other.map(drop).map_err(Into::into),
    }
}

/// Keeps our services registered with BlueZ for as long as the companion runs. Each companion
/// tunnel's local port goes to [links].
pub async fn serve(links: UnboundedSender<u16>) {
    let system = match Connection::system().await {
        Ok(c) => c,
        Err(e) => return warn!("no Bluetooth fallback: no system bus: {e:#}"),
    };
    if let Err(e) = system.object_server().at(PROFILE_PATH, Profile).await {
        return warn!("no Bluetooth fallback: {e:#}");
    }
    if let Err(e) = system.object_server().at(COMPANION_PATH, CompanionProfile(links)).await {
        return warn!("no Bluetooth fallback: {e:#}");
    }
    if let Err(e) = system.object_server().at(PAD_PATH, PadProfile).await {
        return warn!("no Bluetooth fallback: {e:#}");
    }
    if let Err(e) = system.object_server().at(POINTER_PATH, PointerProfile).await {
        return warn!("no Bluetooth fallback: {e:#}");
    }
    let mut registered = false;
    loop {
        let result = match register(&system, PROFILE_PATH, SERVICE, CHANNEL).await {
            Ok(()) => match register(&system, COMPANION_PATH, COMPANION_SERVICE, COMPANION_CHANNEL).await {
                Ok(()) => match register(&system, PAD_PATH, PAD_SERVICE, PAD_CHANNEL).await {
                    Ok(()) => register(&system, POINTER_PATH, POINTER_SERVICE, POINTER_CHANNEL).await,
                    failed => failed,
                },
                failed => failed,
            },
            failed => failed,
        };
        match result {
            Ok(()) => {
                if !registered {
                    info!("bluetooth fallback ready");
                }
                registered = true;
            }
            Err(e) => {
                if registered {
                    warn!("bluetooth fallback lost: {e:#}");
                    registered = false;
                } else {
                    debug!("bluetooth fallback not available: {e:#}");
                }
            }
        }
        tokio::time::sleep(REGISTER_EVERY).await;
    }
}

/// This PC's Bluetooth address, for the phone to remember (it can't scan for it with Wi-Fi gone).
pub async fn address() -> Option<String> {
    let system = Connection::system().await.ok()?;
    let name = std::fs::read_dir("/sys/class/bluetooth").ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.contains(':')).min()?;
    let reply = system
        .call_method(
            Some("org.bluez"),
            format!("/org/bluez/{name}").as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.bluez.Adapter1", "Address"),
        )
        .await
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    String::try_from(value).ok()
}
