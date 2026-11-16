//! Relay client.
//!
//! Three ways to reach a relay:
//!
//! - **Embedded Tor** (default for any non-local relay): HTTP/1.1 over an Arti stream.
//!   Every room uses its own isolated circuits, which are replaced every
//!   [`ROTATE_EVERY`](crate::tor::ROTATE_EVERY).
//! - **External Tor**: through a SOCKS5h proxy (system tor on 9050, Tor Browser on 9150).
//! - **Direct**: loopback relays, or explicitly allowed (exposes your IP to the relay).
//!
//! Requests carry no User-Agent, no cookies, no compression headers.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use arti_client::TorClient;
use bytes::Bytes;
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;
use tokio::sync::Mutex;
use tor_rtcompat::PreferredRuntime;

use crate::tor::{EmbeddedTor, ROTATE_EVERY};

/// How we reach the relay.
#[derive(Clone)]
pub enum Route {
    /// Direct connection (loopback, or explicitly allowed).
    Direct,
    /// External SOCKS5h proxy (system tor / Tor Browser).
    Socks(String),
    /// The app's own Tor client.
    Embedded(Arc<EmbeddedTor>),
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

impl Route {
    /// Short human label, e.g. "built-in tor".
    pub fn label(&self) -> String {
        match self {
            Route::Direct => "direct".into(),
            Route::Socks(p) => format!("tor {}", p.trim_start_matches("socks5h://")),
            Route::Embedded(_) => "built-in tor".into(),
        }
    }

    /// Does this route go through Tor?
    pub fn is_tor(&self) -> bool {
        !matches!(self, Route::Direct)
    }
}

/// Result of a fetch.
pub enum Fetch {
    /// New blobs, in order, with their sequence numbers.
    Blobs(Vec<(u64, Vec<u8>)>),
    /// Mailbox does not exist (or wrong password).
    NotFound,
    /// Mailbox was burned.
    Gone,
}

/// Result of a post.
pub enum Post {
    /// Stored.
    Ok,
    /// Mailbox does not exist.
    NotFound,
    /// Mailbox was burned.
    Gone,
}

struct ArtiConn {
    client: Arc<TorClient<PreferredRuntime>>,
    sender: Option<SendRequest<Full<Bytes>>>,
    since: Instant,
}

struct ArtiHttp {
    tor: Arc<EmbeddedTor>,
    host: String,
    port: u16,
    conn: Mutex<ArtiConn>,
}

impl ArtiHttp {
    async fn connect(&self, c: &mut ArtiConn) -> Result<()> {
        let stream = tokio::time::timeout(
            Duration::from_secs(90),
            c.client.connect((self.host.as_str(), self.port)),
        )
        .await
        .map_err(|_| anyhow!("timed out reaching the relay through Tor"))?
        .context("could not reach the relay through Tor")?;
        let (sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .context("HTTP handshake with relay failed")?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        c.sender = Some(sender);
        Ok(())
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        header: Option<(&str, String)>,
        body: Vec<u8>,
    ) -> Result<(StatusCode, Bytes)> {
        let mut c = self.conn.lock().await;
        // Periodically move to brand-new circuits ("new identity").
        if c.since.elapsed() > ROTATE_EVERY {
            c.client = self.tor.isolated();
            c.sender = None;
            c.since = Instant::now();
        }
        let mut last_err = None;
        for _attempt in 0..2 {
            if c.sender.as_ref().is_none_or(|s| s.is_closed()) {
                if let Err(e) = self.connect(&mut c).await {
                    last_err = Some(e);
                    continue;
                }
            }
            let mut req = Request::builder()
                .method(method.clone())
                .uri(path)
                .header(http::header::HOST, &self.host);
            if let Some((k, v)) = &header {
                req = req.header(*k, v);
            }
            let req = req.body(Full::new(Bytes::from(body.clone())))?;
            let sender = c.sender.as_mut().expect("connected above");
            let res = tokio::time::timeout(Duration::from_secs(90), async {
                sender.ready().await?;
                let resp = sender.send_request(req).await?;
                let status = resp.status();
                let bytes = resp.into_body().collect().await?.to_bytes();
                Ok::<_, hyper::Error>((status, bytes))
            })
            .await;
            match res {
                Ok(Ok(r)) => return Ok(r),
                Ok(Err(e)) => last_err = Some(e.into()),
                Err(_) => last_err = Some(anyhow!("relay request timed out")),
            }
            c.sender = None; // reconnect on next attempt
        }
        Err(last_err.unwrap_or_else(|| anyhow!("relay request failed")))
    }
}

enum Backend {
    Http(reqwest::Client),
    Arti(ArtiHttp),
}

/// A relay client bound to one room's mailbox.
pub struct Relay {
    backend: Backend,
    /// Full URL for reqwest backends.
    url: String,
    /// Path (with mailbox) for the Arti backend.
    path: String,
}

impl Relay {
    /// Create a client for `mailbox` on the relay at `base`.
    pub fn new(base: &str, route: &Route, mailbox: &[u8; 32]) -> Result<Self> {
        let path = format!("/v1/mb/{}", hex::encode(mailbox));
        let url = format!("{}{}", base.trim_end_matches('/'), path);
        let backend = match route {
            Route::Embedded(tor) => {
                let u = url::Url::parse(base).context("invalid relay URL")?;
                let host = u.host_str().context("relay URL has no host")?.to_owned();
                let port = u.port_or_known_default().unwrap_or(80);
                if u.scheme() != "http" {
                    bail!("use http:// for .onion relays (Tor already encrypts and authenticates them)");
                }
                Backend::Arti(ArtiHttp {
                    tor: tor.clone(),
                    host,
                    port,
                    conn: Mutex::new(ArtiConn {
                        client: tor.isolated(), // per-room circuit isolation
                        sender: None,
                        since: Instant::now(),
                    }),
                })
            }
            Route::Direct | Route::Socks(_) => {
                let mut b = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(60))
                    .timeout(Duration::from_secs(90))
                    .pool_idle_timeout(Duration::from_secs(30));
                b = match route {
                    Route::Socks(p) => {
                        b.proxy(reqwest::Proxy::all(p).context("invalid proxy URL")?)
                    }
                    _ => b.no_proxy(),
                };
                Backend::Http(b.build()?)
            }
        };
        Ok(Relay { backend, url, path })
    }

