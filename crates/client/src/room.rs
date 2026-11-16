//! A running room.
//!
//! The [`Session`] (all keys) lives on the UI thread inside [`Room`]. Two background
//! tasks do the I/O: a poller that fetches new blobs in order, and a sender that
//! posts strictly in order. They talk to the UI thread over channels and call a
//! `notify` hook so GUIs can repaint immediately.

use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ghostline_core::proto::MsgId;
use ghostline_core::{Event, Nickname, RoomId, Session};
use tokio::sync::mpsc as tk_mpsc;
use zeroize::Zeroizing;

use crate::net::{Fetch, Post, Relay, Route};
use crate::setup::Prepared;

/// Callback used to wake up the UI (e.g. `egui::Context::request_repaint`).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy)]
enum PostTag {
    Msg(MsgId),
    Leave,
    Other,
}

enum NetCmd {
    Post(Vec<u8>, PostTag),
    Burn(Zeroizing<[u8; 32]>),
}

enum NetEvent {
    Blobs(Vec<Vec<u8>>),
    Posted(PostTag),
    Vanished,
    BurnDone,
    Error(String),
    Online,
}

/// What a UI needs to react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomEvent {
    /// Protocol event (message, join, status change, burn…).
    Core(Event),
    /// Relay unreachable (will keep retrying).
    ConnError(String),
    /// Relay reachable again.
    Online,
    /// Our "leave" was delivered — safe to close.
    LeftDone,
    /// Our burn completed — room deleted on the relay.
    BurnDone,
    /// Room disappeared from the relay (burned by someone).
    Vanished,
}

/// A joined room.
pub struct Room {
    session: Session,
    tx: tk_mpsc::UnboundedSender<NetCmd>,
    rx: std_mpsc::Receiver<NetEvent>,
    delete_token: Zeroizing<[u8; 32]>,
    unread: Vec<MsgId>,
    /// Room ID.
    pub id: RoomId,
    /// Room password (kept so the user can share it again).
    pub password: Zeroizing<String>,
    /// e.g. "tor 127.0.0.1:9050" or "direct".
    pub route_label: String,
    /// Send read receipts?
    pub read_receipts: bool,
    leaving: bool,
    burning: bool,
}

impl Room {
    /// Enter a prepared room: posts our Hello and starts the network tasks.
    pub fn enter(
        rt: &tokio::runtime::Handle,
        prepared: Prepared,
        nick: Nickname,
        read_receipts: bool,
        notify: Notify,
    ) -> Result<Self> {
        let Prepared {
            id,
            password,
            secrets,
            relay,
            route,
        } = prepared;
        let session = Session::new(nick, &secrets);
        let hello = session.hello_blob()?;
        let poll = match route {
            Route::Direct => Duration::from_millis(700),
            Route::Socks(_) | Route::Embedded(_) => Duration::from_millis(1500),
        };
        let (tx, rx) = spawn_network(rt, Arc::new(relay), poll, notify);
        let _ = tx.send(NetCmd::Post(hello, PostTag::Other));
        Ok(Room {
            session,
            tx,
            rx,
            delete_token: Zeroizing::new(secrets.delete_token),
            unread: Vec::new(),
            id,
            password,
            route_label: route.label(),
            read_receipts,
            leaving: false,
            burning: false,
        })
    }

    /// Our display name (`nick#fingerprint`).
    pub fn me(&self) -> String {
        self.session.me_display()
    }

    /// Other active members.
    pub fn members(&self) -> Vec<String> {
        self.session.members()
    }

