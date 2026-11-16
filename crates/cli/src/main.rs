//! `ghostline` — terminal client.
//!
//! ```text
//! ghostline create                     # new room, prints ID + password
//! ghostline join 2404 9760 5034 …      # join (asks for password + nickname)
//! ghostline suggest-password           # just print a strong password
//! ```

mod ui;

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use ghostline_client::{setup, NetConfig, Prepared, Room, TorMode};
use ghostline_core::{password, Nickname, RoomId};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    name = "ghostline",
    version,
    about = "GhostLine — anonymous end-to-end encrypted chat over Tor. No accounts, no logs."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new room and enter it
    Create {
        /// Your nickname in this room (asked if omitted)
        #[arg(long, short)]
        nick: Option<String>,
        #[command(flatten)]
        net: NetArgs,
    },
    /// Join an existing room (asks for the password)
    Join {
        /// Room ID — spaces are fine: `ghostline join 2404 9760 5034 5356 4522 6`
        #[arg(required = true, num_args = 1..)]
        id: Vec<String>,
        /// Your nickname in this room (asked if omitted)
        #[arg(long, short)]
        nick: Option<String>,
        #[command(flatten)]
        net: NetArgs,
    },
    /// Print a strong random password ("Şifre öner")
    SuggestPassword {
        /// Length (minimum 16)
        #[arg(default_value_t = password::DEFAULT_LENGTH)]
        length: usize,
    },
}

#[derive(Args)]
struct NetArgs {
    /// Relay URL (an .onion address in real use)
    #[arg(long, env = "GHOSTLINE_RELAY", default_value = "http://127.0.0.1:7878")]
    relay: String,
    /// Use an already-running Tor instead of the built-in one (SOCKS on 127.0.0.1:<tor-port>)
    #[arg(long)]
    external_tor: bool,
    /// SOCKS port of the external Tor: 9050 = tor service, 9150 = Tor Browser
    #[arg(long, default_value_t = 9050)]
    tor_port: u16,
    /// Custom SOCKS proxy, e.g. socks5h://127.0.0.1:9150 (implies external Tor)
    #[arg(long)]
    proxy: Option<String>,
    /// No Tor at all; allow a direct connection to a non-local relay. Exposes your IP!
    #[arg(long)]
    no_tor: bool,
    /// Do not send read receipts (others will never see the cyan ✓✓)
    #[arg(long)]
    no_read_receipts: bool,
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut s = String::new();
    io::stdin().lock().read_line(&mut s)?;
    Ok(s.trim_end_matches(['\r', '\n']).to_string())
}

fn ask_nick(given: Option<String>) -> Result<Nickname> {
    let mut candidate = given;
    loop {
        let raw = match candidate.take() {
            Some(n) => n,
            None => prompt("nickname: ")?,
        };
        match Nickname::new(&raw) {
            Ok(n) => return Ok(n),
            Err(e) => eprintln!("  ✗ {e}"),
        }
    }
}

fn green(s: &str) -> String {
    format!("\x1b[92m{s}\x1b[0m")
}

fn enable_ansi() {
    // On Windows, crossterm switches the console into VT (ANSI) mode the first
    // time it executes a command; after that our plain ANSI colours work too.
    let _ = ratatui::crossterm::execute!(io::stdout(), ratatui::crossterm::style::ResetColor);
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?)
}

impl NetArgs {
    fn config(&self) -> NetConfig {
        let tor = if self.no_tor {
            TorMode::Off
        } else if let Some(p) = &self.proxy {
            TorMode::External(p.clone())
        } else if self.external_tor {
            TorMode::External(format!("socks5h://127.0.0.1:{}", self.tor_port))
        } else {
            TorMode::Embedded
        };
        NetConfig {
            relay: self.relay.clone(),
            tor,
            allow_direct: self.no_tor,
            embedded: None,
        }
    }
}

/// Start the built-in Tor if this config needs it, with a progress line.
fn ensure_tor(rt: &tokio::runtime::Runtime, cfg: &mut NetConfig) -> Result<()> {
    if !cfg.needs_embedded_tor() {
        return Ok(());
    }
    let last = Arc::new(std::sync::Mutex::new(-1i32));
    let progress: ghostline_client::tor::Progress = Arc::new(move |f: f32, _msg: String| {
        let pct = (f * 100.0) as i32;
        let mut l = last.lock().unwrap();
        if pct != *l {
            *l = pct;
            print!("\r\x1b[92m> connecting to Tor (built-in)… {pct:3}%\x1b[0m");
            let _ = io::stdout().flush();
        }
    });
    let tor = setup::start_tor(rt.handle(), progress)?;
    println!();
    cfg.embedded = Some(tor);
    Ok(())
}

