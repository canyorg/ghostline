//! Wire format, protocol version 2 (libsignal).
//!
//! Every blob stored on the relay is:
//!
//! ```text
//! nonce(24) ‖ XChaCha20-Poly1305(room_key, pad("G2" ‖ bincode(Inner)), aad="ghostline/v1/outer")
//! ```
//!
//! The outer layer (keyed by the room password) hides everything from the relay.
//! Inside it:
//!
//! - [`Hello`]: a member's public Signal identity + PQXDH pre-key bundle, self-signed.
//! - [`Direct`]: a libsignal *pairwise* message (PQXDH + Double Ratchet) carrying a
//!   [`Control`] — used to hand out Sender Keys.
//! - [`Group`]: a libsignal *SenderKeyMessage* carrying [`Content`] (chat, receipts, burn).
//! - [`Leave`]: a signed "I'm leaving" notice (readable by every member, even new ones).

use bincode::Options;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::crypto;
use crate::Error;

/// Protocol version carried in every blob.
pub const PROTOCOL_VERSION: u8 = 2;
const MAGIC: [u8; 2] = [b'G', PROTOCOL_VERSION];
const OUTER_AAD: &[u8] = b"ghostline/v1/outer";

/// Stable per-room member id: SHA-256 of the member's Signal identity key.
pub type MemberId = [u8; 32];
/// Random message identifier.
pub type MsgId = [u8; 16];

/// Top-level message inside the outer encryption layer.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Inner {
    /// Member announcement (public key material).
    Hello(Hello),
    /// Pairwise Signal message.
    Direct(Direct),
    /// Group (Sender Key) message.
    Group(Group),
    /// Member leaves.
    Leave(Leave),
}

/// Public part of a member's Signal identity and PQXDH bundle.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HelloBody {
    /// Serialized libsignal `IdentityKey`.
    pub identity: Vec<u8>,
    /// Signal registration id.
    pub registration_id: u32,
    /// Signed EC pre-key id.
    pub signed_prekey_id: u32,
    /// Signed EC pre-key (serialized `PublicKey`).
    pub signed_prekey: Vec<u8>,
    /// Identity signature over the signed pre-key.
    pub signed_prekey_sig: Vec<u8>,
    /// Kyber/ML-KEM pre-key id.
    pub kyber_prekey_id: u32,
    /// Kyber/ML-KEM pre-key (serialized `kem::PublicKey`).
    pub kyber_prekey: Vec<u8>,
    /// Identity signature over the Kyber pre-key.
    pub kyber_prekey_sig: Vec<u8>,
    /// Nickname (validated by receivers).
    pub nick: String,
}

/// [`HelloBody`] signed with the member's identity key (XEdDSA).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Hello {
    /// The announcement.
    pub body: HelloBody,
    /// Signature over `"ghostline/v2/hello" ‖ bincode(body)`.
    pub sig: Vec<u8>,
}

/// Pairwise libsignal message.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Direct {
    /// Sender.
    pub from: MemberId,
    /// Recipient.
    pub to: MemberId,
    /// `true` = `PreKeySignalMessage` (session setup), `false` = `SignalMessage`.
    pub prekey: bool,
    /// Serialized libsignal message.
    pub body: Vec<u8>,
}

/// Group message.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Group {
    /// Sender.
    pub from: MemberId,
    /// Serialized libsignal `SenderKeyMessage`.
    pub body: Vec<u8>,
}

/// Signed leave notice.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Leave {
    /// Leaving member.
    pub member: MemberId,
    /// Signature over `"ghostline/v2/leave" ‖ member`.
    pub sig: Vec<u8>,
}

/// Plaintext of a [`Direct`] message.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Control {
    /// Serialized libsignal `SenderKeyDistributionMessage`.
    SenderKey(Vec<u8>),
}

/// Plaintext of a [`Group`] message.
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
    /// Delivery / read receipts.
    Receipt {
        /// Delivered or read.
        kind: ReceiptKind,
        /// Which messages.
        ids: Vec<MsgId>,
    },
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

/// Signature domain for [`HelloBody`].
pub const HELLO_DOMAIN: &[u8] = b"ghostline/v2/hello";
/// Signature domain for [`Leave`].
pub const LEAVE_DOMAIN: &[u8] = b"ghostline/v2/leave";

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

/// Wrap an [`Inner`] for the relay.
pub fn seal_outer(room_key: &[u8; 32], inner: &Inner) -> Result<Vec<u8>, Error> {
    let mut plain = MAGIC.to_vec();
    plain.extend_from_slice(&encode(inner));
    let padded = crypto::pad(&plain)?;
    let (nonce, ct) = crypto::seal(room_key, OUTER_AAD, &padded);
    let mut out = Vec::with_capacity(24 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Unwrap a blob fetched from the relay.
///
/// Returns [`Error::OldVersion`] for blobs from an older GhostLine (right room key,
/// unknown format), so the UI can tell the user to update.
pub fn open_outer(room_key: &[u8; 32], blob: &[u8]) -> Result<Inner, Error> {
    if blob.len() < 24 + 16 {
        return Err(Error::Malformed);
    }
    let nonce: [u8; 24] = blob[..24].try_into().expect("24 bytes");
    let padded = crypto::open(room_key, &nonce, OUTER_AAD, &blob[24..])?;
    let plain = crypto::unpad(&padded)?;
    match plain.strip_prefix(&MAGIC) {
        Some(rest) => decode(rest),
        None => Err(Error::OldVersion),
    }
}
