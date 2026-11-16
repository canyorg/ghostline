//! Green-on-black terminal UI (ratatui + crossterm; works on Linux, macOS and
//! Windows Terminal / PowerShell).

use std::collections::HashMap;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{Local, TimeZone};
use ghostline_core::proto::MsgId;
use ghostline_core::{Event, RoomId, Session, Status};
use ratatui::crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;
use tokio::sync::mpsc as tk_mpsc;
use unicode_width::UnicodeWidthChar;
use zeroize::Zeroizing;

use crate::net::{Fetch, Post, Relay};

const GREEN: Color = Color::Green;
const BRIGHT: Color = Color::LightGreen;
const DIM: Color = Color::DarkGray;

/// What the room needs besides the session.
pub struct RoomInfo {
    pub id: RoomId,
    pub password: Zeroizing<String>,
    pub route_label: String,
    pub delete_token: Zeroizing<[u8; 32]>,
    pub poll_interval: Duration,
    pub read_receipts: bool,
}

/// How the chat ended.
pub enum Exit {
    Quit,
    BurnedByMe,
    BurnedBy(String),
    Vanished,
}

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

fn spawn_network(
    rt: &tokio::runtime::Runtime,
    relay: Arc<Relay>,
    poll: Duration,
) -> (
    tk_mpsc::UnboundedSender<NetCmd>,
    std_mpsc::Receiver<NetEvent>,
) {
    let (cmd_tx, mut cmd_rx) = tk_mpsc::unbounded_channel::<NetCmd>();
    let (ev_tx, ev_rx) = std_mpsc::channel::<NetEvent>();

    // Poller: fetch new blobs in order.
    let (r, tx) = (relay.clone(), ev_tx.clone());
    rt.spawn(async move {
        let mut since = 0u64;
        let mut failing = false;
        loop {
            match r.fetch(since).await {
                Ok(Fetch::Blobs(v)) => {
                    if failing {
                        failing = false;
                        let _ = tx.send(NetEvent::Online);
                    }
                    let more = v.len() >= 256;
                    if let Some((last, _)) = v.last() {
                        since = last + 1;
                    }
                    if !v.is_empty()
                        && tx
                            .send(NetEvent::Blobs(v.into_iter().map(|(_, b)| b).collect()))
                            .is_err()
                    {
                        return;
                    }
                    if more {
                        continue;
                    }
                }
                Ok(Fetch::NotFound) | Ok(Fetch::Gone) => {
                    let _ = tx.send(NetEvent::Vanished);
                    return;
                }
                Err(e) => {
                    failing = true;
                    if tx.send(NetEvent::Error(e.to_string())).is_err() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
            tokio::time::sleep(poll).await;
        }
    });

    // Sender: posts strictly in order.
    rt.spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                NetCmd::Post(blob, tag) => {
                    let mut tries = 0;
                    loop {
                        match relay.post(blob.clone()).await {
                            Ok(Post::Ok) => {
                                let _ = ev_tx.send(NetEvent::Posted(tag));
                                break;
                            }
                            Ok(Post::NotFound) | Ok(Post::Gone) => {
                                let _ = ev_tx.send(NetEvent::Vanished);
                                break;
                            }
                            Err(e) => {
                                tries += 1;
                                let _ = ev_tx.send(NetEvent::Error(e.to_string()));
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
                    let _ = ev_tx.send(NetEvent::BurnDone);
                }
            }
        }
    });

    (cmd_tx, ev_rx)
}

enum Entry {
    Sys(String, Color),
    Msg {
        id: Option<MsgId>,
        ts: u64,
        from: String,
        text: String,
        mine: bool,
    },
}

enum Mode {
    Chat,
    Leaving(Instant),
    Burning(Instant),
    Ending(Instant, Exit),
}

struct Ui {
    entries: Vec<Entry>,
    statuses: HashMap<MsgId, Status>,
    input: String,
    scroll: usize,
    me: String,
    room: String,
    route: String,
    members: usize,
    conn_error: Option<String>,
    unread: Vec<MsgId>,
    mode: Mode,
}

impl Ui {
    fn sys(&mut self, s: impl Into<String>) {
        self.entries.push(Entry::Sys(s.into(), GREEN));
    }
    fn alert(&mut self, s: impl Into<String>, c: Color) {
        self.entries.push(Entry::Sys(s.into(), c));
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

const BANNER: &[&str] = &[
    " ██████╗ ██╗  ██╗ ██████╗ ███████╗████████╗██╗     ██╗███╗   ██╗███████╗",
    "██╔════╝ ██║  ██║██╔═══██╗██╔════╝╚══██╔══╝██║     ██║████╗  ██║██╔════╝",
    "██║  ███╗███████║██║   ██║███████╗   ██║   ██║     ██║██╔██╗ ██║█████╗  ",
    "██║   ██║██╔══██║██║   ██║╚════██║   ██║   ██║     ██║██║╚██╗██║██╔══╝  ",
    "╚██████╔╝██║  ██║╚██████╔╝███████║   ██║   ███████╗██║██║ ╚████║███████╗",
    " ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝   ╚══════╝╚═╝╚═╝  ╚═══╝╚══════╝",
];

/// Run the chat until the user quits or the room is burned.
pub fn run(
    rt: &tokio::runtime::Runtime,
    relay: Arc<Relay>,
    mut session: Session,
    info: RoomInfo,
) -> Result<Exit> {
    let (net, events) = spawn_network(rt, relay, info.poll_interval);

    let mut ui = Ui {
        entries: Vec::new(),
        statuses: HashMap::new(),
        input: String::new(),
        scroll: 0,
        me: session.me_display(),
        room: info.id.display_grouped(),
        route: info.route_label.clone(),
        members: 0,
        conn_error: None,
        unread: Vec::new(),
        mode: Mode::Chat,
    };
    for l in BANNER {
        ui.entries.push(Entry::Sys((*l).to_string(), GREEN));
    }
    ui.sys("");
    ui.sys(format!("> connected to room {} via {}", ui.room, ui.route));
    ui.sys(format!(
        "> you are {} — the #code is your key fingerprint",
        ui.me
    ));
    ui.sys("> end-to-end encrypted · no accounts · no logs · type /help");
    ui.sys("");

    let mut terminal = ratatui::init();
    let result = (|| -> Result<Exit> {
        loop {
            // 1) Network events.
            while let Ok(ev) = events.try_recv() {
                match ev {
                    NetEvent::Blobs(blobs) => {
                        for b in blobs {
                            let (evs, out) = session.handle_blob(&b);
                            for o in out {
                                let _ = net.send(NetCmd::Post(o, PostTag::Other));
                            }
                            for e in evs {
                                apply_event(&mut ui, e);
                            }
                        }
                        if let Some(r) = session.take_delivered_receipt() {
                            let _ = net.send(NetCmd::Post(r, PostTag::Other));
                        }
                        ui.members = session.members().len();
                    }
                    NetEvent::Posted(PostTag::Msg(id)) => {
                        if let Some(e) = session.mark_sent(&id) {
                            apply_event(&mut ui, e);
                        }
                    }
                    NetEvent::Posted(PostTag::Leave) => {
                        if matches!(ui.mode, Mode::Leaving(_)) {
                            return Ok(Exit::Quit);
                        }
                    }
                    NetEvent::Posted(PostTag::Other) => {}
                    NetEvent::BurnDone => return Ok(Exit::BurnedByMe),
                    NetEvent::Vanished => {
                        if matches!(ui.mode, Mode::Burning(_)) {
                            return Ok(Exit::BurnedByMe);
                        }
                        if !matches!(ui.mode, Mode::Ending(..)) {
                            ui.alert(
                                "> 🔥 ROOM BURNED — deleted from relay, wiping keys",
                                Color::Red,
                            );
                            ui.mode = Mode::Ending(Instant::now(), Exit::Vanished);
                        }
                    }
                    NetEvent::Error(e) => ui.conn_error = Some(e),
                    NetEvent::Online => ui.conn_error = None,
                }
            }

            // 2) Timeouts for leaving / burning / ending.
            match &ui.mode {
                Mode::Leaving(t) if t.elapsed() > Duration::from_secs(4) => return Ok(Exit::Quit),
                Mode::Burning(t) if t.elapsed() > Duration::from_secs(8) => {
                    return Ok(Exit::BurnedByMe)
                }
                Mode::Ending(t, _) if t.elapsed() > Duration::from_millis(1800) => {
                    if let Mode::Ending(_, exit) = std::mem::replace(&mut ui.mode, Mode::Chat) {
                        return Ok(exit);
                    }
                }
                _ => {}
            }

            // 3) Draw.
            terminal.draw(|f| draw(f, &ui))?;

            // 4) Keyboard.
            if !event::poll(Duration::from_millis(50))? {
                continue;
            }
            let CEvent::Key(k) = event::read()? else {
                continue;
            };
            if k.kind != KeyEventKind::Press {
                continue; // Windows reports press + release
            }
            // Any key press = the user is looking: send read receipts.
            if info.read_receipts && !ui.unread.is_empty() {
                let ids = std::mem::take(&mut ui.unread);
                if let Some(r) = session.read_receipt(ids) {
                    let _ = net.send(NetCmd::Post(r, PostTag::Other));
                }
            }
            if !matches!(ui.mode, Mode::Chat) {
                continue;
            }
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            match k.code {
                KeyCode::Char('c') | KeyCode::Char('d') if ctrl => {
                    start_leave(&mut ui, &mut session, &net);
                }
                KeyCode::Char('u') if ctrl => ui.input.clear(),
                KeyCode::Char(c) => {
                    if ui.input.chars().count() < ghostline_core::session::MAX_TEXT_CHARS {
                        ui.input.push(c);
                    }
                }
                KeyCode::Backspace => {
                    ui.input.pop();
                }
                KeyCode::PageUp | KeyCode::Up => {
                    ui.scroll += if k.code == KeyCode::Up { 1 } else { 10 }
                }
                KeyCode::PageDown | KeyCode::Down => {
                    ui.scroll =
                        ui.scroll
                            .saturating_sub(if k.code == KeyCode::Down { 1 } else { 10 })
                }
                KeyCode::End => ui.scroll = 0,
                KeyCode::Enter => {
                    let line = std::mem::take(&mut ui.input);
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    ui.scroll = 0;
                    if let Some(cmd) = line.strip_prefix('/') {
                        command(cmd, &mut ui, &mut session, &net, &info);
                    } else {
                        match session.send_text(line) {
                            Ok((id, blob)) => {
                                ui.statuses.insert(id, Status::Pending);
                                ui.entries.push(Entry::Msg {
                                    id: Some(id),
                                    ts: now_secs(),
                                    from: ui.me.clone(),
                                    text: line.to_string(),
                                    mine: true,
                                });
                                let _ = net.send(NetCmd::Post(blob, PostTag::Msg(id)));
                            }
                            Err(e) => ui.alert(format!("> not sent: {e}"), Color::Red),
                        }
                    }
                }
                _ => {}
            }
        }
    })();
    ratatui::restore();
    result
}

fn start_leave(ui: &mut Ui, session: &mut Session, net: &tk_mpsc::UnboundedSender<NetCmd>) {
    ui.sys("> leaving room…");
    if let Ok(b) = session.leave_blob() {
        let _ = net.send(NetCmd::Post(b, PostTag::Leave));
    }
    ui.mode = Mode::Leaving(Instant::now());
}

fn command(
    cmd: &str,
    ui: &mut Ui,
    session: &mut Session,
    net: &tk_mpsc::UnboundedSender<NetCmd>,
    info: &RoomInfo,
) {
    match cmd.split_whitespace().next().unwrap_or("") {
        "help" | "h" | "?" => {
            ui.sys("> /who     list members");
            ui.sys("> /info    show room id + password (to share)");
            ui.sys("> /clear   clear the screen");
            ui.sys("> /quit    leave the room (others re-key)  · Ctrl+C");
            ui.sys("> /burn    DELETE the room for everyone, instantly");
            ui.sys("> ↑/↓ PgUp/PgDn scroll · End jump to bottom · Ctrl+U clear input");
            ui.sys("> ✓ on relay · ✓✓ delivered to all · ✓✓ (cyan) read by all");
        }
        "who" => {
            let m = session.members();
            ui.sys(format!("> {} (you)", ui.me));
            if m.is_empty() {
                ui.sys("> nobody else here yet");
            }
            for x in m {
                ui.sys(format!("> {x}"));
            }
        }
        "info" => {
            ui.sys(format!("> room id : {}", info.id.display_grouped()));
            ui.sys(format!("> password: {}", info.password.as_str()));
            ui.alert("> share both only over a channel you trust", Color::Yellow);
        }
        "clear" => ui.entries.clear(),
        "quit" | "exit" | "q" => start_leave(ui, session, net),
        "burn" => {
            ui.alert(
                "> 🔥 BURNING ROOM — deleting from relay, wiping keys",
                Color::Red,
            );
            if let Ok(b) = session.burn_blob() {
                let _ = net.send(NetCmd::Post(b, PostTag::Other));
            }
            let _ = net.send(NetCmd::Burn(info.delete_token.clone()));
            ui.mode = Mode::Burning(Instant::now());
        }
        other => ui.alert(
            format!("> unknown command /{other} — try /help"),
            Color::Yellow,
        ),
    }
}

fn apply_event(ui: &mut Ui, e: Event) {
    match e {
        Event::Joined { who } => ui.sys(format!("> {who} joined")),
        Event::Left { who } => ui.sys(format!("> {who} left — keys rotated")),
        Event::Message { id, from, text, ts } => {
            ui.unread.push(id);
            ui.entries.push(Entry::Msg {
                id: None,
                ts,
                from,
                text,
                mine: false,
            });
        }
        Event::Status { id, status } => {
            ui.statuses.insert(id, status);
        }
        Event::Burned { by } => {
            if !matches!(ui.mode, Mode::Ending(..)) {
                ui.alert(
                    format!("> 🔥 ROOM BURNED BY {by} — wiping keys"),
                    Color::Red,
                );
                ui.mode = Mode::Ending(Instant::now(), Exit::BurnedBy(by));
            }
        }
    }
}

fn width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Hard-wrap by display width, honouring newlines.
fn wrap(text: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        let mut cw = 0;
        for c in para.chars() {
            let cwid = c.width().unwrap_or(0);
            if cw + cwid > w && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                cw = 0;
            }
            cur.push(c);
            cw += cwid;
        }
        out.push(cur);
    }
    out
}

fn status_span(s: Option<&Status>) -> Span<'static> {
    match s {
        None | Some(Status::Pending) => Span::styled(" ⏳", Style::new().fg(DIM)),
        Some(Status::Sent) => Span::styled(" ✓", Style::new().fg(DIM)),
        Some(Status::Delivered) => Span::styled(" ✓✓", Style::new().fg(GREEN)),
        Some(Status::Read) => Span::styled(
            " ✓✓",
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
    }
}

fn build_rows(ui: &Ui, w: usize) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for e in &ui.entries {
        match e {
            Entry::Sys(t, c) => {
                for l in wrap(t, w) {
                    rows.push(Line::styled(l, Style::new().fg(*c)));
                }
            }
            Entry::Msg {
                id,
                ts,
                from,
                text,
                mine,
            } => {
                let time = Local
                    .timestamp_opt(*ts as i64, 0)
                    .single()
                    .map(|t| t.format("%H:%M").to_string())
                    .unwrap_or_else(|| "--:--".into());
                let time = format!("[{time}] ");
                let who = if *mine {
                    "you".to_string()
                } else {
                    from.clone()
                };
                let sep = " › ";
                let pre_w = width(&time) + width(&who) + width(sep);
                let text_w = w.saturating_sub(pre_w).max(10);
                let nick_style = if *mine {
                    Style::new().fg(BRIGHT).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(GREEN).add_modifier(Modifier::BOLD)
                };
                let text_style = Style::new().fg(if *mine { BRIGHT } else { GREEN });
                let parts = wrap(text, text_w);
                let n = parts.len();
                for (i, p) in parts.into_iter().enumerate() {
                    let mut spans = if i == 0 {
                        vec![
                            Span::styled(time.clone(), Style::new().fg(DIM)),
                            Span::styled(who.clone(), nick_style),
                            Span::styled(sep, Style::new().fg(DIM)),
                        ]
                    } else {
                        vec![Span::raw(" ".repeat(pre_w))]
                    };
                    spans.push(Span::styled(p, text_style));
                    if i + 1 == n && *mine {
                        spans.push(status_span(id.as_ref().and_then(|i| ui.statuses.get(i))));
                    }
                    rows.push(Line::from(spans));
                }
            }
        }
    }
    rows
}

