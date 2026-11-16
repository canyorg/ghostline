//! Screens and state machine of the desktop app.

use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Key, Layout, Margin, RichText, ScrollArea,
    Sense, Stroke, TextEdit, Vec2,
};
use ghostline_client::{setup, NetConfig, Prepared, Room, RoomEvent};
use ghostline_core::proto::MsgId;
use ghostline_core::{password, Event, Nickname, RoomId, Status};
use zeroize::Zeroizing;

use crate::i18n::{Lang, T};
use crate::matrix::{self, Rain};
use crate::theme::*;

const BURN_HOLD: Duration = Duration::from_millis(1500);

pub struct GhostApp {
    rt: tokio::runtime::Runtime,
    lang: Lang,
    net: NetConfig,
    tor_port_text: String,
    read_receipts: bool,
    screen: Screen,
    rain: Rain,
    last_frame: Instant,
    capture_blocked: bool,
    copied: Option<(&'static str, Instant)>,
    quit_requested: bool,
}

enum Screen {
    Home,
    Create(CreateForm),
    Join(JoinForm),
    Working(Working),
    Created(Option<Prepared>, Nickname),
    Chat(Box<Chat>),
    Ended(End),
}

#[derive(Clone)]
enum End {
    Left,
    BurnedByMe,
    BurnedBy(String),
    Vanished,
}

struct CreateForm {
    nick: String,
    own: bool,
    pw: Zeroizing<String>,
    pw2: Zeroizing<String>,
    show: bool,
    suggestion: Zeroizing<String>,
    error: Option<String>,
}

impl CreateForm {
    fn new() -> Self {
        CreateForm {
            nick: String::new(),
            own: false,
            pw: Zeroizing::new(String::new()),
            pw2: Zeroizing::new(String::new()),
            show: false,
            suggestion: password::generate(password::DEFAULT_LENGTH),
            error: None,
        }
    }
}

struct JoinForm {
    id: String,
    pw: Zeroizing<String>,
    nick: String,
    show: bool,
    error: Option<String>,
}

enum Back {
    Create(CreateForm),
    Join(JoinForm),
}

struct Working {
    rx: mpsc::Receiver<anyhow::Result<Prepared>>,
    nick: Nickname,
    back: Option<Back>,
    created: bool,
}

enum Entry {
    Sys(String, Color32),
    Msg {
        id: Option<MsgId>,
        ts: u64,
        from: String,
        text: String,
        mine: bool,
    },
}

struct Chat {
    room: Room,
    me: String,
    entries: Vec<Entry>,
    statuses: HashMap<MsgId, Status>,
    input: String,
    show_members: bool,
    show_info: bool,
    conn_err: bool,
    burn_hold: Option<Instant>,
    ending: Option<(Instant, End)>,
    leaving: Option<Instant>,
    burning: Option<Instant>,
    stick_bottom: bool,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn card() -> Frame {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, DIM))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(24))
}

fn big_button(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    ui.add_sized(
        [300.0, 44.0],
        egui::Button::new(RichText::new(text).size(17.0).strong().color(color))
            .stroke(Stroke::new(1.5, color))
            .fill(Color32::from_rgba_unmultiplied(0, 40, 12, 200)),
    )
}

/// Big glowing title.
fn logo(ui: &mut egui::Ui) {
    let font = egui::FontId::monospace(46.0);
    let text = "GHOSTLINE";
    let galley = ui
        .painter()
        .layout_no_wrap(text.into(), font.clone(), GREEN);
    let (rect, _) = ui.allocate_exact_size(galley.size() + Vec2::new(0.0, 6.0), Sense::hover());
    let p = ui.painter();
    for (dx, dy, a) in [
        (-2.0, 0.0, 40),
        (2.0, 0.0, 40),
        (0.0, -2.0, 40),
        (0.0, 2.0, 40),
        (0.0, 0.0, 255),
    ] {
        let col = if a == 255 {
            GREEN
        } else {
            Color32::from_rgba_unmultiplied(0, 255, 65, a)
        };
        p.text(
            rect.center() + Vec2::new(dx, dy),
            Align2::CENTER_CENTER,
            text,
            font.clone(),
            col,
        );
    }
}

