//! One phone's session is the engine's (`vyoma_engine::session`); Linux's side of it is
//! [crate::platform::Linux].

pub use vyoma_engine::session::{Host, Phone, incoming_all, outgoing_all, phone_name, serve};
