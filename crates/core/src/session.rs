//! Room session state machine (sans-IO), protocol v2 — built on **libsignal**.
//!
//! The network layer feeds every blob from the relay mailbox, in order, into
//! [`Session::handle_blob`]; the session returns UI [`Event`]s plus blobs that must be
//! posted back.
//!
//! ## Cryptography
//!
//! Signal's own library (`libsignal-protocol`) does all message cryptography:
//!
//! - **Identity:** each member creates a fresh Signal identity key for this room only
//!   (rooms cannot be linked to each other).
//! - **Pairwise sessions:** every pair of members runs **PQXDH** (X25519 + ML-KEM-1024,
//!   post-quantum) followed by the **Double Ratchet** — forward secrecy *and*
//!   post-compromise security, exactly as in Signal.
//! - **Group messages:** Signal's **Sender Keys** (as used by Signal and WhatsApp groups).
//!   Each member's sender key is handed out over the pairwise Double Ratchet sessions.
//! - **Re-keying:** a member's sender key is replaced when someone leaves and after every
//!   [`REKEY_EVERY`] messages, so a stolen sender key stops working.
//! - **Verification:** [`Session::member_details`] returns Signal **safety numbers**.
//!
//! Everything lives in memory (libsignal's in-memory stores) and is dropped with the session.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::executor::block_on;
use libsignal_protocol::{
    create_sender_key_distribution_message, group_decrypt, group_encrypt, kem, message_decrypt,
    message_encrypt, process_prekey_bundle, process_sender_key_distribution_message,
    CiphertextMessage, DeviceId, Fingerprint, GenericSignedPreKey, IdentityKey, IdentityKeyPair,
    InMemSignalProtocolStore, KeyPair, KyberPreKeyRecord, PreKeyBundle, PreKeySignalMessage,
    ProtocolAddress, PublicKey, SenderKeyDistributionMessage, SignalMessage, SignedPreKeyRecord,
    Timestamp,
};
use rand09::RngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto;
use crate::proto::*;
use crate::{Error, Nickname, RoomSecrets};

/// Maximum characters per chat message.
pub const MAX_TEXT_CHARS: usize = 4000;
/// A member rotates its sender key after this many messages.
pub const REKEY_EVERY: u32 = 100;
const SIGNED_PREKEY_ID: u32 = 1;
const KYBER_PREKEY_ID: u32 = 1;

/// Delivery state of a message *we* sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Not yet acknowledged by the relay.
    Pending,
    /// Stored on the relay (✓).
    Sent,
    /// Decrypted on every recipient's device (✓✓).
    Delivered,
    /// Seen by every recipient (blue ✓✓).
    Read,
}

/// Things the UI should show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A member appeared.
    Joined {
        /// Display name `nick#fingerprint`.
        who: String,
    },
    /// A member left.
    Left {
        /// Display name.
        who: String,
    },
    /// Incoming chat message.
    Message {
        /// Message id.
        id: MsgId,
        /// Display name of the sender.
        from: String,
        /// Text.
        text: String,
        /// Sender's unix time (seconds).
        ts: u64,
    },
    /// Status change of one of our messages.
    Status {
        /// Message id.
        id: MsgId,
        /// New status.
        status: Status,
    },
    /// Someone burned the room.
    Burned {
        /// Display name.
        by: String,
    },
    /// Someone in the room runs an older, incompatible GhostLine.
    OutdatedPeer,
}

/// A member as shown in the members list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberInfo {
    /// `nick#fingerprint`.
    pub display: String,
    /// Signal safety number (60 digits in 12 groups of 5). Identical on both sides.
    pub safety_number: String,
}

struct Member {
    nick: String,
    identity: IdentityKey,
    addr: ProtocolAddress,
    active: bool,
    safety_number: String,
}

struct Outgoing {
    recipients: HashSet<MemberId>,
    delivered: HashSet<MemberId>,
    read: HashSet<MemberId>,
    status: Status,
}