impl GhostApp {
    pub fn new(cc: &eframe::CreationContext<'_>, capture_blocked: bool) -> Self {
        crate::theme::apply(&cc.egui_ctx);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let net = NetConfig {
            relay: std::env::var("GHOSTLINE_RELAY")
                .unwrap_or_else(|_| "http://127.0.0.1:7878".into()),
            ..NetConfig::default()
        };
        GhostApp {
            rt,
            lang: Lang::detect(),
            tor_port_text: net.tor_port.to_string(),
            net,
            read_receipts: true,
            screen: Screen::Home,
            rain: Rain::new(),
            last_frame: Instant::now(),
            capture_blocked,
            copied: None,
            quit_requested: false,
        }
    }

    fn t(&self) -> &'static T {
        self.lang.t()
    }

    fn copy_button(&mut self, ui: &mut egui::Ui, key: &'static str, value: &str) {
        let t = self.t();
        let fresh = matches!(self.copied, Some((k, at)) if k == key && at.elapsed() < Duration::from_secs(2));
        let label = if fresh { t.copied } else { t.copy };
        if ui.button(label).clicked() {
            ui.ctx().copy_text(value.to_owned());
            self.copied = Some((key, Instant::now()));
        }
    }

    // ───────────────────────────── Home ─────────────────────────────

    fn home(&mut self, ctx: &egui::Context) {
        let t = self.t();
        let mut next = None;
        egui::Area::new(egui::Id::unique("home"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                card().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        logo(ui);
                        ui.add_space(6.0);
                        ui.label(RichText::new(t.tagline).size(18.0).strong().color(GREEN));
                        ui.label(RichText::new(t.subtitle).small().color(DIM));
                        ui.add_space(18.0);
                        if big_button(ui, &format!("▸ {}", t.create_chat), GREEN).clicked() {
                            next = Some(Screen::Create(CreateForm::new()));
                        }
                        ui.add_space(4.0);
                        if big_button(ui, &format!("▸ {}", t.join_room), GREEN).clicked() {
                            next = Some(Screen::Join(JoinForm {
                                id: String::new(),
                                pw: Zeroizing::new(String::new()),
                                nick: String::new(),
                                show: false,
                                error: None,
                            }));
                        }
                        ui.add_space(14.0);
                    });
                    egui::CollapsingHeader::new(RichText::new(t.network).color(DIM))
                        .default_open(false)
                        .show(ui, |ui| self.network_settings(ui));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        for l in Lang::ALL {
                            let sel = self.lang == l;
                            let txt = RichText::new(l.label()).color(if sel { GREEN } else { DIM });
                            if ui.selectable_label(sel, txt).clicked() {
                                self.lang = l;
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (txt, col) = if self.capture_blocked {
                                (format!("🛡 {}", t.capture_on), DIM)
                            } else {
                                (format!("⚠ {}", t.capture_off), AMBER)
                            };
                            ui.label(RichText::new(txt).small().color(col));
                        });
                    });
                });
            });
        if let Some(s) = next {
            self.screen = s;
        }
    }

    fn network_settings(&mut self, ui: &mut egui::Ui) {
        let t = self.t();
        egui::Grid::new("net")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(t.relay);
                ui.add(TextEdit::singleline(&mut self.net.relay).desired_width(320.0));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut self.net.tor, t.use_tor);
                ui.end_row();
                ui.label(t.tor_port);
                ui.horizontal(|ui| {
                    if ui
                        .add(TextEdit::singleline(&mut self.tor_port_text).desired_width(60.0))
                        .changed()
                    {
                        if let Ok(p) = self.tor_port_text.trim().parse() {
                            self.net.tor_port = p;
                        }
                    }
                    ui.label(RichText::new(t.tor_hint).small().color(DIM));
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut self.read_receipts, t.read_receipts);
                ui.end_row();
                ui.label("");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.net.no_tor, t.allow_direct);
                    if self.net.no_tor {
                        ui.label(RichText::new(t.allow_direct_warn).small().color(RED));
                    }
                });
                ui.end_row();
            });
    }

    // ──────────────────────────── Create ────────────────────────────

    fn create(&mut self, ctx: &egui::Context, mut f: CreateForm) -> Screen {
        let t = self.t();
        let mut go_back = false;
        let mut submit = false;
        let mut copy: Option<String> = None;
        egui::Area::new(egui::Id::unique("create"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                card().show(ui, |ui| {
                    ui.set_width(520.0);
                    ui.label(
                        RichText::new(format!("> {}", t.create_chat))
                            .heading()
                            .color(GREEN),
                    );
                    ui.add_space(10.0);

                    ui.label(t.nickname);
                    let r = ui.add(
                        TextEdit::singleline(&mut f.nick)
                            .hint_text("Zero Cool")
                            .desired_width(f32::INFINITY),
                    );
                    if f.nick.is_empty() && !r.has_focus() && f.error.is_none() {
                        r.request_focus();
                    }
                    nick_hint(ui, &f.nick, t);
                    ui.add_space(8.0);

                    ui.label(t.password);
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut f.own, false, t.pw_auto);
                        ui.radio_value(&mut f.own, true, t.pw_own);
                    });
                    if !f.own {
                        ui.horizontal(|ui| {
                            Frame::new()
                                .fill(Color32::from_rgb(0, 14, 4))
                                .stroke(Stroke::new(1.0, DIM))
                                .inner_margin(Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(f.suggestion.as_str())
                                            .monospace()
                                            .size(16.0)
                                            .color(GREEN),
                                    );
                                });
                            if ui.button(format!("⟳ {}", t.suggest)).clicked() {
                                f.suggestion = password::generate(password::DEFAULT_LENGTH);
                            }
                            if ui.button(t.copy).clicked() {
                                copy = Some(f.suggestion.to_string());
                            }
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} · ~{:.0} bit",
                                password::DEFAULT_LENGTH,
                                password::generated_entropy_bits(password::DEFAULT_LENGTH)
                            ))
                            .small()
                            .color(DIM),
                        );
                    } else {
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut *f.pw)
                                    .password(!f.show)
                                    .desired_width(330.0),
                            );
                            ui.checkbox(&mut f.show, t.show);
                            if ui.button(format!("⟳ {}", t.suggest)).clicked() {
                                let p = password::generate(password::DEFAULT_LENGTH);
                                *f.pw = p.to_string();
                                *f.pw2 = p.to_string();
                                f.show = true;
                            }
                        });
                        ui.label(t.repeat);
                        ui.add(
                            TextEdit::singleline(&mut *f.pw2)
                                .password(!f.show)
                                .desired_width(330.0),
                        );
                        if !f.pw.is_empty() && f.pw.chars().count() < password::MIN_USER_LENGTH {
                            ui.label(RichText::new(t.pw_short).small().color(AMBER));
                        } else if !f.pw2.is_empty() && *f.pw != *f.pw2 {
                            ui.label(RichText::new(t.pw_mismatch).small().color(AMBER));
                        }
                    }

                    if let Some(e) = &f.error {
                        ui.add_space(6.0);
                        ui.label(RichText::new(format!("✖ {e}")).color(RED));
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui.button(t.back).clicked() {
                            go_back = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let enter = ui.input(|i| i.key_pressed(Key::Enter));
                            if ui
                                .add(egui::Button::new(
                                    RichText::new(format!("{} ▸", t.create_btn))
                                        .strong()
                                        .color(GREEN),
                                ))
                                .clicked()
                                || enter
                            {
                                submit = true;
                            }
                        });
                    });
                });
            });
        if let Some(c) = copy {
            ctx.copy_text(c);
        }
        if go_back {
            return Screen::Home;
        }
        if !submit {
            return Screen::Create(f);
        }
        // Validate.
        let nick = match Nickname::new(&f.nick) {
            Ok(n) => n,
            Err(e) => {
                f.error = Some(format!("{}: {e}", t.nickname));
                return Screen::Create(f);
            }
        };
        let pw: Zeroizing<String> = if f.own {
            if let Err(e) = password::validate_user_password(&f.pw) {
                f.error = Some(e.to_string());
                return Screen::Create(f);
            }
            if *f.pw != *f.pw2 {
                f.error = Some(t.pw_mismatch.into());
                return Screen::Create(f);
            }
            f.pw.clone()
        } else {
            f.suggestion.clone()
        };
        if let Err(e) = self.net.route() {
            f.error = Some(e.to_string());
            return Screen::Create(f);
        }
        let (tx, rx) = mpsc::channel();
        let (h, cfg, c) = (self.rt.handle().clone(), self.net.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = setup::create_room(&h, &cfg, Some(pw.as_str()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        Screen::Working(Working {
            rx,
            nick,
            back: Some(Back::Create(f)),
            created: true,
        })
    }

    // ───────────────────────────── Join ─────────────────────────────

    fn join(&mut self, ctx: &egui::Context, mut f: JoinForm) -> Screen {
        let t = self.t();
        let mut go_back = false;
        let mut submit = false;
        egui::Area::new(egui::Id::unique("join"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                card().show(ui, |ui| {
                    ui.set_width(520.0);
                    ui.label(
                        RichText::new(format!("> {}", t.join_room))
                            .heading()
                            .color(GREEN),
                    );
                    ui.add_space(10.0);

                    ui.label(t.room_id);
                    let r = ui.add(
                        TextEdit::singleline(&mut f.id)
                            .hint_text("1234 5678 9012 3456 78")
                            .font(egui::TextStyle::Heading)
                            .desired_width(f32::INFINITY),
                    );
                    if f.id.is_empty() && !r.has_focus() && f.error.is_none() {
                        r.request_focus();
                    }
                    let digits = f.id.chars().filter(|c| c.is_ascii_digit()).count();
                    if digits >= 16 {
                        match RoomId::parse(&f.id) {
                            Ok(_) => ui.label(
                                RichText::new(format!("✔ {}", t.id_ok)).small().color(GREEN),
                            ),
                            Err(e) => ui.label(
                                RichText::new(format!("✖ {} ({e})", t.id_bad))
                                    .small()
                                    .color(AMBER),
                            ),
                        };
                    }
                    ui.add_space(8.0);

                    ui.label(t.password);
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut *f.pw)
                                .password(!f.show)
                                .desired_width(380.0),
                        );
                        ui.checkbox(&mut f.show, t.show);
                    });
                    ui.add_space(8.0);

                    ui.label(t.nickname);
                    ui.add(
                        TextEdit::singleline(&mut f.nick)
                            .hint_text("Acid Burn")
                            .desired_width(f32::INFINITY),
                    );
                    nick_hint(ui, &f.nick, t);

                    if let Some(e) = &f.error {
                        ui.add_space(6.0);
                        ui.label(RichText::new(format!("✖ {e}")).color(RED));
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui.button(t.back).clicked() {
                            go_back = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let enter = ui.input(|i| i.key_pressed(Key::Enter));
                            if ui
                                .add(egui::Button::new(
                                    RichText::new(t.join_btn).strong().color(GREEN),
                                ))
                                .clicked()
                                || enter
                            {
                                submit = true;
                            }
                        });
                    });
                });
            });
        if go_back {
            return Screen::Home;
        }
        if !submit {
            return Screen::Join(f);
        }
        let id = match RoomId::parse(&f.id) {
            Ok(id) => id,
            Err(e) => {
                f.error = Some(format!("{}: {e}", t.room_id));
                return Screen::Join(f);
            }
        };
        if f.pw.is_empty() {
            f.error = Some(format!("{}?", t.password));
            return Screen::Join(f);
        }
        let nick = match Nickname::new(&f.nick) {
            Ok(n) => n,
            Err(e) => {
                f.error = Some(format!("{}: {e}", t.nickname));
                return Screen::Join(f);
            }
        };
        if let Err(e) = self.net.route() {
            f.error = Some(e.to_string());
            return Screen::Join(f);
        }
        let (tx, rx) = mpsc::channel();
        let (h, cfg, c, pw) = (
            self.rt.handle().clone(),
            self.net.clone(),
            ctx.clone(),
            f.pw.clone(),
        );
        std::thread::spawn(move || {
            let r = setup::join_room(&h, &cfg, id, pw);
            let _ = tx.send(r);
            c.request_repaint();
        });
        Screen::Working(Working {
            rx,
            nick,
            back: Some(Back::Join(f)),
            created: false,
        })
    }

    // ──────────────────────────── Working ───────────────────────────

    fn working(&mut self, ctx: &egui::Context, mut w: Working) -> Screen {
        let t = self.t();
        match w.rx.try_recv() {
            Ok(Ok(prepared)) => {
                return if w.created {
                    Screen::Created(Some(prepared), w.nick)
                } else {
                    self.enter(ctx, prepared, w.nick)
                };
            }
            Ok(Err(e)) => {
                let msg = format!("{e:#}");
                return match w.back.take() {
                    Some(Back::Create(mut f)) => {
                        f.error = Some(msg);
                        Screen::Create(f)
                    }
                    Some(Back::Join(mut f)) => {
                        f.error = Some(msg);
                        Screen::Join(f)
                    }
                    None => Screen::Home,
                };
            }
            Err(_) => {}
        }
        egui::Area::new(egui::Id::unique("working"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(t.working).color(GREEN));
                    });
                    ui.label(
                        RichText::new(format!("via {}", self.route_label()))
                            .small()
                            .color(DIM),
                    );
                });
            });
        Screen::Working(w)
    }

    fn route_label(&self) -> String {
        self.net
            .route()
            .map(|r| r.label())
            .unwrap_or_else(|e| e.to_string())
    }

    fn enter(&mut self, ctx: &egui::Context, prepared: Prepared, nick: Nickname) -> Screen {
        let c = ctx.clone();
        match Room::enter(
            self.rt.handle(),
            prepared,
            nick,
            self.read_receipts,
            Arc::new(move || c.request_repaint()),
        ) {
            Ok(room) => {
                let t = self.t();
                let me = room.me();
                let entries = vec![
                    Entry::Sys(
                        format!("> {} · {}", room.id.display_grouped(), room.route_label),
                        MID,
                    ),
                    Entry::Sys(format!("> {}", t.welcome), MID),
                    Entry::Sys(format!("> {}", t.fingerprint_hint), DIM),
                ];
                Screen::Chat(Box::new(Chat {
                    room,
                    me,
                    entries,
                    statuses: HashMap::new(),
                    input: String::new(),
                    show_members: false,
                    show_info: false,
                    conn_err: false,
                    burn_hold: None,
                    ending: None,
                    leaving: None,
                    burning: None,
                    stick_bottom: true,
                }))
            }
            Err(e) => Screen::Join(JoinForm {
                id: String::new(),
                pw: Zeroizing::new(String::new()),
                nick: String::new(),
                show: false,
                error: Some(e.to_string()),
            }),
        }
    }

    // ──────────────────────────── Created ───────────────────────────

    fn created(&mut self, ctx: &egui::Context, mut p: Option<Prepared>, nick: Nickname) -> Screen {
        let t = self.t();
        let (id, pw) = match &p {
            Some(p) => (p.id.display_grouped(), p.password.to_string()),
            None => return Screen::Home,
        };
        let pw = Zeroizing::new(pw);
        let mut go = false;
        egui::Area::new(egui::Id::unique("created"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                card().show(ui, |ui| {
                    ui.set_width(560.0);
                    ui.label(
                        RichText::new(format!("✔ {}", t.room_created))
                            .heading()
                            .color(GREEN),
                    );
                    ui.add_space(12.0);
                    egui::Grid::new("creds")
                        .num_columns(3)
                        .spacing([12.0, 12.0])
                        .show(ui, |ui| {
                            ui.label(t.room_id);
                            ui.label(
                                RichText::new(&id)
                                    .monospace()
                                    .size(20.0)
                                    .strong()
                                    .color(GREEN),
                            );
                            self.copy_button(ui, "id", &id);
                            ui.end_row();
                            ui.label(t.password);
                            ui.label(
                                RichText::new(pw.as_str())
                                    .monospace()
                                    .size(18.0)
                                    .color(GREEN),
                            );
                            self.copy_button(ui, "pw", &pw);
                            ui.end_row();
                        });
                    ui.add_space(10.0);
                    ui.label(RichText::new(format!("⚠ {}", t.share_hint)).color(AMBER));
                    ui.label(RichText::new(t.clipboard_hint).small().color(DIM));
                    ui.add_space(14.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(
                                RichText::new(t.enter_room).strong().color(GREEN),
                            ))
                            .clicked()
                        {
                            go = true;
                        }
                    });
                });
            });
        if go {
            let prepared = p.take().expect("checked above");
            return self.enter(ctx, prepared, nick);
        }
        Screen::Created(p, nick)
    }

    // ───────────────────────────── Ended ────────────────────────────

    fn ended(&mut self, ctx: &egui::Context, e: End) -> Screen {
        let t = self.t();
        let (msg, col) = match &e {
            End::Left => (t.you_left.to_string(), GREEN),
            End::BurnedByMe => (format!("🔥 {}", t.you_burned), RED),
            End::BurnedBy(by) => (format!("🔥 {} {by}. {}", t.burned_by, t.burned), RED),
            End::Vanished => (format!("🔥 {}", t.burned), RED),
        };
        let mut home = false;
        egui::Area::new(egui::Id::unique("ended"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                card().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(msg).size(17.0).color(col));
                        ui.add_space(14.0);
                        if ui.button(t.to_start).clicked() {
                            home = true;
                        }
                    });
                });
            });
        if home {
            Screen::Home
        } else {
            Screen::Ended(e)
        }
    }

    // ───────────────────────────── Chat ─────────────────────────────

    fn chat(&mut self, ui: &mut egui::Ui, mut c: Box<Chat>) -> Screen {
        let t = self.t();
        let ctx = ui.ctx().clone();

        // Network / protocol events.
        for ev in c.room.poll() {
            match ev {
                RoomEvent::Core(e) => apply_event(&mut c, e, t),
                RoomEvent::ConnError(_) => c.conn_err = true,
                RoomEvent::Online => c.conn_err = false,
                RoomEvent::LeftDone => return self.after_chat(End::Left),
                RoomEvent::BurnDone => return self.after_chat(End::BurnedByMe),
                RoomEvent::Vanished => {
                    if c.ending.is_none() {
                        c.entries
                            .push(Entry::Sys(format!("> 🔥 {}", t.burned), RED));
                        c.ending = Some((Instant::now(), End::Vanished));
                    }
                }
            }
        }
        // Timeouts.
        if let Some((at, e)) = &c.ending {
            if at.elapsed() > Duration::from_millis(1800) {
                return self.after_chat(e.clone());
            }
        }
        if c.leaving
            .is_some_and(|t| t.elapsed() > Duration::from_secs(4))
        {
            return self.after_chat(End::Left);
        }
        if c.burning
            .is_some_and(|t| t.elapsed() > Duration::from_secs(8))
        {
            return self.after_chat(End::BurnedByMe);
        }
        // Window close → leave politely first.
        if self.quit_requested && c.leaving.is_none() && c.burning.is_none() {
            c.room.leave();
            c.leaving = Some(Instant::now());
        }
        // Read receipts while the window has focus.
        if ctx.input(|i| i.focused) {
            c.room.mark_read();
        }
        let busy = c.leaving.is_some() || c.burning.is_some() || c.ending.is_some();
        let members = c.room.members();

        // ── top bar
        egui::Panel::top("top")
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("GHOSTLINE").strong().size(16.0).color(GREEN));
                    ui.separator();
                    ui.label(
                        RichText::new(c.room.id.display_grouped())
                            .size(16.0)
                            .color(GREEN),
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Hold-to-burn button.
                        let progress = c
                            .burn_hold
                            .map(|s| (s.elapsed().as_secs_f32() / BURN_HOLD.as_secs_f32()).min(1.0))
                            .unwrap_or(0.0);
                        let btn = egui::Button::new(
                            RichText::new(format!("🔥 {}", t.burn)).strong().color(RED),
                        )
                        .stroke(Stroke::new(1.5, RED))
                        .fill(Color32::from_rgb(40, 0, 4))
                        .sense(Sense::click_and_drag());
                        let r = ui.add_enabled(!busy, btn).on_hover_text(t.burn_hold);
                        if progress > 0.0 {
                            let mut fill = r.rect;
                            fill.set_width(r.rect.width() * progress);
                            ui.painter().rect_filled(
                                fill,
                                CornerRadius::same(3),
                                Color32::from_rgba_unmultiplied(255, 40, 50, 110),
                            );
                        }
                        if r.is_pointer_button_down_on() {
                            let start = *c.burn_hold.get_or_insert_with(Instant::now);
                            if start.elapsed() >= BURN_HOLD {
                                c.burn_hold = None;
                                c.room.burn();
                                c.burning = Some(Instant::now());
                                c.entries.push(Entry::Sys(format!("> 🔥 {}…", t.burn), RED));
                            }
                        } else {
                            c.burn_hold = None;
                        }

                        if ui.add_enabled(!busy, egui::Button::new(t.leave)).clicked() {
                            c.room.leave();
                            c.leaving = Some(Instant::now());
                            c.entries.push(Entry::Sys(format!("> {}", t.leaving), MID));
                        }
                        ui.toggle_value(&mut c.show_info, t.info);
                        ui.toggle_value(&mut c.show_members, format!("👥 {}", t.members));
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&c.me).small().color(MID));
                    ui.label(RichText::new("·").small().color(FAINT));
                    ui.label(
                        RichText::new(format!("{} {}", members.len() + 1, t.online))
                            .small()
                            .color(MID),
                    );
                    ui.label(RichText::new("·").small().color(FAINT));
                    if c.conn_err {
                        ui.label(RichText::new(t.reconnecting).small().color(AMBER));
                    } else {
                        ui.label(RichText::new(&c.room.route_label).small().color(DIM));
                    }
                    ui.label(RichText::new("·").small().color(FAINT));
                    ui.label(RichText::new("🔒 E2E").small().color(GREEN));
                    ui.label(RichText::new("·").small().color(FAINT));
                    if self.capture_blocked {
                        ui.label(
                            RichText::new(format!("🛡 {}", t.capture_on))
                                .small()
                                .color(DIM),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!("⚠ {}", t.capture_off))
                                .small()
                                .color(AMBER),
                        );
                    }
                });
            });

        // ── composer
        let mut send = false;
        egui::Panel::bottom("composer")
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 110.0;
                    let r = ui.add_enabled(
                        !busy,
                        TextEdit::multiline(&mut c.input)
                            .desired_rows(1)
                            .desired_width(w)
                            .hint_text(t.type_msg)
                            .char_limit(ghostline_core::session::MAX_TEXT_CHARS),
                    );
                    if !busy && !r.has_focus() && !c.show_info {
                        r.request_focus();
                    }
                    if r.has_focus()
                        && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift)
                    {
                        send = true;
                    }
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new(format!("{} ▸", t.send)).strong().color(GREEN),
                            )
                            .min_size(Vec2::new(96.0, 32.0)),
                        )
                        .clicked()
                    {
                        send = true;
                    }
                });
            });
        if send {
            let text = c.input.trim_end_matches(['\n', '\r']).trim().to_string();
            c.input.clear();
            if !text.is_empty() {
                match c.room.send_text(&text) {
                    Ok(id) => {
                        c.statuses.insert(id, Status::Pending);
                        c.entries.push(Entry::Msg {
                            id: Some(id),
                            ts: now_secs(),
                            from: c.me.clone(),
                            text,
                            mine: true,
                        });
                        c.stick_bottom = true;
                    }
                    Err(e) => c.entries.push(Entry::Sys(format!("✖ {e}"), RED)),
                }
            }
        }

        // ── members panel
        if c.show_members {
            egui::Panel::right("members")
                .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(12)))
                .default_size(220.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("👥 {}", t.members))
                            .strong()
                            .color(GREEN),
                    );
                    ui.separator();
                    ui.label(RichText::new(format!("● {} ({})", c.me, t.you)).color(GREEN));
                    if members.is_empty() {
                        ui.label(RichText::new(t.nobody).small().color(DIM));
                    }
                    for m in &members {
                        ui.label(RichText::new(format!("● {m}")).color(MID));
                    }
                });
        }

        // ── messages
        egui::CentralPanel::no_frame().show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    let maxw = (ui.available_width() * 0.7).max(240.0);
                    for e in &c.entries {
                        match e {
                            Entry::Sys(s, col) => {
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new(s).small().color(*col));
                                });
                            }
                            Entry::Msg {
                                id,
                                ts,
                                from,
                                text,
                                mine,
                            } => {
                                let status = id.and_then(|i| c.statuses.get(&i).copied());
                                bubble(ui, maxw, *mine, from, text, *ts, status, t);
                            }
                        }
                    }
                    ui.add_space(8.0);
                });
        });

        // ── info window
        if c.show_info {
            let id = c.room.id.display_grouped();
            let pw = Zeroizing::new(c.room.password.to_string());
            let mut open = true;
            egui::Window::new(t.info)
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    egui::Grid::new("info")
                        .num_columns(3)
                        .spacing([12.0, 10.0])
                        .show(ui, |ui| {
                            ui.label(t.room_id);
                            ui.label(RichText::new(&id).monospace().size(18.0).color(GREEN));
                            self.copy_button(ui, "id", &id);
                            ui.end_row();
                            ui.label(t.password);
                            ui.label(
                                RichText::new(pw.as_str())
                                    .monospace()
                                    .size(16.0)
                                    .color(GREEN),
                            );
                            self.copy_button(ui, "pw", &pw);
                            ui.end_row();
                        });
                    ui.label(
                        RichText::new(format!("⚠ {}", t.share_hint))
                            .small()
                            .color(AMBER),
                    );
                });
            if !open {
                c.show_info = false;
            }
        }

        Screen::Chat(c)
    }

    fn after_chat(&mut self, e: End) -> Screen {
        if self.quit_requested {
            Screen::Home // window will close
        } else {
            Screen::Ended(e)
        }
    }
}

