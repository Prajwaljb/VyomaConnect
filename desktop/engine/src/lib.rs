//! Vyoma's PC side, once for every platform. The engine knows what each phone request means, who may
//! make it (permits), what to remember (activity), which tasks it starts and when they end, and what
//! to tell the phone this PC offers. How a thing is done on this computer (take a screenshot, move the
//! pointer, read the wallpaper) is a [Platform]'s: one per OS, each choosing among the ways its
//! desktops allow.

pub mod gamepad;
pub mod calls;
pub mod commands;
pub mod dispatch;
pub mod media;
pub mod mousepad;
pub mod notifications;
#[cfg(feature = "phone-files")]
pub mod phonefiles;
pub mod platform;
pub mod session;
pub mod stock;
pub mod texting;
pub mod ways;
pub mod window;

pub use platform::{Cx, Feature, GamepadInput, Handoff, Platform, Sampler, Started};
