//! Wire format.
//!
//! Every blob stored on the relay is:
//!
//! ```text
//! nonce(24) ‖ XChaCha20-Poly1305(room_key, pad(bincode(Inner)), aad="ghostline/v1/outer")
//! ```
//!
//! The outer layer hides *everything* (type, sender, size class only) from the relay.
//! Inside, each [`Inner`] variant is signed with the sender's per-room Ed25519 key,
//! and chat content is additionally encrypted with the sender's ratcheting chain key,
//! so knowing the room password alone does **not** let anyone read messages.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use bincode::Options;

use crate::crypto;
use crate::Error;

/// Ed25519 public key of a room member (per room, never reused across rooms).
pub type MemberId = [u8; 32];
/// Random message identifier.
pub type MsgId = [u8; 16];

const OUTER_AAD: &[u8] = b"ghostline/v1/outer";
const SIG_DOMAIN: &[u8] = b"ghostline/v1/sig/";

/// Signature domain for [`HelloBody`].
pub const KIND_HELLO: u8 = 1;
/// Signature domain for [`KeyDistBody`].
pub const KIND_KEYDIST: u8 = 2;
/// Signature domain for [`MsgBody`].
pub const KIND_MSG: u8 = 3;

/// Top-level message, as found inside the outer encryption layer.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Inner {
    /// "I am here": announces a member's keys and nickname.
    Hello(Signed<HelloBody>),
    /// A sender chain key, encrypted to exactly one member.
    KeyDist(Signed<KeyDistBody>),
    /// Chat content encrypted with the sender's chain.
    Msg(Signed<MsgBody>),
}

/// A body plus an Ed25519 signature over `domain ‖ kind ‖ bincode(body)`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Signed<T> {
    /// The signed payload.
    pub body: T,
    /// 64-byte Ed25519 signature.
    pub sig: Vec<u8>,
}

/// Member announcement.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HelloBody {
    /// Signing key (= member identity in this room).
    pub member: MemberId,
    /// X25519 public key used for key distribution.
    pub dh: [u8; 32],
    /// Nickname (validated by receivers).
    pub nick: String,
}

/// Chain key handed to one member.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct KeyDistBody {
    /// Sender.
    pub from: MemberId,
    /// Recipient.
    pub to: MemberId,
    /// Sender's key epoch (increments on every re-key).
    pub epoch: u32,
    /// AEAD nonce.
    pub nonce: [u8; 24],
    /// Encrypted `chain_key(32) ‖ next_index(u32 BE)`.
    pub ct: Vec<u8>,
}

/// Encrypted chat payload.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MsgBody {
    /// Sender.
    pub from: MemberId,
    /// Sender's key epoch.
    pub epoch: u32,
    /// Position in the sender's chain.
    pub index: u32,
    /// AEAD nonce.
    pub nonce: [u8; 24],
    /// Encrypted, bincode-encoded [`Content`].
    pub ct: Vec<u8>,
}

/// What a [`MsgBody`] decrypts to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// A chat message.
    Text {
        /// Message id (for receipts).
        id: MsgId,
        /// Message text.
        text: String,
        /// Sender's unix time in seconds.
        ts: u64,
    },
    /// Delivery / read receipts for one or more messages.
    Receipt {
        /// Delivered or read.
        kind: ReceiptKind,
        /// Which messages.
        ids: Vec<MsgId>,
    },
    /// Sender leaves the room.
    Leave,
    /// Sender burned the room.
    Burn,
}

/// Receipt type.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptKind {
    /// Arrived on the recipient's device (✓✓).
    Delivered,
    /// Seen by the recipient (blue ✓✓).
    Read,
}

fn codec() -> impl Options {
    bincode::options().with_limit(70_000)
}

/// Serialize with the protocol's bincode settings.
pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    codec()
        .serialize(v)
        .expect("in-memory serialization cannot fail")
}

/// Deserialize with the protocol's bincode settings (size-limited, no trailing bytes).
pub fn decode<T: DeserializeOwned>(b: &[u8]) -> Result<T, Error> {
    codec().deserialize(b).map_err(|_| Error::Malformed)
}

fn sig_input(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(SIG_DOMAIN.len() + 1 + body.len());
    m.extend_from_slice(SIG_DOMAIN);
    m.push(kind);
    m.extend_from_slice(body);
    m
}

impl<T: Serialize> Signed<T> {
    /// Sign `body` with `key` under the given kind.
    pub fn new(key: &SigningKey, kind: u8, body: T) -> Self {
        let sig = key.sign(&sig_input(kind, &encode(&body)));
        Signed {
            body,
            sig: sig.to_bytes().to_vec(),
        }
    }

    /// Verify against the claimed signer's public key.
    pub fn verify(&self, kind: u8, signer: &MemberId) -> Result<(), Error> {
        let vk = VerifyingKey::from_bytes(signer).map_err(|_| Error::BadSignature)?;
        let sig: [u8; 64] = self
            .sig
            .as_slice()
            .try_into()
            .map_err(|_| Error::BadSignature)?;
        vk.verify(
            &sig_input(kind, &encode(&self.body)),
            &Signature::from_bytes(&sig),
        )
        .map_err(|_| Error::BadSignature)
    }
}

/// Wrap an [`Inner`] for the relay.
pub fn seal_outer(room_key: &[u8; 32], inner: &Inner) -> Result<Vec<u8>, Error> {
    let padded = crypto::pad(&encode(inner))?;
    let (nonce, ct) = crypto::seal(room_key, OUTER_AAD, &padded);
    let mut out = Vec::with_capacity(24 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Unwrap a blob fetched from the relay.
pub fn open_outer(room_key: &[u8; 32], blob: &[u8]) -> Result<Inner, Error> {
    if blob.len() < 24 + 16 {
        return Err(Error::Malformed);
    }
    let nonce: [u8; 24] = blob[..24].try_into().expect("24 bytes");
    let padded = crypto::open(room_key, &nonce, OUTER_AAD, &blob[24..])?;
    decode(crypto::unpad(&padded)?)
}