fn draw(f: &mut Frame, ui: &Ui) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(Color::Black)), area);
    let [head, body, input] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .areas(area);

    let border = |title: &'static str| {
        Block::bordered()
            .border_type(BorderType::Plain)
            .border_style(Style::new().fg(GREEN))
            .title(Span::styled(
                title,
                Style::new().fg(BRIGHT).add_modifier(Modifier::BOLD),
            ))
    };

    // Header
    let (net_txt, net_col) = match &ui.conn_error {
        None => (format!("via {}", ui.route), GREEN),
        Some(_) => ("reconnecting…".to_string(), Color::Yellow),
    };
    let header = Line::from(vec![
        Span::styled(" room ", Style::new().fg(DIM)),
        Span::styled(ui.room.clone(), Style::new().fg(BRIGHT)),
        Span::styled("  │  ", Style::new().fg(DIM)),
        Span::styled(ui.me.clone(), Style::new().fg(GREEN)),
        Span::styled("  │  ", Style::new().fg(DIM)),
        Span::styled(format!("{} online", ui.members + 1), Style::new().fg(GREEN)),
        Span::styled("  │  ", Style::new().fg(DIM)),
        Span::styled(net_txt, Style::new().fg(net_col)),
        Span::styled("  │  ", Style::new().fg(DIM)),
        Span::styled("E2E 🔒", Style::new().fg(BRIGHT)),
    ]);
    f.render_widget(Paragraph::new(header).block(border(" GHOSTLINE ")), head);

    // Messages
    let inner_w = body.width.saturating_sub(2) as usize;
    let inner_h = body.height.saturating_sub(2) as usize;
    let rows = build_rows(ui, inner_w);
    let max_scroll = rows.len().saturating_sub(inner_h);
    let scroll = ui.scroll.min(max_scroll);
    let end = rows.len() - scroll;
    let start = end.saturating_sub(inner_h);
    let visible: Vec<Line> = rows[start..end].to_vec();
    let title = if scroll > 0 {
        " messages ↑ (End = bottom) "
    } else {
        " messages "
    };
    f.render_widget(Paragraph::new(visible).block(border(title)), body);

    // Input
    let title = match ui.mode {
        Mode::Chat => " > type a message · /help ",
        Mode::Leaving(_) => " leaving… ",
        Mode::Burning(_) => " burning… ",
        Mode::Ending(..) => " closing… ",
    };
    let avail = input.width.saturating_sub(4) as usize;
    let mut shown: String = ui.input.clone();
    while width(&shown) > avail {
        shown.remove(0);
    }
    let line = Line::from(vec![
        Span::styled("> ", Style::new().fg(BRIGHT)),
        Span::styled(shown.clone(), Style::new().fg(BRIGHT)),
    ]);
    f.render_widget(Paragraph::new(line).block(border(title)), input);
    if matches!(ui.mode, Mode::Chat) {
        f.set_cursor_position(Position::new(
            input.x + 3 + width(&shown) as u16,
            input.y + 1,
        ));
    }
}
