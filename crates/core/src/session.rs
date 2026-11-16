//! Room session state machine (sans-IO).
//!
//! The network layer feeds every blob from the relay mailbox, in order, into
//! [`Session::handle_blob`]; the session returns UI [`Event`]s plus blobs that must
//! be posted back. No sockets, no clocks except the message timestamp — easy to test.
//!
//! ## Cryptography (v0, "Sender Keys")
//!
//! Modelled on Signal's Sender Keys group scheme, built from audited primitives:
//!
//! - Each member creates fresh per-room keys: Ed25519 (identity/signing) and X25519.
//! - Each member owns a **sending chain**: `mk_i = HKDF(ck_i, "msg")`,
//!   `ck_{i+1} = HKDF(ck_i, "next")`. Used keys are deleted → forward secrecy.
//! - A member's chain key is handed to each other member individually, encrypted with
//!   `HKDF(X25519(sender, recipient))`. A passive attacker who knows the room password
//!   therefore still cannot read messages.
//! - When someone leaves, everyone re-keys (new epoch) so the leaver cannot read on.
//! - Newcomers receive the chain at its *current* index, so they cannot read history.
//! - Every message is signed; nobody can impersonate another member's identity key.
//!
//! Roadmap: replace with libsignal (PQXDH + Double Ratchet pairwise sessions) for
//! post-compromise security.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::crypto;
use crate::proto::*;
use crate::{Error, Nickname, RoomSecrets};

const MAX_SKIP: u32 = 2000;
const MAX_SKIPPED_KEYS: usize = 4000;
/// Maximum characters per chat message.
pub const MAX_TEXT_CHARS: usize = 4000;

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
}

#[derive(Zeroize, ZeroizeOnDrop)]
struct SendChain {
    epoch: u32,
    ck: [u8; 32],
    index: u32,
}

