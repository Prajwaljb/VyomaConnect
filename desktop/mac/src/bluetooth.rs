//! Bluetooth on the Mac, as on Linux and Windows: RFCOMM services under the phone's UUIDs, each
//! published with an SDP record (IOBluetooth), for the link without Wi-Fi (piped to the agent's own
//! TCP port, as if the phone had dialled it) and the touchpad (core `pointer::read_frames`). Each
//! connection is piped to a loopback TCP pair: what the phone sends is written to it from
//! IOBluetooth's callbacks, what comes back is written to the phone on the radio's own thread
//! (IOBluetooth's objects belong to the run loop they were made on), one piece at a time.
//! The controller's service isn't published: a Mac has no virtual gamepad.
//! macOS asks once whether Vyoma may use Bluetooth (Vyoma.app says why: NSBluetoothAlwaysUsageDescription).
//! Built and type-checked for macOS on CI; not yet run on a Mac.

use serde_json::json;
use tokio::sync::mpsc;
use tracing::{debug, info};
use vyoma_core::link::{Sender, packet};
use vyoma_core::pointer::{BUTTONS, Frame};

use crate::input::{Act, Button, Pointer};

/// The link without Wi-Fi and the touchpad (the phone's own UUIDs; core pointer's for the touchpad).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const LINK: u128 = 0x6f1d3e52_8a47_4c1b_9d06_2be7a5c4f801;
pub const POINTER: u128 = 0x6f1d3e52_8a47_4c1b_9d06_2be7a5c4f804;

const BUTTON_OF: [Button; BUTTONS] = [Button::Left, Button::Right, Button::Middle];

/// "AA:BB:CC:DD:EE:FF" from macOS's "aa-bb-cc-dd-ee-ff"; None if it isn't an address.
fn format_address(text: &str) -> Option<String> {
    let parts: Vec<&str> = text.trim().split(['-', ':']).collect();
    let ok = parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    (ok && parts.iter().any(|p| *p != "00")).then(|| parts.join(":").to_uppercase())
}

/// A UUID as SDP writes it: 16 bytes, most significant first.
fn uuid_bytes(uuid: u128) -> [u8; 16] {
    uuid.to_be_bytes()
}

/// The touchpad over Bluetooth for one phone until it closes: frames from the radio into pointer moves.
pub async fn touchpad(
    sender: Sender,
    name: String,
    tx: mpsc::Sender<Frame>,
    mut rx: mpsc::Receiver<Frame>,
) {
    let refusal = if !radio::ready() {
        Some("This Mac's Bluetooth is off or missing")
    } else if !crate::input::trusted() {
        crate::input::ask();
        Some("Allow Vyoma in System Settings > Privacy & Security > Accessibility, then try again")
    } else {
        None
    };
    if let Some(error) = refusal {
        info!("{name}: no touchpad over Bluetooth: {error}");
        let _ = sender
            .send(&packet("vyoma.pointer.ready", json!({ "error": error })))
            .await;
        return;
    }
    let Some((_open, key)) = vyoma_core::pointer::open(tx) else {
        return;
    };
    if sender
        .send(&packet("vyoma.pointer.ready", json!({ "key": key })))
        .await
        .is_err()
    {
        return;
    }
    let mut pointer = Pointer::default();
    let mut held = [false; BUTTONS];
    while let Some(frame) = rx.recv().await {
        let acts = match frame {
            Frame::KeepAwake => continue,
            Frame::Move(dx, dy) => vec![Act::Move(dx, dy)],
            Frame::Scroll(dx, dy) => vec![Act::Scroll(dx, dy)],
            Frame::Button(b, down) => {
                held[b] = down;
                vec![Act::Button(BUTTON_OF[b], down)]
            }
            // Nothing stays pressed after the radio drops or the page closes.
            Frame::StreamEnded | Frame::Close => {
                let up: Vec<Act> = held
                    .iter_mut()
                    .enumerate()
                    .filter_map(|(b, h)| {
                        std::mem::take(h).then(|| Act::Button(BUTTON_OF[b], false))
                    })
                    .collect();
                let _ = pointer.handle(&up);
                if frame == Frame::Close {
                    return;
                }
                continue;
            }
        };
        if pointer.handle(&acts).is_err() {
            debug!("touchpad: macOS won't let Vyoma move the pointer");
        }
    }
}

