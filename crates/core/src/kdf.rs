//! Room key derivation.
//!
//! ```text
//! salt        = SHA-256("ghostline/v1/room-salt" ‖ room_id_digits)[..16]
//! master      = Argon2id(password, salt, m=128 MiB, t=3, p=4)        // 32 bytes
//! room_key    = HKDF-SHA256(master, info="ghostline/v1/room-key")     // bootstrap AEAD key
//! mailbox_id  = HKDF-SHA256(master, info="ghostline/v1/mailbox-id")   // address on the relay
//! delete_tok  = HKDF-SHA256(master, info="ghostline/v1/delete-token") // "Odayı sil" capability
//! delete_hash = SHA-256(delete_tok)                                   // registered on relay
//! ```
//!
//! Design notes:
//! - The relay only ever sees `mailbox_id` and `delete_hash`. Both depend on the
//!   **password**, so knowing the room ID alone does not even let you find the
//!   room's mailbox, let alone read it.
//! - `room_key` is only the *bootstrap* key: it encrypts the join handshake.
//!   Actual messages run over Signal Protocol sessions (Double Ratchet / Sender
//!   Keys), giving forward secrecy — a leaked room password does **not** expose
//!   past messages.
//! - Any member who knows the password can delete the room. This is intentional
//!   ("burn the room" must work even if the creator is gone).

use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Error, RoomId};

const SALT_DOMAIN: &[u8] = b"ghostline/v1/room-salt";
const INFO_ROOM_KEY: &[u8] = b"ghostline/v1/room-key";
const INFO_MAILBOX: &[u8] = b"ghostline/v1/mailbox-id";
const INFO_DELETE: &[u8] = b"ghostline/v1/delete-token";

/// Argon2id cost parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_cost_kib: u32,
    /// Iterations.
    pub t_cost: u32,
    /// Lanes.
    pub p_cost: u32,
}

impl KdfParams {
    /// Production parameters for desktop (≈0.5–1 s on a modern laptop).
    pub const DESKTOP: KdfParams = KdfParams {
        m_cost_kib: 128 * 1024,
        t_cost: 3,
        p_cost: 4,
    };

    /// Tiny parameters. **Only** for tests — never ship these.
    #[doc(hidden)]
    pub const INSECURE_TEST_ONLY: KdfParams = KdfParams {
        m_cost_kib: 64,
        t_cost: 1,
        p_cost: 1,
    };
}

impl Default for KdfParams {
    fn default() -> Self {
        Self::DESKTOP
    }
}

/// Secrets derived for one room. Wiped from memory when dropped.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct RoomSecrets {
    /// Bootstrap AEAD key (join handshake only).
    pub room_key: [u8; 32],
    /// Mailbox address on the relay.
    pub mailbox_id: [u8; 32],
    /// Secret proving the right to delete the room.
    pub delete_token: [u8; 32],
}

impl RoomSecrets {
    /// SHA-256 of the delete token — the only delete-related value the relay stores.
    pub fn delete_token_hash(&self) -> [u8; 32] {
        Sha256::digest(self.delete_token).into()
    }
}

impl core::fmt::Debug for RoomSecrets {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RoomSecrets(<redacted>)")
    }
}

fn room_salt(id: &RoomId) -> [u8; 16] {
    let mut h = Sha256::new();
    h.update(SALT_DOMAIN);
    h.update(id.as_str().as_bytes());
    let digest = h.finalize();
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&digest[..16]);
    salt
}

/// Derive all room secrets from (room ID, password).
pub fn derive_room_secrets(
    id: &RoomId,
    password: &str,
    params: &KdfParams,
) -> Result<RoomSecrets, Error> {
    let argon_params = Params::new(params.m_cost_kib, params.t_cost, params.p_cost, Some(32))
        .map_err(|_| Error::Kdf)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

    let mut master = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password.as_bytes(), &room_salt(id), master.as_mut())
        .map_err(|_| Error::Kdf)?;

    let hk = Hkdf::<Sha256>::new(None, master.as_ref());
    let mut s = RoomSecrets {
        room_key: [0; 32],
        mailbox_id: [0; 32],
        delete_token: [0; 32],
    };
    hk.expand(INFO_ROOM_KEY, &mut s.room_key)
        .map_err(|_| Error::Kdf)?;
    hk.expand(INFO_MAILBOX, &mut s.mailbox_id)
        .map_err(|_| Error::Kdf)?;
    hk.expand(INFO_DELETE, &mut s.delete_token)
        .map_err(|_| Error::Kdf)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: KdfParams = KdfParams::INSECURE_TEST_ONLY;

    #[test]
    fn deterministic() {
        let id = RoomId::generate();
        let a = derive_room_secrets(&id, "hunter2hunter2", &P).unwrap();
        let b = derive_room_secrets(&id, "hunter2hunter2", &P).unwrap();
        assert_eq!(a.room_key, b.room_key);
        assert_eq!(a.mailbox_id, b.mailbox_id);
        assert_eq!(a.delete_token, b.delete_token);
    }

    #[test]
    fn outputs_are_independent() {
        let id = RoomId::generate();
        let s = derive_room_secrets(&id, "hunter2hunter2", &P).unwrap();
        assert_ne!(s.room_key, s.mailbox_id);
        assert_ne!(s.room_key, s.delete_token);
        assert_ne!(s.mailbox_id, s.delete_token);
        assert_ne!(s.delete_token_hash(), s.delete_token);
    }

    #[test]
    fn wrong_password_gives_different_mailbox() {
        let id = RoomId::generate();
        let a = derive_room_secrets(&id, "hunter2hunter2", &P).unwrap();
        let b = derive_room_secrets(&id, "hunter2hunter3", &P).unwrap();
        assert_ne!(a.mailbox_id, b.mailbox_id);
        assert_ne!(a.room_key, b.room_key);
    }

    #[test]
    fn same_password_different_room() {
        let a = derive_room_secrets(&RoomId::generate(), "hunter2hunter2", &P).unwrap();
        let b = derive_room_secrets(&RoomId::generate(), "hunter2hunter2", &P).unwrap();
        assert_ne!(a.mailbox_id, b.mailbox_id);
    }

    #[test]
    fn grouped_and_plain_id_derive_the_same() {
        let id = RoomId::generate();
        let id2 = RoomId::parse(&id.display_grouped()).unwrap();
        let a = derive_room_secrets(&id, "hunter2hunter2", &P).unwrap();
        let b = derive_room_secrets(&id2, "hunter2hunter2", &P).unwrap();
        assert_eq!(a.mailbox_id, b.mailbox_id);
    }

    #[test]
    fn debug_is_redacted() {
        let s = derive_room_secrets(&RoomId::generate(), "hunter2hunter2", &P).unwrap();
        assert_eq!(format!("{s:?}"), "RoomSecrets(<redacted>)");
    }
}
