//! `ghostline` — terminal client.
//!
//! ```text
//! ghostline create                     # new room, prints ID + password
//! ghostline join 2404 9760 5034 …      # join (asks for password + nickname)
//! ghostline suggest-password           # just print a strong password
//! ```

mod net;
mod ui;

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ghostline_core::{password, KdfParams, Nickname, RoomCredentials, RoomId, Session};
use zeroize::Zeroizing;

use net::{Fetch, Relay, Route};

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
    /// Route through Tor (SOCKS on 127.0.0.1:<tor-port>). Automatic for .onion relays.
    #[arg(long)]
    tor: bool,
    /// Tor SOCKS port: 9050 = tor service, 9150 = Tor Browser
    #[arg(long, default_value_t = 9050)]
    tor_port: u16,
    /// Custom SOCKS proxy, e.g. socks5h://127.0.0.1:9150
    #[arg(long)]
    proxy: Option<String>,
    /// Allow a direct (non-Tor) connection to a non-local relay. Exposes your IP!
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

fn derive(creds: &RoomCredentials) -> Result<ghostline_core::RoomSecrets> {
    print!("{}", green("> deriving keys (argon2id, 128 MiB)… "));
    io::stdout().flush()?;
    let s = creds.derive(&KdfParams::DESKTOP)?;
    println!("{}", green("ok"));
    Ok(s)
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?)
}

fn route_for(n: &NetArgs) -> Result<Route> {
    net::choose_route(&n.relay, n.tor, n.tor_port, n.proxy.as_deref(), n.no_tor)
}

fn create(nick: Option<String>, n: NetArgs) -> Result<()> {
    let route = route_for(&n)?;
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
    let (creds, secrets, relay) = loop {
        let creds = RoomCredentials::new_room(user_pw)?;
        let secrets = derive(&creds)?;
        let relay = Relay::new(&n.relay, &route, &secrets.mailbox_id)?;
        println!(
            "{}",
            green(&format!("> contacting relay via {}…", route.label()))
        );
        let created = rt
            .block_on(relay.create(&secrets.delete_token_hash()))
            .context("could not reach the relay — is ghostline-relay running? (or Tor?)")?;
        if created {
            break (creds, secrets, relay);
        }
    };

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
    println!("  room id : {}", green(&creds.id.display_grouped()));
    println!("  password: {}", green(creds.password.as_str()));
    println!();
    println!(
        "  join with:  ghostline join {}",
        creds.id.display_grouped()
    );
    println!();
    prompt(&green("press Enter to open the room… "))?;

    enter(
        rt,
        relay,
        creds.id.clone(),
        creds.password.clone(),
        secrets,
        nick,
        route,
        &n,
    )
}

fn join(id: Vec<String>, nick: Option<String>, n: NetArgs) -> Result<()> {
    let route = route_for(&n)?;
    let id = RoomId::parse(&id.join(" ")).map_err(|e| anyhow::anyhow!("room id: {e}"))?;
    let pw = Zeroizing::new(rpassword::prompt_password("password: ")?);
    let nick = ask_nick(nick)?;
    let creds = RoomCredentials {
        id: id.clone(),
        password: pw,
    };
    let secrets = derive(&creds)?;
    let rt = runtime()?;
    let relay = Relay::new(&n.relay, &route, &secrets.mailbox_id)?;
    println!(
        "{}",
        green(&format!("> contacting relay via {}…", route.label()))
    );
    match rt
        .block_on(relay.fetch(u64::MAX))
        .context("could not reach the relay — is it running? (or Tor?)")?
    {
        Fetch::Blobs(_) => {}
        Fetch::NotFound => {
            bail!("room not found — or wrong password (these look identical on purpose)")
        }
        Fetch::Gone => bail!("this room has been burned"),
    }
    enter(
        rt,
        relay,
        id,
        creds.password.clone(),
        secrets,
        nick,
        route,
        &n,
    )
}

#[allow(clippy::too_many_arguments)]
fn enter(
    rt: tokio::runtime::Runtime,
    relay: Relay,
    id: RoomId,
    password: Zeroizing<String>,
    secrets: ghostline_core::RoomSecrets,
    nick: Nickname,
    route: Route,
    n: &NetArgs,
) -> Result<()> {
    let session = Session::new(nick, &secrets);
    let relay = Arc::new(relay);
    rt.block_on(relay.post(session.hello_blob()?))?;

    let info = ui::RoomInfo {
        id,
        password,
        route_label: route.label(),
        delete_token: Zeroizing::new(secrets.delete_token),
        poll_interval: match route {
            Route::Direct => Duration::from_millis(700),
            Route::Socks(_) => Duration::from_millis(1500),
        },
        read_receipts: !n.no_read_receipts,
    };
    drop(secrets); // zeroized on drop

    let exit = ui::run(&rt, relay, session, info)?;
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