    async fn call(
        &self,
        method: Method,
        query: &str,
        header: Option<(&str, String)>,
        body: Vec<u8>,
    ) -> Result<(StatusCode, Bytes)> {
        match &self.backend {
            Backend::Arti(a) => {
                a.request(method, &format!("{}{query}", self.path), header, body)
                    .await
            }
            Backend::Http(c) => {
                let mut r = c.request(method, format!("{}{query}", self.url)).body(body);
                if let Some((k, v)) = header {
                    r = r.header(k, v);
                }
                let resp = r.send().await.map_err(explain_reqwest)?;
                let status = resp.status();
                Ok((status, resp.bytes().await?))
            }
        }
    }

    /// Returns false if the mailbox id already exists.
    pub async fn create(&self, delete_hash: &[u8; 32]) -> Result<bool> {
        let (s, _) = self
            .call(Method::PUT, "", None, delete_hash.to_vec())
            .await?;
        match s {
            StatusCode::CREATED => Ok(true),
            StatusCode::CONFLICT | StatusCode::GONE => Ok(false),
            s => bail!("relay refused to create room: HTTP {s}"),
        }
    }

    /// Fetch blobs with sequence number ≥ `since`.
    pub async fn fetch(&self, since: u64) -> Result<Fetch> {
        let (s, body) = self
            .call(Method::GET, &format!("?since={since}"), None, Vec::new())
            .await?;
        match s {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Ok(Fetch::NotFound),
            StatusCode::GONE => return Ok(Fetch::Gone),
            s => bail!("relay error: HTTP {s}"),
        }
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

    /// Append a blob.
    pub async fn post(&self, blob: Vec<u8>) -> Result<Post> {
        let (s, _) = self.call(Method::POST, "", None, blob).await?;
        Ok(match s {
            StatusCode::OK => Post::Ok,
            StatusCode::NOT_FOUND => Post::NotFound,
            StatusCode::GONE => Post::Gone,
            s => bail!("relay error: HTTP {s}"),
        })
    }

    /// Delete the mailbox (burn the room).
    pub async fn burn(&self, token: &[u8; 32]) -> Result<bool> {
        let (s, _) = self
            .call(
                Method::DELETE,
                "",
                Some(("x-delete-token", hex::encode(token))),
                Vec::new(),
            )
            .await?;
        Ok(matches!(
            s,
            StatusCode::NO_CONTENT | StatusCode::NOT_FOUND | StatusCode::GONE
        ))
    }
}

fn explain_reqwest(e: reqwest::Error) -> anyhow::Error {
    if e.is_connect() {
        anyhow!("could not connect to the relay ({e})")
    } else if e.is_timeout() {
        anyhow!("the relay did not answer in time ({e})")
    } else {
        e.into()
    }
}

/// Is `relay_url` on this computer?
pub fn is_loopback(relay_url: &str) -> bool {
    url::Url::parse(relay_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .is_some_and(|h| matches!(h.as_str(), "127.0.0.1" | "localhost" | "[::1]" | "::1"))
}

/// Is `relay_url` an onion service?
pub fn is_onion(relay_url: &str) -> bool {
    url::Url::parse(relay_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .is_some_and(|h| h.ends_with(".onion"))
}
