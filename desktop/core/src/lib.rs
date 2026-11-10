//! The part of Vyoma's desktop side that every desktop shares: who this PC is, the KDE Connect link
//! to a phone and payloads over it. Pure protocol: nothing here calls a desktop's own APIs, so it
//! builds and tests the same on any Linux desktop and on macOS (see docs/PLAN.md section 10).

pub mod cert;
pub mod device;
pub mod identity;
pub mod invite;
pub mod lan;
pub mod link;
pub mod pairing;
pub mod payload;
pub mod trust;
