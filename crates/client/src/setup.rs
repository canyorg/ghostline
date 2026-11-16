//! Creating and joining rooms. These functions block (Argon2id + network) —
//! call them from a worker thread in GUIs.

use anyhow::{bail, Context, Result};
use ghostline_core::{KdfParams, RoomCredentials, RoomId, RoomSecrets};
use zeroize::Zeroizing;

use crate::net::{self, Fetch, Relay, Route};

/// How to reach the relay.
#[derive(Clone, Debug)]
pub struct NetConfig {
    /// Relay base URL, e.g. `http://abcd….onion` or `http://127.0.0.1:7878`.
    pub relay: String,
    /// Force Tor (automatic for .onion).
    pub tor: bool,
    /// Tor SOCKS port (9050 tor service, 9150 Tor Browser).
    pub tor_port: u16,
    /// Custom SOCKS proxy URL.
    pub proxy: Option<String>,
    /// Allow direct connection to a non-local relay.
    pub no_tor: bool,
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            relay: "http://127.0.0.1:7878".into(),
            tor: false,
            tor_port: 9050,
            proxy: None,
            no_tor: false,
        }
    }
}

impl NetConfig {
    /// Decide the route (refuses clearnet without Tor unless `no_tor`).
    pub fn route(&self) -> Result<Route> {
        net::choose_route(
            &self.relay,
            self.tor,
            self.tor_port,
            self.proxy.as_deref(),
            self.no_tor,
        )
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
            .context("could not reach the relay — is it running? is Tor running?")?;
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
        .context("could not reach the relay — is it running? is Tor running?")?
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
