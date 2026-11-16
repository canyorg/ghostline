//! # ghostline-core
//!
//! Protocol core of GhostLine — Signal Protocol + Tor onion transport, zero metadata.
//!
//! Milestone M1 (this crate, today):
//! - [`room_id`]  — variable-length numeric room IDs with a Damm check digit
//! - [`password`] — CSPRNG password generator ("Şifre öner") and user-password policy
//! - [`nickname`] — nickname validation (anti-spoofing: no bidi/zero-width chars)
//! - [`kdf`]      — Argon2id + HKDF derivation of room key, mailbox id and delete token
//!
//! Milestone M2 (v0):
//! - [`crypto`]  — XChaCha20-Poly1305, HKDF, size padding
//! - [`proto`]   — wire format (protocol v2)
//! - [`session`] — sans-IO room state machine on **libsignal**: PQXDH + Double Ratchet
//!   pairwise sessions, Sender Keys for groups, receipts, re-keying, safety numbers, burn
//!
//! See `docs/ARCHITECTURE.md`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod crypto;
pub mod error;
pub mod kdf;
pub mod nickname;
pub mod password;
pub mod proto;
pub mod room_id;
pub mod session;

pub use error::Error;
pub use kdf::{KdfParams, RoomSecrets};
pub use nickname::Nickname;
pub use room_id::RoomId;
pub use session::{Event, MemberInfo, Session, Status};

use zeroize::Zeroizing;

/// Everything a user needs to share (out of band) so someone else can join a room.
///
/// Both values are fixed for the lifetime of the room; they never rotate.
/// The *password* is the actual secret — the ID alone grants nothing.
pub struct RoomCredentials {
    /// Public-ish room identifier (16–23 digits incl. check digit).
    pub id: RoomId,
    /// Room password. Wiped from memory on drop.
    pub password: Zeroizing<String>,
}

impl RoomCredentials {
    /// Create credentials for a brand-new room.
    ///
    /// If `password` is `None`, a strong random password is generated
    /// (same generator as the "Şifre öner" button).
    pub fn new_room(password: Option<&str>) -> Result<Self, Error> {
        let password = match password {
            Some(p) => {
                password::validate_user_password(p)?;
                Zeroizing::new(p.to_owned())
            }
            None => password::generate(password::DEFAULT_LENGTH),
        };
        Ok(Self {
            id: RoomId::generate(),
            password,
        })
    }

    /// Derive the room secrets (slow on purpose: Argon2id).
    pub fn derive(&self, params: &KdfParams) -> Result<RoomSecrets, Error> {
        kdf::derive_room_secrets(&self.id, &self.password, params)
    }
}

impl core::fmt::Debug for RoomCredentials {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RoomCredentials")
            .field("id", &self.id)
            .field("password", &"<redacted>")
            .finish()
    }
}