impl Outgoing {
    fn recompute(&mut self) -> Option<Status> {
        let new = if self.recipients.is_empty() || self.status < Status::Sent {
            self.status
        } else if self.recipients.is_subset(&self.read) {
            Status::Read
        } else if self.recipients.is_subset(&self.delivered) {
            Status::Delivered
        } else {
            self.status
        };
        if new > self.status {
            self.status = new;
            Some(new)
        } else {
            None
        }
    }
}

/// Short, human-comparable fingerprint of a member id (6 hex chars).
pub fn fingerprint(m: &MemberId) -> String {
    m[..3].iter().map(|b| format!("{b:02x}")).collect()
}

fn display(nick: &str, m: &MemberId) -> String {
    format!("{nick}#{}", fingerprint(m))
}

fn member_id(identity: &IdentityKey) -> MemberId {
    let mut h = Sha256::new();
    h.update(b"ghostline/v2/member");
    h.update(identity.serialize());
    h.finalize().into()
}

fn address(id: &MemberId) -> ProtocolAddress {
    ProtocolAddress::new(
        id.iter().map(|b| format!("{b:02x}")).collect(),
        DeviceId::try_from(1u32).expect("1 is a valid device id"),
    )
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sig_err<E: std::fmt::Display>(e: E) -> Error {
    Error::Signal(e.to_string())
}

fn signed(domain: &[u8], body: &[u8]) -> Vec<u8> {
    [domain, body].concat()
}

/// One user's view of one room.
pub struct Session {
    room_key: Zeroizing<[u8; 32]>,
    identity: IdentityKeyPair,
    me: MemberId,
    my_addr: ProtocolAddress,
    nick: Nickname,
    store: InMemSignalProtocolStore,
    hello: Hello,
    dist_id: Uuid,
    sent_since_rekey: u32,
    members: HashMap<MemberId, Member>,
    outbox: HashMap<MsgId, Outgoing>,
    pending_delivered: Vec<MsgId>,
    warned_outdated: bool,
}

impl Session {
    /// Fresh per-room Signal identity, pre-keys and sender key.
    pub fn new(nick: Nickname, secrets: &RoomSecrets) -> Result<Self, Error> {
        let mut rng = rand09::rng();
        let identity = IdentityKeyPair::generate(&mut rng);
        let me = member_id(identity.identity_key());
        let my_addr = address(&me);
        let registration_id = rng.next_u32() & 0x3fff;
        let mut store = InMemSignalProtocolStore::new(identity, registration_id)?;

        // Signed EC pre-key.
        let spk = KeyPair::generate(&mut rng);
        let spk_sig = identity
            .private_key()
            .calculate_signature(&spk.public_key.serialize(), &mut rng)
            .map_err(sig_err)?;
        let ts = Timestamp::from_epoch_millis(now_secs() * 1000);
        let spk_record = SignedPreKeyRecord::new(SIGNED_PREKEY_ID.into(), ts, &spk, &spk_sig);
        block_on(libsignal_protocol::SignedPreKeyStore::save_signed_pre_key(
            &mut store,
            SIGNED_PREKEY_ID.into(),
            &spk_record,
        ))?;

        // Post-quantum (ML-KEM-1024 / Kyber) last-resort pre-key.
        let kyber = KyberPreKeyRecord::generate(
            kem::KeyType::Kyber1024,
            KYBER_PREKEY_ID.into(),
            identity.private_key(),
        )?;
        block_on(libsignal_protocol::KyberPreKeyStore::save_kyber_pre_key(
            &mut store,
            KYBER_PREKEY_ID.into(),
            &kyber,
        ))?;

        let body = HelloBody {
            identity: identity.identity_key().serialize().to_vec(),
            registration_id,
            signed_prekey_id: SIGNED_PREKEY_ID,
            signed_prekey: spk.public_key.serialize().to_vec(),
            signed_prekey_sig: spk_sig.to_vec(),
            kyber_prekey_id: KYBER_PREKEY_ID,
            kyber_prekey: kyber.public_key()?.serialize().to_vec(),
            kyber_prekey_sig: kyber.signature()?,
            nick: nick.as_str().to_owned(),
        };
        let sig = identity
            .private_key()
            .calculate_signature(&signed(HELLO_DOMAIN, &encode(&body)), &mut rng)
            .map_err(sig_err)?
            .to_vec();

        let mut s = Session {
            room_key: Zeroizing::new(secrets.room_key),
            identity,
            me,
            my_addr,
            nick,
            store,
            hello: Hello { body, sig },
            dist_id: Uuid::from_bytes(crypto::random16()),
            sent_since_rekey: 0,
            members: HashMap::new(),
            outbox: HashMap::new(),
            pending_delivered: Vec::new(),
            warned_outdated: false,
        };
        // Create our first sender key chain.
        s.current_skdm()?;
        Ok(s)
    }

    /// Our display name (`nick#fingerprint`).
    pub fn me_display(&self) -> String {
        display(self.nick.as_str(), &self.me)
    }

    /// Display names of the other active members.
    pub fn members(&self) -> Vec<String> {
        self.member_details()
            .into_iter()
            .map(|m| m.display)
            .collect()
    }

    /// Other active members with their Signal safety numbers.
    pub fn member_details(&self) -> Vec<MemberInfo> {
        let mut v: Vec<MemberInfo> = self
            .members
            .iter()
            .filter(|(_, m)| m.active)
            .map(|(id, m)| MemberInfo {
                display: display(&m.nick, id),
                safety_number: m.safety_number.clone(),
            })
            .collect();
        v.sort_by(|a, b| a.display.cmp(&b.display));
        v
    }

    /// Blob announcing ourselves. Post this first.
    pub fn hello_blob(&self) -> Result<Vec<u8>, Error> {
        seal_outer(&self.room_key, &Inner::Hello(self.hello.clone()))
    }

    fn current_skdm(&mut self) -> Result<SenderKeyDistributionMessage, Error> {
        let mut rng = rand09::rng();
        Ok(block_on(create_sender_key_distribution_message(
            &self.my_addr,
            self.dist_id,
            &mut self.store,
            &mut rng,
        ))?)
    }

    fn direct(&mut self, to: &MemberId, control: &Control) -> Result<Vec<u8>, Error> {
        let addr = self.members.get(to).ok_or(Error::UnknownKey)?.addr.clone();
        let mut rng = rand09::rng();
        let ct = block_on(message_encrypt(
            &encode(control),
            &addr,
            &self.my_addr,
            &mut self.store.session_store,
            &mut self.store.identity_store,
            SystemTime::now(),
            &mut rng,
        ))?;
        let prekey = matches!(ct, CiphertextMessage::PreKeySignalMessage(_));
        seal_outer(
            &self.room_key,
            &Inner::Direct(Direct {
                from: self.me,
                to: *to,
                prekey,
                body: ct.serialize().to_vec(),
            }),
        )
    }

    fn send_sender_key_to(&mut self, to: &MemberId) -> Result<Vec<u8>, Error> {
        let skdm = self.current_skdm()?;
        self.direct(to, &Control::SenderKey(skdm.serialized().to_vec()))
    }

    /// New sender key, handed to every active member over the pairwise sessions.
    fn rekey(&mut self) -> Vec<Vec<u8>> {
        self.dist_id = Uuid::from_bytes(crypto::random16());
        self.sent_since_rekey = 0;
        let targets: Vec<MemberId> = self
            .members
            .iter()
            .filter(|(_, m)| m.active)
            .map(|(id, _)| *id)
            .collect();
        targets
            .iter()
            .filter_map(|id| self.send_sender_key_to(id).ok())
            .collect()
    }

    fn group(&mut self, content: &Content) -> Result<Vec<u8>, Error> {
        let mut rng = rand09::rng();
        let skm = block_on(group_encrypt(
            &mut self.store,
            &self.my_addr,
            self.dist_id,
            &encode(content),
            &mut rng,
        ))?;
        self.sent_since_rekey += 1;
        seal_outer(
            &self.room_key,
            &Inner::Group(Group {
                from: self.me,
                body: skm.serialized().to_vec(),
            }),
        )
    }

    /// Encrypt a chat message. Returns its id and the blobs to post **in order**
    /// (a sender-key rotation may come first).
    pub fn send_text(&mut self, text: &str) -> Result<(MsgId, Vec<Vec<u8>>), Error> {
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(Error::TooLarge);
        }
        let mut blobs = Vec::new();
        if self.sent_since_rekey >= REKEY_EVERY {
            blobs.extend(self.rekey());
        }
        let id = crypto::random16();
        blobs.push(self.group(&Content::Text {
            id,
            text: text.to_owned(),
            ts: now_secs(),
        })?);
        let recipients = self
            .members
            .iter()
            .filter(|(_, m)| m.active)
            .map(|(k, _)| *k)
            .collect();
        self.outbox.insert(
            id,
            Outgoing {
                recipients,
                delivered: HashSet::new(),
                read: HashSet::new(),
                status: Status::Pending,
            },
        );
        Ok((id, blobs))
    }

    /// The relay acknowledged one of our messages (✓).
    pub fn mark_sent(&mut self, id: &MsgId) -> Option<Event> {
        let o = self.outbox.get_mut(id)?;
        if o.status >= Status::Sent {
            return None;
        }
        o.status = Status::Sent;
        let status = o.recompute().unwrap_or(Status::Sent);
        Some(Event::Status { id: *id, status })
    }

    /// Delivery receipt for everything decrypted since the last call.
    pub fn take_delivered_receipt(&mut self) -> Option<Vec<u8>> {
        if self.pending_delivered.is_empty() {
            return None;
        }
        let ids = std::mem::take(&mut self.pending_delivered);
        self.group(&Content::Receipt {
            kind: ReceiptKind::Delivered,
            ids,
        })
        .ok()
    }

    /// Read receipt for messages the user has seen.
    pub fn read_receipt(&mut self, ids: Vec<MsgId>) -> Option<Vec<u8>> {
        if ids.is_empty() {
            return None;
        }
        self.group(&Content::Receipt {
            kind: ReceiptKind::Read,
            ids,
        })
        .ok()
    }

    /// "I'm leaving" — post before closing. Signed, readable by every member.
    pub fn leave_blob(&mut self) -> Result<Vec<u8>, Error> {
        let mut rng = rand09::rng();
        let sig = self
            .identity
            .private_key()
            .calculate_signature(&signed(LEAVE_DOMAIN, &self.me), &mut rng)
            .map_err(sig_err)?
            .to_vec();
        seal_outer(
            &self.room_key,
            &Inner::Leave(Leave {
                member: self.me,
                sig,
            }),
        )
    }

    /// "Room burned" — post, then DELETE the mailbox.
    pub fn burn_blob(&mut self) -> Result<Vec<u8>, Error> {
        self.group(&Content::Burn)
    }

    /// Process one blob from the mailbox. Undecryptable / foreign / invalid blobs
    /// are silently ignored (e.g. history from before we joined).
    pub fn handle_blob(&mut self, blob: &[u8]) -> (Vec<Event>, Vec<Vec<u8>>) {
        let mut events = Vec::new();
        let mut out = Vec::new();
        if let Err(Error::OldVersion) = self.handle_inner(blob, &mut events, &mut out) {
            if !self.warned_outdated {
                self.warned_outdated = true;
                events.push(Event::OutdatedPeer);
            }
        }
        (events, out)
    }

    fn handle_inner(
        &mut self,
        blob: &[u8],
        events: &mut Vec<Event>,
        out: &mut Vec<Vec<u8>>,
    ) -> Result<(), Error> {
        match open_outer(&self.room_key, blob)? {
            Inner::Hello(h) => self.on_hello(h, events, out),
            Inner::Direct(d) => self.on_direct(d),
            Inner::Group(g) => self.on_group(g, events),
            Inner::Leave(l) => self.on_leave(l, events, out),
        }
    }

    fn on_hello(
        &mut self,
        h: Hello,
        events: &mut Vec<Event>,
        out: &mut Vec<Vec<u8>>,
    ) -> Result<(), Error> {
        let b = &h.body;
        let identity = IdentityKey::decode(&b.identity)?;
        let id = member_id(&identity);
        if id == self.me {
            return Ok(());
        }
        if !identity
            .public_key()
            .verify_signature(&signed(HELLO_DOMAIN, &encode(b)), &h.sig)
        {
            return Err(Error::BadSignature);
        }
        let nick = Nickname::new(&b.nick).map_err(|_| Error::Malformed)?;
        if self.members.contains_key(&id) {
            return Ok(()); // replayed hello, or a member who left (never re-admitted)
        }
        let bundle = PreKeyBundle::new(
            b.registration_id,
            DeviceId::try_from(1u32).expect("valid"),
            None,
            b.signed_prekey_id.into(),
            PublicKey::deserialize(&b.signed_prekey).map_err(sig_err)?,
            b.signed_prekey_sig.clone(),
            b.kyber_prekey_id.into(),
            kem::PublicKey::deserialize(&b.kyber_prekey)?,
            b.kyber_prekey_sig.clone(),
            identity,
        )?;
        let addr = address(&id);
        let mut rng = rand09::rng();
        // PQXDH: verifies both pre-key signatures and starts a Double Ratchet session.
        block_on(process_prekey_bundle(
            &addr,
            &self.my_addr,
            &mut self.store.session_store,
            &mut self.store.identity_store,
            &bundle,
            SystemTime::now(),
            &mut rng,
        ))?;
        let safety_number = Fingerprint::new(
            2,
            5200,
            &self.me,
            self.identity.identity_key(),
            &id,
            &identity,
        )
        .and_then(|f| f.display_string())
        .map(|s| {
            s.as_bytes()
                .chunks(5)
                .map(|c| String::from_utf8_lossy(c).into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
        self.members.insert(
            id,
            Member {
                nick: nick.as_str().to_owned(),
                identity,
                addr,
                active: true,
                safety_number,
            },
        );
        events.push(Event::Joined {
            who: display(nick.as_str(), &id),
        });
        out.push(self.send_sender_key_to(&id)?);
        Ok(())
    }

    fn on_direct(&mut self, d: Direct) -> Result<(), Error> {
        if d.to != self.me {
            return Ok(());
        }
        let m = self.members.get(&d.from).ok_or(Error::UnknownKey)?;
        if !m.active {
            return Ok(());
        }
        let addr = m.addr.clone();
        let ct = if d.prekey {
            CiphertextMessage::PreKeySignalMessage(PreKeySignalMessage::try_from(&d.body[..])?)
        } else {
            CiphertextMessage::SignalMessage(SignalMessage::try_from(&d.body[..])?)
        };
        let mut rng = rand09::rng();
        let store = &mut self.store;
        let plain = Zeroizing::new(block_on(message_decrypt(
            &ct,
            &addr,
            &self.my_addr,
            &mut store.session_store,
            &mut store.identity_store,
            &mut store.pre_key_store,
            &store.signed_pre_key_store,
            &mut store.kyber_pre_key_store,
            &mut rng,
        ))?);
        match decode::<Control>(&plain)? {
            Control::SenderKey(bytes) => {
                let skdm = SenderKeyDistributionMessage::try_from(&bytes[..])?;
                block_on(process_sender_key_distribution_message(
                    &addr,
                    &skdm,
                    &mut self.store.sender_key_store,
                ))?;
            }
        }
        Ok(())
    }

    fn on_group(&mut self, g: Group, events: &mut Vec<Event>) -> Result<(), Error> {
        if g.from == self.me {
            return Ok(());
        }
        let m = self.members.get(&g.from).ok_or(Error::UnknownKey)?;
        if !m.active {
            return Ok(());
        }
        let (addr, who) = (m.addr.clone(), display(&m.nick, &g.from));
        let plain = Zeroizing::new(block_on(group_decrypt(
            &g.body,
            &mut self.store.sender_key_store,
            &addr,
        ))?);
        let from = g.from;
        match decode::<Content>(&plain)? {
            Content::Text { id, text, ts } => {
                if text.chars().count() > MAX_TEXT_CHARS {
                    return Err(Error::TooLarge);
                }
                self.pending_delivered.push(id);
                events.push(Event::Message {
                    id,
                    from: who,
                    text,
                    ts,
                });
            }
            Content::Receipt { kind, ids } => {
                for id in ids {
                    if let Some(o) = self.outbox.get_mut(&id) {
                        if !o.recipients.contains(&from) {
                            continue;
                        }
                        o.delivered.insert(from);
                        if kind == ReceiptKind::Read {
                            o.read.insert(from);
                        }
                        if let Some(status) = o.recompute() {
                            events.push(Event::Status { id, status });
                        }
                    }
                }
                self.outbox.retain(|_, o| o.status != Status::Read);
            }
            Content::Burn => events.push(Event::Burned { by: who }),
        }
        Ok(())
    }

    fn on_leave(
        &mut self,
        l: Leave,
        events: &mut Vec<Event>,
        out: &mut Vec<Vec<u8>>,
    ) -> Result<(), Error> {
        if l.member == self.me {
            return Ok(());
        }
        let m = self.members.get_mut(&l.member).ok_or(Error::UnknownKey)?;
        if !m.active {
            return Ok(());
        }
        if !m
            .identity
            .public_key()
            .verify_signature(&signed(LEAVE_DOMAIN, &l.member), &l.sig)
        {
            return Err(Error::BadSignature);
        }
        m.active = false;
        let who = display(&m.nick, &l.member);
        events.push(Event::Left { who });
        for (id, o) in self.outbox.iter_mut() {
            o.recipients.remove(&l.member);
            if let Some(status) = o.recompute() {
                events.push(Event::Status { id: *id, status });
            }
        }
        // The leaver must not be able to read anything we send from now on.
        out.extend(self.rekey());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KdfParams, RoomCredentials};

    /// In-memory relay: a list of blobs; each client has a read cursor.
    struct Sim {
        mailbox: Vec<Vec<u8>>,
        clients: Vec<(Session, usize, Vec<Event>)>,
    }

    impl Sim {
        fn new() -> (Self, RoomSecrets) {
            let creds = RoomCredentials::new_room(None).unwrap();
            let secrets = creds.derive(&KdfParams::INSECURE_TEST_ONLY).unwrap();
            (
                Sim {
                    mailbox: vec![],
                    clients: vec![],
                },
                secrets,
            )
        }
        fn join(&mut self, nick: &str, secrets: &RoomSecrets) -> usize {
            let s = Session::new(Nickname::new(nick).unwrap(), secrets).unwrap();
            self.mailbox.push(s.hello_blob().unwrap());
            self.clients.push((s, 0, vec![]));
            self.pump();
            self.clients.len() - 1
        }
        fn post(&mut self, blob: Vec<u8>) {
            self.mailbox.push(blob);
        }
        fn pump(&mut self) {
            loop {
                let mut progressed = false;
                for i in 0..self.clients.len() {
                    while self.clients[i].1 < self.mailbox.len() {
                        let blob = self.mailbox[self.clients[i].1].clone();
                        self.clients[i].1 += 1;
                        progressed = true;
                        let (ev, out) = self.clients[i].0.handle_blob(&blob);
                        self.clients[i].2.extend(ev);
                        self.mailbox.extend(out);
                    }
                    if let Some(r) = self.clients[i].0.take_delivered_receipt() {
                        self.mailbox.push(r);
                        progressed = true;
                    }
                }
                if !progressed {
                    break;
                }
            }
        }
        fn send(&mut self, who: usize, text: &str) -> MsgId {
            let (id, blobs) = self.clients[who].0.send_text(text).unwrap();
            self.mailbox.extend(blobs);
            self.clients[who].0.mark_sent(&id);
            self.pump();
            id
        }
        fn texts(&self, who: usize) -> Vec<String> {
            self.clients[who]
                .2
                .iter()
                .filter_map(|e| match e {
                    Event::Message { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        }
        fn last_status(&self, who: usize, id: MsgId) -> Option<Status> {
            self.clients[who].2.iter().rev().find_map(|e| match e {
                Event::Status { id: i, status } if *i == id => Some(*status),
                _ => None,
            })
        }
    }

    #[test]
    fn two_members_chat_with_receipts() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        let id = sim.send(a, "hello bob");
        sim.send(b, "hi alice");
        assert_eq!(sim.texts(b), vec!["hello bob"]);
        assert_eq!(sim.texts(a), vec!["hi alice"]);
        assert_eq!(sim.last_status(a, id), Some(Status::Delivered));

        let rr = sim.clients[b].0.read_receipt(vec![id]).unwrap();
        sim.post(rr);
        sim.pump();
        assert_eq!(sim.last_status(a, id), Some(Status::Read));
    }

    #[test]
    fn three_members() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        let c = sim.join("carol", &sec);
        sim.send(c, "carol here");
        sim.send(a, "hi carol");
        assert_eq!(sim.texts(a), vec!["carol here"]);
        assert_eq!(sim.texts(b), vec!["carol here", "hi carol"]);
        assert_eq!(sim.texts(c), vec!["hi carol"]);
    }

    #[test]
    fn newcomer_cannot_read_history() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        sim.send(a, "secret before carol");
        let c = sim.join("carol", &sec);
        sim.send(a, "welcome carol");
        assert_eq!(sim.texts(c), vec!["welcome carol"]);
        assert_eq!(sim.texts(b), vec!["secret before carol", "welcome carol"]);
    }

    #[test]
    fn leaver_cannot_read_after_rekey() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        let c = sim.join("carol", &sec);
        let leave = sim.clients[c].0.leave_blob().unwrap();
        sim.post(leave);
        sim.pump();
        assert!(sim.clients[a]
            .2
            .iter()
            .any(|e| matches!(e, Event::Left { .. })));
        sim.send(a, "after carol left");
        assert_eq!(sim.texts(b), vec!["after carol left"]);
        assert!(sim.texts(c).is_empty());
    }

    #[test]
    fn periodic_rekey_keeps_working() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        let n = REKEY_EVERY + 20;
        for i in 0..n {
            sim.send(a, &format!("m{i}"));
        }
        assert_eq!(sim.texts(b).len(), n as usize);
        assert_eq!(sim.texts(b).last().unwrap(), &format!("m{}", n - 1));
    }

    #[test]
    fn password_holder_cannot_read_or_forge() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        sim.send(a, "only for members");

        // Eve knows the password and reads the whole mailbox without announcing herself.
        let mut eve = Session::new(Nickname::new("eve").unwrap(), &sec).unwrap();
        let mut seen = vec![];
        for blob in sim.mailbox.clone() {
            seen.extend(eve.handle_blob(&blob).0);
        }
        assert!(!seen.iter().any(|e| matches!(e, Event::Message { .. })));

        // Replay of alice's group message: rejected by libsignal (duplicate counter).
        let alice = sim.clients[a].0.me;
        let replay = sim
            .mailbox
            .iter()
            .find(|blob| {
                matches!(open_outer(&sec.room_key, blob), Ok(Inner::Group(g)) if g.from == alice)
            })
            .unwrap()
            .clone();
        let before = sim.texts(b).len();
        sim.post(replay.clone());
        sim.pump();
        assert_eq!(sim.texts(b).len(), before);

        // Tampered group message: signature check fails.
        let Ok(Inner::Group(mut g)) = open_outer(&sec.room_key, &replay) else {
            panic!()
        };
        let n = g.body.len();
        g.body[n - 70] ^= 1;
        sim.post(seal_outer(&sec.room_key, &Inner::Group(g)).unwrap());
        sim.pump();
        assert_eq!(sim.texts(b).len(), before);

        // Eve claims to be alice in a group message: she has no sender key for that address.
        let (_, blobs) = eve.send_text("i am alice").unwrap();
        let Ok(Inner::Group(mut g)) = open_outer(&sec.room_key, blobs.last().unwrap()) else {
            panic!()
        };
        g.from = alice;
        sim.post(seal_outer(&sec.room_key, &Inner::Group(g)).unwrap());
        sim.pump();
        assert_eq!(sim.texts(b).len(), before);

        // Eve forges a Leave for alice: signature fails, alice stays.
        sim.post(
            seal_outer(
                &sec.room_key,
                &Inner::Leave(Leave {
                    member: alice,
                    sig: vec![0u8; 64],
                }),
            )
            .unwrap(),
        );
        sim.pump();
        assert!(!sim.clients[b]
            .2
            .iter()
            .any(|e| matches!(e, Event::Left { .. })));
    }

    #[test]
    fn wrong_password_sees_nothing() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        sim.send(a, "x");
        let (_, other) = Sim::new();
        let mut mallory = Session::new(Nickname::new("mallory").unwrap(), &other).unwrap();
        for blob in &sim.mailbox {
            assert!(mallory.handle_blob(blob).0.is_empty());
        }
    }

    #[test]
    fn burn_is_seen_by_everyone() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        let burn = sim.clients[a].0.burn_blob().unwrap();
        sim.post(burn);
        sim.pump();
        assert!(sim.clients[b]
            .2
            .iter()
            .any(|e| matches!(e, Event::Burned { .. })));
    }

    #[test]
    fn out_of_order_and_never_twice() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        let (_, m1) = sim.clients[a].0.send_text("one").unwrap();
        let (_, m2) = sim.clients[a].0.send_text("two").unwrap();
        let (ev2, _) = sim.clients[b].0.handle_blob(&m2[0]);
        let (ev1, _) = sim.clients[b].0.handle_blob(&m1[0]);
        assert!(matches!(&ev2[0], Event::Message { text, .. } if text == "two"));
        assert!(matches!(&ev1[0], Event::Message { text, .. } if text == "one"));
        assert!(sim.clients[b].0.handle_blob(&m1[0]).0.is_empty());
    }

    #[test]
    fn safety_numbers_match_on_both_sides() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        let sa = &sim.clients[a].0.member_details()[0].safety_number;
        let sb = &sim.clients[b].0.member_details()[0].safety_number;
        assert_eq!(sa, sb);
        assert_eq!(sa.replace(' ', "").len(), 60);
    }

    #[test]
    fn old_version_blob_is_reported_once() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        // A v1-style blob: right room key, no "G2" magic.
        let padded = crypto::pad(&[0u8, 1, 2, 3]).unwrap();
        let (n, ct) = crypto::seal(&sec.room_key, b"ghostline/v1/outer", &padded);
        let blob = [n.to_vec(), ct].concat();
        let (ev, _) = sim.clients[a].0.handle_blob(&blob);
        assert_eq!(ev, vec![Event::OutdatedPeer]);
        assert!(sim.clients[a].0.handle_blob(&blob).0.is_empty());
    }

    #[test]
    fn blobs_are_padded() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let _b = sim.join("bob", &sec);
        let (_, short) = sim.clients[a].0.send_text("hi").unwrap();
        let (_, longer) = sim.clients[a].0.send_text(&"x".repeat(60)).unwrap();
        assert_eq!(short[0].len(), longer[0].len());
    }
}