    /// Drain network events, run them through the session, return UI events.
    pub fn poll(&mut self) -> Vec<RoomEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                NetEvent::Blobs(blobs) => {
                    for b in blobs {
                        let (evs, posts) = self.session.handle_blob(&b);
                        for p in posts {
                            let _ = self.tx.send(NetCmd::Post(p, PostTag::Other));
                        }
                        for e in evs {
                            if let Event::Message { id, .. } = &e {
                                self.unread.push(*id);
                            }
                            out.push(RoomEvent::Core(e));
                        }
                    }
                    if let Some(r) = self.session.take_delivered_receipt() {
                        let _ = self.tx.send(NetCmd::Post(r, PostTag::Other));
                    }
                }
                NetEvent::Posted(PostTag::Msg(id)) => {
                    if let Some(e) = self.session.mark_sent(&id) {
                        out.push(RoomEvent::Core(e));
                    }
                }
                NetEvent::Posted(PostTag::Leave) => {
                    if self.leaving {
                        out.push(RoomEvent::LeftDone)
                    }
                }
                NetEvent::Posted(PostTag::Other) => {}
                NetEvent::BurnDone => out.push(RoomEvent::BurnDone),
                NetEvent::Vanished => {
                    if self.burning {
                        out.push(RoomEvent::BurnDone)
                    } else {
                        out.push(RoomEvent::Vanished)
                    }
                }
                NetEvent::Error(e) => out.push(RoomEvent::ConnError(e)),
                NetEvent::Online => out.push(RoomEvent::Online),
            }
        }
        out
    }

    /// Encrypt and send a message. Returns its id (track it for ✓/✓✓).
    pub fn send_text(&mut self, text: &str) -> Result<MsgId> {
        let (id, blob) = self.session.send_text(text)?;
        let _ = self.tx.send(NetCmd::Post(blob, PostTag::Msg(id)));
        Ok(id)
    }

    /// The user is looking at the conversation: send read receipts for what arrived.
    pub fn mark_read(&mut self) {
        if !self.read_receipts || self.unread.is_empty() {
            return;
        }
        let ids = std::mem::take(&mut self.unread);
        if let Some(r) = self.session.read_receipt(ids) {
            let _ = self.tx.send(NetCmd::Post(r, PostTag::Other));
        }
    }

    /// Announce that we leave (others re-key). Wait for [`RoomEvent::LeftDone`].
    pub fn leave(&mut self) {
        if self.leaving || self.burning {
            return;
        }
        self.leaving = true;
        if let Ok(b) = self.session.leave_blob() {
            let _ = self.tx.send(NetCmd::Post(b, PostTag::Leave));
        }
    }

    /// Burn the room for everyone. Wait for [`RoomEvent::BurnDone`].
    pub fn burn(&mut self) {
        if self.burning {
            return;
        }
        self.burning = true;
        if let Ok(b) = self.session.burn_blob() {
            let _ = self.tx.send(NetCmd::Post(b, PostTag::Other));
        }
        let _ = self.tx.send(NetCmd::Burn(self.delete_token.clone()));
    }
}

fn spawn_network(
    rt: &tokio::runtime::Handle,
    relay: Arc<Relay>,
    poll: Duration,
    notify: Notify,
) -> (
    tk_mpsc::UnboundedSender<NetCmd>,
    std_mpsc::Receiver<NetEvent>,
) {
    let (cmd_tx, mut cmd_rx) = tk_mpsc::unbounded_channel::<NetCmd>();
    let (ev_tx, ev_rx) = std_mpsc::channel::<NetEvent>();

    let send = {
        let notify = notify.clone();
        move |tx: &std_mpsc::Sender<NetEvent>, e: NetEvent| -> bool {
            let ok = tx.send(e).is_ok();
            notify();
            ok
        }
    };

    // Poller.
    let (r, tx, send1) = (relay.clone(), ev_tx.clone(), send.clone());
    rt.spawn(async move {
        let mut since = 0u64;
        let mut failing = false;
        loop {
            match r.fetch(since).await {
                Ok(Fetch::Blobs(v)) => {
                    if failing {
                        failing = false;
                        send1(&tx, NetEvent::Online);
                    }
                    let more = v.len() >= 256;
                    if let Some((last, _)) = v.last() {
                        since = last + 1;
                    }
                    if !v.is_empty()
                        && !send1(
                            &tx,
                            NetEvent::Blobs(v.into_iter().map(|(_, b)| b).collect()),
                        )
                    {
                        return;
                    }
                    if more {
                        continue;
                    }
                }
                Ok(Fetch::NotFound) | Ok(Fetch::Gone) => {
                    send1(&tx, NetEvent::Vanished);
                    return;
                }
                Err(e) => {
                    failing = true;
                    if !send1(&tx, NetEvent::Error(e.to_string())) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
            tokio::time::sleep(poll).await;
        }
    });

    // Sender (strict order).
    rt.spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                NetCmd::Post(blob, tag) => {
                    let mut tries = 0;
                    loop {
                        match relay.post(blob.clone()).await {
                            Ok(Post::Ok) => {
                                send(&ev_tx, NetEvent::Posted(tag));
                                break;
                            }
                            Ok(Post::NotFound) | Ok(Post::Gone) => {
                                send(&ev_tx, NetEvent::Vanished);
                                break;
                            }
                            Err(e) => {
                                tries += 1;
                                send(&ev_tx, NetEvent::Error(e.to_string()));
                                if tries >= 5 {
                                    break;
                                }
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            }
                        }
                    }
                }
                NetCmd::Burn(token) => {
                    let _ = relay.burn(&token).await;
                    send(&ev_tx, NetEvent::BurnDone);
                }
            }
        }
    });

    (cmd_tx, ev_rx)
}