pub use radio::{address, start};

#[cfg(not(target_os = "macos"))]
mod radio {
    pub fn ready() -> bool {
        false
    }
    pub fn start(_tcp_port: u16, _rt: tokio::runtime::Handle) {}
    pub fn address() -> Option<String> {
        None
    }
}

#[cfg(target_os = "macos")]
mod radio {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::Message;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send, sel};
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSNumber, NSString};
    use objc2_io_bluetooth::{
        IOBluetoothHostController, IOBluetoothRFCOMMChannel, IOBluetoothRFCOMMChannelDelegate,
        IOBluetoothSDPServiceRecord, IOBluetoothUserNotification,
        IOBluetoothUserNotificationChannelDirection,
    };
    use tracing::{info, warn};

    use super::{LINK, POINTER, format_address, uuid_bytes};

    static READY: AtomicBool = AtomicBool::new(false);
    /// Radio off at start: looked at again this often.
    const RETRY: Duration = Duration::from_secs(30);

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRunLoopGetCurrent() -> *const c_void;
        fn CFRunLoopRunInMode(
            mode: *const c_void,
            seconds: f64,
            return_after_source_handled: u8,
        ) -> i32;
        fn CFRunLoopPerformBlock(
            rl: *const c_void,
            mode: *const c_void,
            block: &block2::DynBlock<dyn Fn()>,
        );
        fn CFRunLoopWakeUp(rl: *const c_void);
        static kCFRunLoopDefaultMode: *const c_void;
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Service {
        Link,
        Pointer,
    }

    pub fn ready() -> bool {
        READY.load(Ordering::Relaxed)
    }

    /// This Mac's radio address, for the phone to keep (with Wi-Fi gone it can't look for the Mac).
    pub fn address() -> Option<String> {
        // SAFETY: IOBluetooth's own controller object, read once.
        let text = unsafe { IOBluetoothHostController::defaultController()?.addressAsString()? };
        format_address(&text.to_string())
    }

    /// The services, on a thread of their own with its run loop, for as long as the agent runs.
    pub fn start(tcp_port: u16, rt: tokio::runtime::Handle) {
        if !explained() {
            info!(
                "Bluetooth: off outside Vyoma.app (macOS stops a program that asks without saying why)"
            );
            return;
        }
        let _ = std::thread::Builder::new()
            .name("vyoma-bluetooth".into())
            .spawn(move || {
                loop {
                    if address().is_some() {
                        match publish(tcp_port, rt.clone()) {
                            Ok(published) => {
                                READY.store(true, Ordering::Relaxed);
                                info!("Bluetooth: the link and the touchpad are listening");
                                serve(published);
                            }
                            Err(e) => warn!("Bluetooth: {e}"),
                        }
                    }
                    READY.store(false, Ordering::Relaxed);
                    std::thread::sleep(RETRY);
                }
            });
    }

    /// Run from Vyoma.app, whose Info.plist says why it uses Bluetooth: without that, macOS stops
    /// the process the moment it touches the radio.
    fn explained() -> bool {
        objc2::rc::autoreleasepool(|_| {
            let key = NSString::from_str("NSBluetoothAlwaysUsageDescription");
            objc2_foundation::NSBundle::mainBundle()
                .objectForInfoDictionaryKey(&key)
                .is_some()
        })
    }

    /// The published records and their open notifications: dropped together.
    struct Published {
        records: Vec<Retained<IOBluetoothSDPServiceRecord>>,
        notes: Vec<Retained<IOBluetoothUserNotification>>,
        _listener: Retained<Listener>,
    }

    impl Drop for Published {
        fn drop(&mut self) {
            // SAFETY: our own notifications and records, ended once.
            unsafe {
                for n in &self.notes {
                    n.unregister();
                }
                for r in &self.records {
                    r.removeServiceRecord();
                }
            }
        }
    }

    /// The run loop until the radio goes (checked every few seconds).
    fn serve(published: Published) {
        loop {
            // SAFETY: this thread's own run loop, in its default mode.
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 5.0, 0) };
            if address().is_none() {
                break;
            }
        }
        drop(published);
    }

    fn publish(tcp_port: u16, rt: tokio::runtime::Handle) -> Result<Published, String> {
        let listener = Listener::new(ListenerIvars {
            tcp_port,
            rt,
            services: RefCell::new(HashMap::new()),
        });
        let mut published = Published {
            records: Vec::new(),
            notes: Vec::new(),
            _listener: listener.clone(),
        };
        for (service, uuid, name) in [
            (Service::Link, LINK, "Vyoma link"),
            (Service::Pointer, POINTER, "Vyoma touchpad"),
        ] {
            let dict = record(uuid, name);
            // SAFETY: the same object; IOBluetooth takes the dictionary untyped.
            let dict: &NSDictionary = unsafe { &*Retained::as_ptr(&dict).cast::<NSDictionary>() };
            // SAFETY: a service dictionary as IOBluetooth documents it; the record is removed in Drop.
            let record = unsafe {
                IOBluetoothSDPServiceRecord::publishedServiceRecordWithDictionary(Some(dict))
            }
            .ok_or("macOS didn't publish the service")?;
            let mut channel = 0u8;
            // SAFETY: an out-param for the channel macOS gave the record.
            if unsafe { record.getRFCOMMChannelID(&mut channel) } != 0 {
                return Err("the service has no RFCOMM channel".into());
            }
            published.records.push(record);
            listener
                .ivars()
                .services
                .borrow_mut()
                .insert(channel, service);
            // SAFETY: our listener answers opened:channel:; the notification is unregistered in Drop.
            let note = unsafe {
                IOBluetoothRFCOMMChannel::registerForChannelOpenNotifications_selector_withChannelID_direction(
                    Some(&listener),
                    Some(sel!(opened:channel:)),
                    channel,
                    IOBluetoothUserNotificationChannelDirection::Incoming,
                )
            }
            .ok_or("macOS didn't take the channel's notification")?;
            published.notes.push(note);
        }
        Ok(published)
    }

    /// The SDP record: the service's UUID, L2CAP and an RFCOMM channel (macOS picks a free one),
    /// public browse group, its name.
    fn record(uuid: u128, name: &str) -> Retained<NSDictionary<NSString, AnyObject>> {
        let class_ids = NSArray::from_retained_slice(&[NSData::with_bytes(&uuid_bytes(uuid))]);
        let l2cap = NSArray::from_retained_slice(&[NSData::with_bytes(&[0x01, 0x00])]);
        let channel = NSDictionary::<NSString, AnyObject>::from_slices(
            &[
                &*NSString::from_str("DataElementType"),
                &*NSString::from_str("DataElementSize"),
                &*NSString::from_str("DataElementValue"),
            ],
            &[
                &*NSNumber::new_u8(1),
                &*NSNumber::new_u8(1),
                &*NSNumber::new_u8(1),
            ],
        );
        let rfcomm =
            NSArray::<AnyObject>::from_slice(&[&*NSData::with_bytes(&[0x00, 0x03]), &*channel]);
        let protocols = NSArray::<AnyObject>::from_slice(&[&*l2cap, &*rfcomm]);
        let browse = NSArray::from_retained_slice(&[NSData::with_bytes(&[0x10, 0x02])]);
        let keys = [
            "0001 - ServiceClassIDList",
            "0004 - ProtocolDescriptorList",
            "0005 - BrowseGroupList*",
            "0100 - ServiceName*",
        ]
        .map(NSString::from_str);
        let name = NSString::from_str(name);
        let values: [&AnyObject; 4] = [&class_ids, &protocols, &browse, &name];
        NSDictionary::from_slices(&keys.each_ref().map(|k| &**k), &values)
    }

    struct ListenerIvars {
        tcp_port: u16,
        rt: tokio::runtime::Handle,
        services: RefCell<HashMap<u8, Service>>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "VyomaBluetoothListener"]
        #[ivars = ListenerIvars]
        struct Listener;

        impl Listener {
            #[unsafe(method(opened:channel:))]
            fn opened(&self, _note: &IOBluetoothUserNotification, channel: &IOBluetoothRFCOMMChannel) {
                // SAFETY: reading the channel IOBluetooth handed us, on its run loop.
                let id = unsafe { channel.getChannelID() };
                let Some(service) = self.ivars().services.borrow().get(&id).copied() else { return };
                let ivars = self.ivars();
                if let Err(e) = connect(service, channel, ivars.tcp_port, &ivars.rt) {
                    warn!("Bluetooth: {e}");
                    // SAFETY: as above.
                    unsafe { channel.closeChannel() };
                }
            }
        }
    );

    impl Listener {
        fn new(ivars: ListenerIvars) -> Retained<Self> {
            let this = Self::alloc().set_ivars(ivars);
            // SAFETY: NSObject's init on our freshly allocated object.
            unsafe { msg_send![super(this), init] }
        }
    }

    struct PipeIvars {
        /// Toward the agent (or the touchpad's reader).
        tcp: RefCell<Option<TcpStream>>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "VyomaBluetoothPipe"]
        #[ivars = PipeIvars]
        struct Pipe;

        unsafe impl NSObjectProtocol for Pipe {}

        unsafe impl IOBluetoothRFCOMMChannelDelegate for Pipe {
            #[unsafe(method(rfcommChannelData:data:length:))]
            unsafe fn data(
                &self,
                _channel: Option<&IOBluetoothRFCOMMChannel>,
                data: *mut c_void,
                length: usize,
            ) {
                let mut tcp = self.ivars().tcp.borrow_mut();
                // SAFETY: IOBluetooth's buffer of [length] bytes, valid during the call.
                let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
                if tcp.as_mut().is_some_and(|t| t.write_all(bytes).is_err())
                    && let Some(t) = tcp.take()
                {
                    let _ = t.shutdown(Shutdown::Both);
                }
            }

            #[unsafe(method(rfcommChannelClosed:))]
            unsafe fn closed(&self, channel: Option<&IOBluetoothRFCOMMChannel>) {
                if let Some(t) = self.ivars().tcp.borrow_mut().take() {
                    let _ = t.shutdown(Shutdown::Both);
                }
                if let Some(c) = channel {
                    OPEN.with_borrow_mut(|open| open.remove(&key(c)));
                }
            }
        }
    );

    thread_local! {
        /// Each open channel with its delegate, kept alive until it closes (a channel doesn't keep
        /// its delegate), by the channel's address.
        static OPEN: RefCell<HashMap<usize, (Retained<IOBluetoothRFCOMMChannel>, Retained<Pipe>)>> = RefCell::new(HashMap::new());
    }

    fn key(channel: &IOBluetoothRFCOMMChannel) -> usize {
        channel as *const IOBluetoothRFCOMMChannel as usize
    }

    /// The loopback side for [service]: the agent's TCP port for the link, a touchpad reader for the pointer.
    fn connect(
        service: Service,
        channel: &IOBluetoothRFCOMMChannel,
        tcp_port: u16,
        rt: &tokio::runtime::Handle,
    ) -> std::io::Result<()> {
        let near = match service {
            Service::Link => TcpStream::connect(("127.0.0.1", tcp_port))?,
            Service::Pointer => {
                let local = TcpListener::bind("127.0.0.1:0")?;
                let near = TcpStream::connect(local.local_addr()?)?;
                let (far, _) = local.accept()?;
                far.set_nodelay(true)?;
                far.set_nonblocking(true)?;
                let _guard = rt.enter();
                let stream = tokio::net::TcpStream::from_std(far)?;
                rt.spawn(vyoma_core::pointer::read_frames(stream));
                near
            }
        };
        near.set_nodelay(true)?;
        let back = near.try_clone()?;
        let pipe = Pipe::alloc().set_ivars(PipeIvars {
            tcp: RefCell::new(Some(near)),
        });
        // SAFETY: NSObject's init on our freshly allocated object.
        let pipe: Retained<Pipe> = unsafe { msg_send![super(pipe), init] };
        // SAFETY: the channel IOBluetooth opened for us; our pipe answers its delegate methods.
        let set = unsafe { channel.setDelegate(Some(&pipe)) };
        if set != 0 {
            return Err(std::io::Error::other(format!(
                "the channel took no delegate ({set:#x})"
            )));
        }
        let held = channel.retain();
        let id = key(channel);
        OPEN.with_borrow_mut(|open| open.insert(id, (held, pipe)));
        // SAFETY: this thread's run loop, alive while the services are.
        let run_loop = unsafe { CFRunLoopGetCurrent() } as usize;
        let _ = std::thread::Builder::new()
            .name("vyoma-bt-out".into())
            .spawn(move || to_phone(back, id, run_loop));
        info!(
            "Bluetooth: a phone opened the {}",
            if service == Service::Link {
                "link"
            } else {
                "touchpad"
            }
        );
        Ok(())
    }

    /// What comes back over TCP, written to the phone on the radio's thread, one piece at a time
    /// (each waits for the one before, so nothing piles up behind a slow radio).
    fn to_phone(mut tcp: TcpStream, id: usize, run_loop: usize) {
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<bool>(1);
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match tcp.read(&mut buf) {
                Ok(n) if n > 0 => n,
                _ => break,
            };
            let bytes = buf[..n].to_vec();
            let done = done_tx.clone();
            let block = RcBlock::new(move || {
                let ok = OPEN.with_borrow(|open| {
                    open.get(&id)
                        .is_some_and(|(channel, _)| write(channel, &bytes))
                });
                let _ = done.send(ok);
            });
            // SAFETY: the radio thread's run loop (alive for as long as the services); the block is
            // copied by CoreFoundation and run there.
            unsafe {
                CFRunLoopPerformBlock(run_loop as *const c_void, kCFRunLoopDefaultMode, &block);
                CFRunLoopWakeUp(run_loop as *const c_void);
            }
            if !matches!(done_rx.recv_timeout(Duration::from_secs(30)), Ok(true)) {
                break;
            }
        }
        // The agent's side ended: the phone's channel closes too.
        let block = RcBlock::new(move || {
            if let Some((channel, _)) = OPEN.with_borrow_mut(|open| open.remove(&id)) {
                // SAFETY: our open channel, on its run loop.
                unsafe { channel.closeChannel() };
            }
        });
        // SAFETY: as above.
        unsafe {
            CFRunLoopPerformBlock(run_loop as *const c_void, kCFRunLoopDefaultMode, &block);
            CFRunLoopWakeUp(run_loop as *const c_void);
        }
    }

    /// Writes [bytes] in pieces of the channel's MTU; false if the radio refused.
    fn write(channel: &IOBluetoothRFCOMMChannel, bytes: &[u8]) -> bool {
        // SAFETY: our open channel, on its run loop; each piece's bytes outlive the synchronous write.
        unsafe {
            let mtu = channel.getMTU().max(1) as usize;
            bytes.chunks(mtu).all(|piece| {
                channel.writeSync_length(piece.as_ptr().cast_mut().cast(), piece.len() as u16) == 0
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_read_as_the_phone_keeps_them() {
        assert_eq!(
            format_address("a4-83-e7-12-0b-ff").as_deref(),
            Some("A4:83:E7:12:0B:FF")
        );
        assert_eq!(format_address("00-00-00-00-00-00"), None);
        assert_eq!(format_address("not an address"), None);
    }

    #[test]
    fn uuids_go_most_significant_first() {
        let b = uuid_bytes(POINTER);
        assert_eq!(b[0], 0x6f);
        assert_eq!(b[15], 0x04);
        assert_eq!(
            format!("{POINTER:032x}"),
            vyoma_core::pointer::SERVICE.replace('-', "")
        );
    }
}
