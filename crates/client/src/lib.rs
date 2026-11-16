//! # ghostline-client
//!
//! Everything a GhostLine front-end needs besides drawing pixels:
//!
//! - [`tor`]  — embedded Tor client (Arti)
//! - [`net`]  — talking to a relay over embedded Tor, a SOCKS5h proxy, or loopback
//! - [`setup`] — "create room" / "join room" (key derivation + relay checks)
//! - [`room`] — a running room: background network tasks + the core [`Session`]
//!
//! Both the terminal client and the desktop app are thin UIs over this crate.
//!
//! [`Session`]: ghostline_core::Session

pub mod net;
pub mod room;
pub mod setup;
pub mod tor;

pub use net::{Relay, Route};
pub use room::{Room, RoomEvent};
pub use setup::{NetConfig, Prepared, TorMode};
pub use tor::EmbeddedTor;
