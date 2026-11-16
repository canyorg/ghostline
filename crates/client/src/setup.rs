//! Creating and joining rooms. These functions block (Argon2id + network) —
//! call them from a worker thread in GUIs.

use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use ghostline_core::{KdfParams, RoomCredentials, RoomId, RoomSecrets};
use zeroize::Zeroizing;

use crate::net::{self, Fetch, Relay, Route};
use crate::tor::EmbeddedTor;

/// How to use Tor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TorMode {
    /// The app's own Tor (default).
    Embedded,
    /// A Tor SOCKS proxy that is already running (9050 tor service, 9150 Tor Browser),
    /// or any `socks5h://` URL.
    External(String),
    /// No Tor. Only works for loopback relays unless `allow_direct` is set.
    Off,
}

/// How to reach the relay.
#[derive(Clone)]
pub struct NetConfig {
    /// Relay base URL, e.g. `http://abcd….onion` or `http://127.0.0.1:7878`.
    pub relay: String,
    /// Tor mode.
    pub tor: TorMode,
    /// Allow a direct (non-Tor) connection to a non-local relay. Exposes your IP.
    pub allow_direct: bool,
    /// Embedded Tor, once started.
    pub embedded: Option<Arc<EmbeddedTor>>,
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            relay: "http://127.0.0.1:7878".into(),
            tor: TorMode::Embedded,
            allow_direct: false,
            embedded: None,
        }
    }
}

impl NetConfig {
    /// Does this config need the embedded Tor client to be running?
    pub fn needs_embedded_tor(&self) -> bool {
        self.tor == TorMode::Embedded && !net::is_loopback(&self.relay)
    }

    /// Decide the route. Never talks to a non-local relay without Tor unless allowed.
    pub fn route(&self) -> Result<Route> {
        url::Url::parse(&self.relay).map_err(|_| anyhow!("invalid relay URL: {}", self.relay))?;
        if net::is_loopback(&self.relay) {
            return Ok(Route::Direct);
        }
        match &self.tor {
            TorMode::Embedded => match &self.embedded {
                Some(t) => Ok(Route::Embedded(t.clone())),
                None => bail!("built-in Tor is not ready yet"),
            },
            TorMode::External(proxy) => Ok(Route::Socks(proxy.clone())),
            TorMode::Off if self.allow_direct && !net::is_onion(&self.relay) => Ok(Route::Direct),
            TorMode::Off if net::is_onion(&self.relay) => {
                bail!(".onion relays can only be reached through Tor — turn Tor on")
            }
            TorMode::Off => bail!(
                "refusing to contact a remote relay without Tor (it would see your IP). \
                 Turn Tor on, or explicitly allow direct connections."
            ),
        }
    }

    /// Turn a low-level error into something a human can act on.
    pub fn explain(&self, e: anyhow::Error) -> anyhow::Error {
        let text = format!("{e:#}");
        let refused =
            text.contains("connect") || text.contains("Connect") || text.contains("10061");
        if net::is_loopback(&self.relay) && refused {
            anyhow!(
                "no relay is running on this computer ({}). Set your relay's .onion address \
                 (Relay field in the app, --relay in the terminal) — or start ghostline-relay \
                 here for a local test.",
                self.relay
            )
        } else if matches!(self.tor, TorMode::External(_)) && refused {
            anyhow!("could not reach Tor at the configured SOCKS port. Is Tor / Tor Browser running? ({text})")
        } else {
            e
        }
    }
}

/// A room that is ready to be entered.
pub struct Prepared {
    /// Room ID.
    pub id: RoomId,
    /// Room password.
    pub password: Zeroizing<String>,
    /// Derived secrets.
    pub secrets: RoomSecrets,
    /// Relay client bound to this room's mailbox.
    pub relay: Relay,
    /// Route used.
    pub route: Route,
}

/// Create a brand-new room on the relay. `password = None` → generated.
pub fn create_room(
    rt: &tokio::runtime::Handle,
    cfg: &NetConfig,
    password: Option<&str>,
) -> Result<Prepared> {
    let route = cfg.route()?;
    for _ in 0..5 {
        let creds = RoomCredentials::new_room(password)?;
        let secrets = creds.derive(&KdfParams::DESKTOP)?;
        let relay = Relay::new(&cfg.relay, &route, &secrets.mailbox_id)?;
        let created = rt
            .block_on(relay.create(&secrets.delete_token_hash()))
            .map_err(|e| cfg.explain(e))?;
        if created {
            return Ok(Prepared {
                id: creds.id.clone(),
                password: creds.password.clone(),
                secrets,
                relay,
                route,
            });
        }
    }
    bail!("could not allocate a room id (relay keeps reporting collisions)")
}

/// Check that a room exists (and the password is right) before entering.
pub fn join_room(
    rt: &tokio::runtime::Handle,
    cfg: &NetConfig,
    id: RoomId,
    password: Zeroizing<String>,
) -> Result<Prepared> {
    let route = cfg.route()?;
    let creds = RoomCredentials { id, password };
    let secrets = creds.derive(&KdfParams::DESKTOP)?;
    let relay = Relay::new(&cfg.relay, &route, &secrets.mailbox_id)?;
    match rt
        .block_on(relay.fetch(u64::MAX))
        .map_err(|e| cfg.explain(e))?
    {
        Fetch::Blobs(_) => {}
        Fetch::NotFound => {
            bail!("room not found — or wrong password (these look identical on purpose)")
        }
        Fetch::Gone => bail!("this room has been burned"),
    }
    Ok(Prepared {
        id: creds.id.clone(),
        password: creds.password.clone(),
        secrets,
        relay,
        route,
    })
}

/// Start the embedded Tor client and block until it is ready.
pub fn start_tor(
    rt: &tokio::runtime::Handle,
    progress: crate::tor::Progress,
) -> Result<Arc<EmbeddedTor>> {
    rt.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(180),
            EmbeddedTor::start(progress),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "could not connect to the Tor network within 3 minutes. Is the internet up? \
                     If Tor is blocked where you are, connect through Mullvad first."
            )
        })?
    })
}
