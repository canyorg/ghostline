//! Relay client. Plain HTTP over Tor (SOCKS5h) — the onion service itself
//! provides encryption and authentication of the relay.
//!
//! Requests carry no User-Agent, no cookies, no compression headers.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::{Client, StatusCode};

/// How we reach the relay.
#[derive(Clone, Debug)]
pub enum Route {
    /// Direct connection (only allowed for loopback, or with --no-tor).
    Direct,
    /// SOCKS5h proxy (Tor). DNS/.onion resolution happens inside Tor.
    Socks(String),
}

impl Route {
    /// Short human label, e.g. "tor 127.0.0.1:9050".
    pub fn label(&self) -> String {
        match self {
            Route::Direct => "direct".into(),
            Route::Socks(p) => format!("tor {}", p.trim_start_matches("socks5h://")),
        }
    }
}

pub enum Fetch {
    Blobs(Vec<(u64, Vec<u8>)>),
    NotFound,
    Gone,
}

pub enum Post {
    Ok,
    NotFound,
    Gone,
}

pub struct Relay {
    http: Client,
    url: String,
}

impl Relay {
    pub fn new(base: &str, route: &Route, mailbox: &[u8; 32]) -> Result<Self> {
        let mut b = Client::builder()
            .connect_timeout(Duration::from_secs(60))
            .timeout(Duration::from_secs(90))
            .pool_idle_timeout(Duration::from_secs(30));
        b = match route {
            Route::Direct => b.no_proxy(),
            Route::Socks(p) => b.proxy(reqwest::Proxy::all(p).context("invalid proxy URL")?),
        };
        Ok(Relay {
            http: b.build()?,
            url: format!(
                "{}/v1/mb/{}",
                base.trim_end_matches('/'),
                hex::encode(mailbox)
            ),
        })
    }

    /// Returns false if the mailbox id already exists.
    pub async fn create(&self, delete_hash: &[u8; 32]) -> Result<bool> {
        let r = self
            .http
            .put(&self.url)
            .body(delete_hash.to_vec())
            .send()
            .await?;
        match r.status() {
            StatusCode::CREATED => Ok(true),
            StatusCode::CONFLICT | StatusCode::GONE => Ok(false),
            s => bail!("relay refused to create room: HTTP {s}"),
        }
    }

    pub async fn fetch(&self, since: u64) -> Result<Fetch> {
        let r = self
            .http
            .get(format!("{}?since={since}", self.url))
            .send()
            .await?;
        match r.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Ok(Fetch::NotFound),
            StatusCode::GONE => return Ok(Fetch::Gone),
            s => bail!("relay error: HTTP {s}"),
        }
        let body = r.bytes().await?;
        let mut out = Vec::new();
        let mut p = &body[..];
        while p.len() >= 12 {
            let seq = u64::from_be_bytes(p[..8].try_into()?);
            let len = u32::from_be_bytes(p[8..12].try_into()?) as usize;
            if p.len() < 12 + len {
                bail!("truncated response from relay");
            }
            out.push((seq, p[12..12 + len].to_vec()));
            p = &p[12 + len..];
        }
        Ok(Fetch::Blobs(out))
    }

    pub async fn post(&self, blob: Vec<u8>) -> Result<Post> {
        let r = self.http.post(&self.url).body(blob).send().await?;
        Ok(match r.status() {
            StatusCode::OK => Post::Ok,
            StatusCode::NOT_FOUND => Post::NotFound,
            StatusCode::GONE => Post::Gone,
            s => bail!("relay error: HTTP {s}"),
        })
    }

    pub async fn burn(&self, token: &[u8; 32]) -> Result<bool> {
        let r = self
            .http
            .delete(&self.url)
            .header("x-delete-token", hex::encode(token))
            .send()
            .await?;
        Ok(matches!(
            r.status(),
            StatusCode::NO_CONTENT | StatusCode::NOT_FOUND | StatusCode::GONE
        ))
    }
}

/// Decide how to reach `relay_url`. Refuses clearnet connections unless asked.
pub fn choose_route(
    relay_url: &str,
    tor: bool,
    tor_port: u16,
    proxy: Option<&str>,
    no_tor: bool,
) -> Result<Route> {
    let u = url::Url::parse(relay_url).context("invalid --relay URL")?;
    let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
    let loopback = matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]" | "::1");
    let onion = host.ends_with(".onion");

    if let Some(p) = proxy {
        return Ok(Route::Socks(p.to_owned()));
    }
    if tor || onion {
        return Ok(Route::Socks(format!("socks5h://127.0.0.1:{tor_port}")));
    }
    if loopback || no_tor {
        return Ok(Route::Direct);
    }
    bail!(
        "refusing to contact {host} without Tor (it would see your IP).\n\
         Use --tor (Tor on 127.0.0.1:9050, or --tor-port 9150 for Tor Browser), \
         or --no-tor if you really mean it."
    )
}