/// Validate the network settings before asking for passwords.
fn precheck(cfg: &NetConfig) -> Result<()> {
    if cfg.needs_embedded_tor() {
        return Ok(()); // Tor will be started later
    }
    cfg.route().map(|_| ())
}

fn working(cfg: &NetConfig) -> Result<()> {
    let route = cfg.route()?;
    println!("{}", green("> deriving keys (argon2id, 128 MiB)…"));
    println!(
        "{}",
        green(&format!("> contacting relay via {}…", route.label()))
    );
    Ok(())
}

fn create(nick: Option<String>, n: NetArgs) -> Result<()> {
    let mut cfg = n.config();
    precheck(&cfg)?;
    let nick = ask_nick(nick)?;
    let pw = Zeroizing::new(rpassword::prompt_password(
        "password (Enter = generate a strong one, or type ≥12 chars): ",
    )?);
    let user_pw = if pw.is_empty() {
        None
    } else {
        password::validate_user_password(&pw)?;
        let again = Zeroizing::new(rpassword::prompt_password("repeat password: ")?);
        if *again != *pw {
            bail!("passwords do not match");
        }
        Some(pw.as_str())
    };

    let rt = runtime()?;
    ensure_tor(&rt, &mut cfg)?;
    working(&cfg)?;
    let prepared = setup::create_room(rt.handle(), &cfg, user_pw)?;

    println!();
    println!(
        "{}",
        green("╔══════════════════════════════════════════════════════════╗")
    );
    println!(
        "{}",
        green("║  ROOM CREATED — share these two over a trusted channel   ║")
    );
    println!(
        "{}",
        green("╚══════════════════════════════════════════════════════════╝")
    );
    println!("  room id : {}", green(&prepared.id.display_grouped()));
    println!("  password: {}", green(prepared.password.as_str()));
    println!();
    println!(
        "  join with:  ghostline join {}",
        prepared.id.display_grouped()
    );
    println!();
    prompt(&green("press Enter to open the room… "))?;

    enter(rt, prepared, nick, &n)
}

fn join(id: Vec<String>, nick: Option<String>, n: NetArgs) -> Result<()> {
    let mut cfg = n.config();
    precheck(&cfg)?;
    let id = RoomId::parse(&id.join(" ")).map_err(|e| anyhow::anyhow!("room id: {e}"))?;
    let pw = Zeroizing::new(rpassword::prompt_password("password: ")?);
    let nick = ask_nick(nick)?;
    let rt = runtime()?;
    ensure_tor(&rt, &mut cfg)?;
    working(&cfg)?;
    let prepared = setup::join_room(rt.handle(), &cfg, id, pw)?;
    enter(rt, prepared, nick, &n)
}

fn enter(
    rt: tokio::runtime::Runtime,
    prepared: Prepared,
    nick: Nickname,
    n: &NetArgs,
) -> Result<()> {
    let room = Room::enter(
        rt.handle(),
        prepared,
        nick,
        !n.no_read_receipts,
        Arc::new(|| {}),
    )?;
    let exit = ui::run(room)?;
    rt.shutdown_timeout(Duration::from_millis(500));
    match exit {
        ui::Exit::Quit => println!("{}", green("> left the room. keys wiped from memory.")),
        ui::Exit::BurnedByMe => println!(
            "{}",
            green("> 🔥 room burned. relay copy deleted, keys wiped.")
        ),
        ui::Exit::BurnedBy(by) => println!(
            "{}",
            green(&format!("> 🔥 room burned by {by}. keys wiped."))
        ),
        ui::Exit::Vanished => println!(
            "{}",
            green("> 🔥 room burned (gone from relay). keys wiped.")
        ),
    }
    Ok(())
}

fn main() {
    enable_ansi();
    let cli = Cli::parse();
    let r = match cli.cmd {
        Cmd::Create { nick, net } => create(nick, net),
        Cmd::Join { id, nick, net } => join(id, nick, net),
        Cmd::SuggestPassword { length } => {
            let p = password::generate(length);
            println!("{}", p.as_str());
            eprintln!(
                "({} chars, ~{:.0} bits)",
                p.chars().count(),
                password::generated_entropy_bits(p.chars().count())
            );
            Ok(())
        }
    };
    if let Err(e) = r {
        eprintln!("\x1b[91merror:\x1b[0m {e:#}");
        std::process::exit(1);
    }
}