fn nick_hint(ui: &mut egui::Ui, nick: &str, t: &T) {
    if nick.is_empty() {
        ui.label(RichText::new(t.nick_rule).small().color(DIM));
    } else if let Err(e) = Nickname::new(nick) {
        ui.label(RichText::new(format!("✖ {e}")).small().color(AMBER));
    } else {
        ui.label(RichText::new("✔").small().color(GREEN));
    }
}

fn apply_event(c: &mut Chat, e: Event, t: &T) {
    match e {
        Event::Joined { who } => c
            .entries
            .push(Entry::Sys(format!("> {who} {}", t.joined), MID)),
        Event::Left { who } => c
            .entries
            .push(Entry::Sys(format!("> {who} {}", t.left), DIM)),
        Event::Message { from, text, ts, .. } => c.entries.push(Entry::Msg {
            id: None,
            ts,
            from,
            text,
            mine: false,
        }),
        Event::Status { id, status } => {
            c.statuses.insert(id, status);
        }
        Event::Burned { by } => {
            if c.ending.is_none() {
                c.entries
                    .push(Entry::Sys(format!("> 🔥 {} {by}", t.burned_by), RED));
                c.ending = Some((Instant::now(), End::BurnedBy(by)));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn bubble(
    ui: &mut egui::Ui,
    maxw: f32,
    mine: bool,
    from: &str,
    text: &str,
    ts: u64,
    status: Option<Status>,
    t: &T,
) {
    let layout = if mine {
        Layout::right_to_left(Align::TOP)
    } else {
        Layout::left_to_right(Align::TOP)
    };
    ui.with_layout(layout, |ui| {
        ui.add_space(12.0);
        Frame::new()
            .fill(if mine { BUBBLE_ME } else { BUBBLE_THEM })
            .stroke(Stroke::new(1.0, if mine { MID } else { FAINT }))
            .corner_radius(CornerRadius {
                nw: 10,
                ne: 10,
                sw: if mine { 10 } else { 2 },
                se: if mine { 2 } else { 10 },
            })
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                // Size the bubble to its content (up to maxw).
                let measure = |s: &str, size: f32| {
                    ui.painter()
                        .layout_no_wrap(s.to_owned(), egui::FontId::monospace(size), GREEN)
                        .size()
                        .x
                };
                let content_w = text
                    .lines()
                    .map(|l| measure(l, 14.0))
                    .fold(0.0f32, f32::max)
                    .max(if mine { 0.0 } else { measure(from, 11.0) })
                    .max(80.0);
                ui.set_width(content_w.min(maxw) + 2.0);
                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                    if !mine {
                        ui.label(RichText::new(from).small().strong().color(GREEN));
                    }
                    ui.add(
                        egui::Label::new(RichText::new(text).color(if mine { GREEN } else { MID }))
                            .wrap()
                            .selectable(true),
                    );
                    ui.horizontal(|ui| {
                        let time = Local
                            .timestamp_opt(ts as i64, 0)
                            .single()
                            .map(|d| d.format("%H:%M").to_string())
                            .unwrap_or_default();
                        ui.label(RichText::new(time).small().color(DIM));
                        if mine {
                            let (tick, col) = match status {
                                None | Some(Status::Pending) => ("⏳", DIM),
                                Some(Status::Sent) => ("✔", DIM),
                                Some(Status::Delivered) => ("✔✔", MID),
                                Some(Status::Read) => ("✔✔", CYAN),
                            };
                            ui.label(RichText::new(tick).small().strong().color(col));
                        } else {
                            let _ = t;
                        }
                    });
                });
            });
    });
    ui.add_space(2.0);
}

impl eframe::App for GhostApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Close requested while in a room → leave first, close afterwards.
        if ctx.input(|i| i.viewport().close_requested())
            && matches!(self.screen, Screen::Chat(_))
            && !self.quit_requested
        {
            self.quit_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }

        // Background + matrix rain.
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(rect, 0.0, BG);
        let in_chat = matches!(self.screen, Screen::Chat(_));
        self.rain
            .paint(&painter, rect, dt, if in_chat { 0.16 } else { 0.6 });
        matrix::schedule(&ctx);

        let screen = std::mem::replace(&mut self.screen, Screen::Home);
        self.screen = match screen {
            Screen::Home => {
                self.home(&ctx);
                std::mem::replace(&mut self.screen, Screen::Home)
            }
            Screen::Create(f) => self.create(&ctx, f),
            Screen::Join(f) => self.join(&ctx, f),
            Screen::Working(w) => self.working(&ctx, w),
            Screen::Created(p, n) => self.created(&ctx, p, n),
            Screen::Chat(c) => self.chat(ui, c),
            Screen::Ended(e) => self.ended(&ctx, e),
        };

        if self.quit_requested && !matches!(self.screen, Screen::Chat(_)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
