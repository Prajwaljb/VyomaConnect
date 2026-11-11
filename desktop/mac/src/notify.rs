//! Notifications under Vyoma's own name and icon when this runs from Vyoma.app: macOS's
//! UserNotifications, which needs an app bundle (a bare binary asking would crash). Outside one,
//! or when macOS won't take this app's notifications at all, [post] says so and they go by
//! osascript instead. When the person turned Vyoma's notifications off, nothing is shown: that
//! choice isn't routed around.
//! Built on CI only; not yet seen on a real Mac.

use std::sync::Once;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use block2::RcBlock;
use objc2::rc::autoreleasepool;
use objc2::runtime::Bool;
use objc2_foundation::{NSArray, NSBundle, NSError, NSString};
use objc2_user_notifications::{UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter};

const ASKING: u8 = 0;
const ALLOWED: u8 = 1;
const TURNED_OFF: u8 = 2;
const UNAVAILABLE: u8 = 3;

static STATE: AtomicU8 = AtomicU8::new(ASKING);
static ASK: Once = Once::new();
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Running from inside Vyoma.app (the daemon sits in its Contents/MacOS beside the window).
fn bundled() -> bool {
    autoreleasepool(|_| {
        let bundle = NSBundle::mainBundle();
        bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
    })
}

/// Asks macOS once whether Vyoma may notify (the first time, it asks the person). Early, at
/// start, so the answer is in before the first notification.
pub fn start() {
    ASK.call_once(|| {
        if !bundled() {
            STATE.store(UNAVAILABLE, Ordering::SeqCst);
            return;
        }
        let done = RcBlock::new(|granted: Bool, error: *mut NSError| {
            let state = if granted.as_bool() {
                ALLOWED
            } else if error.is_null() {
                TURNED_OFF
            } else {
                // SAFETY: non-null, and macOS keeps it alive for the handler's call.
                let why = unsafe { &*error }.localizedDescription().to_string();
                tracing::warn!("macOS won't show Vyoma's notifications ({why}); using osascript");
                UNAVAILABLE
            };
            STATE.store(state, Ordering::SeqCst);
        });
        autoreleasepool(|_| {
            UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &done);
        });
    });
}

/// Shows it as Vyoma's; false if it should go another way.
pub fn post(title: &str, body: &str) -> bool {
    post_as(None, title, "", body)
}

/// [post] under [id], which a later one replaces and [withdraw] takes down; [subtitle] under the
/// title (a phone notification's app).
pub fn post_as(id: Option<&str>, title: &str, subtitle: &str, body: &str) -> bool {
    start();
    match STATE.load(Ordering::SeqCst) {
        UNAVAILABLE => false,
        TURNED_OFF => true,
        _ => {
            autoreleasepool(|_| {
                let content = UNMutableNotificationContent::new();
                content.setTitle(&NSString::from_str(title));
                content.setBody(&NSString::from_str(body));
                if !subtitle.is_empty() {
                    content.setSubtitle(&NSString::from_str(subtitle));
                }
                let id = NSString::from_str(&id.map(str::to_owned).unwrap_or_else(|| format!("vyoma-{}", NEXT.fetch_add(1, Ordering::Relaxed))));
                let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
                let done = RcBlock::new(|error: *mut NSError| {
                    if !error.is_null() {
                        // SAFETY: as above.
                        tracing::warn!("a notification didn't show: {}", unsafe { &*error }.localizedDescription());
                    }
                });
                UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&done));
            });
            true
        }
    }
}

/// Takes down what [post_as] showed under [id] (the phone dismissed it).
pub fn withdraw(id: &str) {
    if STATE.load(Ordering::SeqCst) != ALLOWED {
        return;
    }
    autoreleasepool(|_| {
        let id = NSString::from_str(id);
        UNUserNotificationCenter::currentNotificationCenter().removeDeliveredNotificationsWithIdentifiers(&NSArray::from_slice(&[&*id]));
    });
}