impl SendChain {
    fn fresh(epoch: u32) -> Self {
        SendChain {
            epoch,
            ck: crypto::random32(),
            index: 0,
        }
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
struct RecvChain {
    ck: [u8; 32],
    next: u32,
    #[zeroize(skip)]
    skipped: HashMap<u32, Zeroizing<[u8; 32]>>,
}

fn chain_step(ck: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    (
        crypto::expand(ck, &[b"ghostline/v1/chain/msg"]),
        crypto::expand(ck, &[b"ghostline/v1/chain/next"]),
    )
}

impl RecvChain {
    fn key_for(&mut self, index: u32) -> Option<Zeroizing<[u8; 32]>> {
        if index < self.next {
            return self.skipped.remove(&index);
        }
        if index - self.next > MAX_SKIP {
            return None;
        }
        while self.next < index {
            let (mk, next) = chain_step(&self.ck);
            self.skipped.insert(self.next, Zeroizing::new(mk));
            self.ck = next;
            self.next += 1;
        }
        let (mk, next) = chain_step(&self.ck);
        self.ck = next;
        self.next += 1;
        while self.skipped.len() > MAX_SKIPPED_KEYS {
            let oldest = *self.skipped.keys().min().expect("non-empty");
            self.skipped.remove(&oldest);
        }
        Some(Zeroizing::new(mk))
    }
}

struct Member {
    nick: String,
    dh: [u8; 32],
    chains: BTreeMap<u32, RecvChain>,
    active: bool,
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

/// Short, human-comparable fingerprint of a member key (6 hex chars).
pub fn fingerprint(m: &MemberId) -> String {
    m[..3].iter().map(|b| format!("{b:02x}")).collect()
}

fn display(nick: &str, m: &MemberId) -> String {
    format!("{nick}#{}", fingerprint(m))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One user's view of one room.
pub struct Session {
    room_key: Zeroizing<[u8; 32]>,
    sign: SigningKey,
    dh: StaticSecret,
    me: MemberId,
    nick: Nickname,
    send: SendChain,
    members: HashMap<MemberId, Member>,
    outbox: HashMap<MsgId, Outgoing>,
    pending_delivered: Vec<MsgId>,
}

impl Session {
    /// Fresh per-room identity.
    pub fn new(nick: Nickname, secrets: &RoomSecrets) -> Self {
        let sign = SigningKey::generate(&mut OsRng);
        let me = sign.verifying_key().to_bytes();
        Session {
            room_key: Zeroizing::new(secrets.room_key),
            sign,
            dh: StaticSecret::random_from_rng(OsRng),
            me,
            nick,
            send: SendChain::fresh(0),
            members: HashMap::new(),
            outbox: HashMap::new(),
            pending_delivered: Vec::new(),
        }
    }

    /// Our display name (`nick#fingerprint`).
    pub fn me_display(&self) -> String {
        display(self.nick.as_str(), &self.me)
    }

    /// Display names of the other active members.
    pub fn members(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .members
            .iter()
            .filter(|(_, m)| m.active)
            .map(|(id, m)| display(&m.nick, id))
            .collect();
        v.sort();
        v
    }

    /// Blob announcing ourselves. Post this first.
    pub fn hello_blob(&self) -> Result<Vec<u8>, Error> {
        let body = HelloBody {
            member: self.me,
            dh: PublicKey::from(&self.dh).to_bytes(),
            nick: self.nick.as_str().to_owned(),
        };
        seal_outer(
            &self.room_key,
            &Inner::Hello(Signed::new(&self.sign, KIND_HELLO, body)),
        )
    }

    fn keydist_key(
        &self,
        their_dh: &[u8; 32],
        from: &MemberId,
        to: &MemberId,
        epoch: u32,
    ) -> Result<Zeroizing<[u8; 32]>, Error> {
        let shared = self.dh.diffie_hellman(&PublicKey::from(*their_dh));
        if !shared.was_contributory() {
            return Err(Error::UnknownKey);
        }
        Ok(Zeroizing::new(crypto::expand(
            shared.as_bytes(),
            &[b"ghostline/v1/keydist", from, to, &epoch.to_be_bytes()],
        )))
    }

    fn keydist_aad(from: &MemberId, to: &MemberId, epoch: u32) -> Vec<u8> {
        [&from[..], &to[..], &epoch.to_be_bytes()].concat()
    }

    fn keydist_to(&self, to: &MemberId, their_dh: &[u8; 32]) -> Result<Vec<u8>, Error> {
        let epoch = self.send.epoch;
        let key = self.keydist_key(their_dh, &self.me, to, epoch)?;
        let mut pt = Zeroizing::new([0u8; 36]);
        pt[..32].copy_from_slice(&self.send.ck);
        pt[32..].copy_from_slice(&self.send.index.to_be_bytes());
        let (nonce, ct) = crypto::seal(&key, &Self::keydist_aad(&self.me, to, epoch), &pt[..]);
        let body = KeyDistBody {
            from: self.me,
            to: *to,
            epoch,
            nonce,
            ct,
        };
        seal_outer(
            &self.room_key,
            &Inner::KeyDist(Signed::new(&self.sign, KIND_KEYDIST, body)),
        )
    }

    fn rekey(&mut self) -> Vec<Vec<u8>> {
        self.send = SendChain::fresh(self.send.epoch + 1);
        let targets: Vec<(MemberId, [u8; 32])> = self
            .members
            .iter()
            .filter(|(_, m)| m.active)
            .map(|(id, m)| (*id, m.dh))
            .collect();
        targets
            .iter()
            .filter_map(|(id, dh)| self.keydist_to(id, dh).ok())
            .collect()
    }

    fn msg_aad(from: &MemberId, epoch: u32, index: u32) -> Vec<u8> {
        [&from[..], &epoch.to_be_bytes(), &index.to_be_bytes()].concat()
    }

    fn encrypt_content(&mut self, content: &Content) -> Result<Vec<u8>, Error> {
        let (mk, next) = chain_step(&self.send.ck);
        let mk = Zeroizing::new(mk);
        let index = self.send.index;
        self.send.ck = next;
        self.send.index += 1;
        let epoch = self.send.epoch;
        let (nonce, ct) = crypto::seal(
            &mk,
            &Self::msg_aad(&self.me, epoch, index),
            &encode(content),
        );
        let body = MsgBody {
            from: self.me,
            epoch,
            index,
            nonce,
            ct,
        };
        seal_outer(
            &self.room_key,
            &Inner::Msg(Signed::new(&self.sign, KIND_MSG, body)),
        )
    }

    /// Encrypt a chat message. Returns its id and the blob to post.
    pub fn send_text(&mut self, text: &str) -> Result<(MsgId, Vec<u8>), Error> {
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(Error::TooLarge);
        }
        let id = crypto::random16();
        let blob = self.encrypt_content(&Content::Text {
            id,
            text: text.to_owned(),
            ts: now(),
        })?;
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
        Ok((id, blob))
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
        self.encrypt_content(&Content::Receipt {
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
        self.encrypt_content(&Content::Receipt {
            kind: ReceiptKind::Read,
            ids,
        })
        .ok()
    }

    /// "I'm leaving" — post before closing.
    pub fn leave_blob(&mut self) -> Result<Vec<u8>, Error> {
        self.encrypt_content(&Content::Leave)
    }

    /// "Room burned" — post, then DELETE the mailbox.
    pub fn burn_blob(&mut self) -> Result<Vec<u8>, Error> {
        self.encrypt_content(&Content::Burn)
    }

    /// Process one blob from the mailbox. Undecryptable / foreign / invalid blobs
    /// are silently ignored (e.g. history from before we joined).
    pub fn handle_blob(&mut self, blob: &[u8]) -> (Vec<Event>, Vec<Vec<u8>>) {
        let mut events = Vec::new();
        let mut out = Vec::new();
        let _ = self.handle_inner(blob, &mut events, &mut out);
        (events, out)
    }

    fn handle_inner(
        &mut self,
        blob: &[u8],
        events: &mut Vec<Event>,
        out: &mut Vec<Vec<u8>>,
    ) -> Result<(), Error> {
        match open_outer(&self.room_key, blob)? {
            Inner::Hello(s) => {
                let b = &s.body;
                if b.member == self.me {
                    return Ok(());
                }
                s.verify(KIND_HELLO, &b.member)?;
                let nick = Nickname::new(&b.nick).map_err(|_| Error::Malformed)?;
                if self.members.get(&b.member).is_some_and(|m| m.active) {
                    return Ok(());
                }
                self.members.insert(
                    b.member,
                    Member {
                        nick: nick.as_str().to_owned(),
                        dh: b.dh,
                        chains: BTreeMap::new(),
                        active: true,
                    },
                );
                events.push(Event::Joined {
                    who: display(nick.as_str(), &b.member),
                });
                out.push(self.keydist_to(&b.member, &b.dh)?);
            }
            Inner::KeyDist(s) => {
                let b = &s.body;
                if b.to != self.me {
                    return Ok(());
                }
                let their_dh = self.members.get(&b.from).ok_or(Error::UnknownKey)?.dh;
                s.verify(KIND_KEYDIST, &b.from)?;
                let key = self.keydist_key(&their_dh, &b.from, &self.me, b.epoch)?;
                let pt = Zeroizing::new(crypto::open(
                    &key,
                    &b.nonce,
                    &Self::keydist_aad(&b.from, &self.me, b.epoch),
                    &b.ct,
                )?);
                if pt.len() != 36 {
                    return Err(Error::Malformed);
                }
                let m = self.members.get_mut(&b.from).ok_or(Error::UnknownKey)?;
                if m.chains.contains_key(&b.epoch) {
                    return Ok(());
                }
                let mut ck = [0u8; 32];
                ck.copy_from_slice(&pt[..32]);
                let next = u32::from_be_bytes(pt[32..36].try_into().expect("4 bytes"));
                m.chains.insert(
                    b.epoch,
                    RecvChain {
                        ck,
                        next,
                        skipped: HashMap::new(),
                    },
                );
                ck.zeroize();
                while m.chains.len() > 2 {
                    let oldest = *m.chains.keys().next().expect("non-empty");
                    m.chains.remove(&oldest);
                }
            }
            Inner::Msg(s) => {
                let b = &s.body;
                if b.from == self.me {
                    return Ok(());
                }
                s.verify(KIND_MSG, &b.from)?;
                let m = self.members.get_mut(&b.from).ok_or(Error::UnknownKey)?;
                let chain = m.chains.get_mut(&b.epoch).ok_or(Error::UnknownKey)?;
                let mk = chain.key_for(b.index).ok_or(Error::UnknownKey)?;
                let pt = Zeroizing::new(crypto::open(
                    &mk,
                    &b.nonce,
                    &Self::msg_aad(&b.from, b.epoch, b.index),
                    &b.ct,
                )?);
                let who = display(&m.nick, &b.from);
                let from = b.from;
                match decode::<Content>(&pt)? {
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
                    Content::Leave => {
                        m.active = false;
                        m.chains.clear();
                        events.push(Event::Left { who });
                        for (id, o) in self.outbox.iter_mut() {
                            o.recipients.remove(&from);
                            if let Some(status) = o.recompute() {
                                events.push(Event::Status { id: *id, status });
                            }
                        }
                        out.extend(self.rekey());
                    }
                    Content::Burn => events.push(Event::Burned { by: who }),
                }
            }
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SigningKey and StaticSecret zeroize themselves (zeroize feature);
        // chains are ZeroizeOnDrop; room_key is Zeroizing.
        self.members.clear();
        self.outbox.clear();
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
            let s = Session::new(Nickname::new(nick).unwrap(), secrets);
            self.mailbox.push(s.hello_blob().unwrap());
            self.clients.push((s, 0, vec![]));
            self.pump();
            self.clients.len() - 1
        }
        fn post(&mut self, blob: Vec<u8>) {
            self.mailbox.push(blob);
        }
        /// Deliver everything to everyone until quiescent.
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
            let (id, blob) = self.clients[who].0.send_text(text).unwrap();
            self.post(blob);
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

        // A malicious leaver keeps processing the mailbox, but gets no new keys.
        sim.send(a, "after carol left");
        assert_eq!(sim.texts(b), vec!["after carol left"]);
        assert!(sim.texts(c).is_empty());
    }

    #[test]
    fn password_holder_cannot_forge_or_read() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        sim.send(a, "only for members");

        // Eve knows the password (room_key) and reads the whole mailbox passively,
        // without posting a Hello: she can open the outer layer, but not the content.
        let mut eve = Session::new(Nickname::new("eve").unwrap(), &sec);
        let mut seen = vec![];
        for blob in sim.mailbox.clone() {
            seen.extend(eve.handle_blob(&blob).0);
        }
        assert!(!seen.iter().any(|e| matches!(e, Event::Message { .. })));

        // Eve replays alice's message: rejected (index already used).
        let alice = sim.clients[a].0.me;
        let replay = sim
            .mailbox
            .iter()
            .find(|blob| {
                matches!(open_outer(&sec.room_key, blob),
                    Ok(Inner::Msg(s)) if s.body.from == alice)
            })
            .unwrap()
            .clone();
        let before = sim.texts(b).len();
        sim.post(replay.clone());
        sim.pump();
        assert_eq!(sim.texts(b).len(), before);

        // Eve tampers with a signed message: signature fails.
        let Ok(Inner::Msg(mut s)) = open_outer(&sec.room_key, &replay) else {
            panic!("expected Msg")
        };
        s.body.index += 100;
        let forged = seal_outer(&sec.room_key, &Inner::Msg(s)).unwrap();
        sim.post(forged);
        sim.pump();
        assert_eq!(sim.texts(b).len(), before);

        // Eve posts a Hello pretending to be alice (alice's member id, Eve's keys):
        // she cannot produce alice's signature.
        let mut fake = match open_outer(&sec.room_key, &sim.mailbox[0]).unwrap() {
            Inner::Hello(h) => h,
            _ => panic!("first blob is alice's hello"),
        };
        fake.body.nick = "alice".into();
        fake.body.dh = PublicKey::from(&eve.dh).to_bytes();
        sim.post(seal_outer(&sec.room_key, &Inner::Hello(fake)).unwrap());
        sim.pump();
        assert_eq!(sim.texts(b).len(), before);
    }

    #[test]
    fn wrong_password_sees_nothing() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        sim.send(a, "x");
        let (_, other) = Sim::new();
        let mut mallory = Session::new(Nickname::new("mallory").unwrap(), &other);
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
    fn out_of_order_and_skipped_messages() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let b = sim.join("bob", &sec);
        let (_, m1) = sim.clients[a].0.send_text("one").unwrap();
        let (_, m2) = sim.clients[a].0.send_text("two").unwrap();
        // Deliver m2 before m1.
        let (ev2, _) = sim.clients[b].0.handle_blob(&m2);
        let (ev1, _) = sim.clients[b].0.handle_blob(&m1);
        assert!(matches!(&ev2[0], Event::Message { text, .. } if text == "two"));
        assert!(matches!(&ev1[0], Event::Message { text, .. } if text == "one"));
        // And never twice.
        assert!(sim.clients[b].0.handle_blob(&m1).0.is_empty());
    }

    #[test]
    fn blobs_are_padded() {
        let (mut sim, sec) = Sim::new();
        let a = sim.join("alice", &sec);
        let _b = sim.join("bob", &sec);
        let (_, short) = sim.clients[a].0.send_text("hi").unwrap();
        let (_, longer) = sim.clients[a].0.send_text(&"x".repeat(60)).unwrap();
        assert_eq!(short.len(), longer.len());
    }
}
