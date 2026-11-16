//! GhostLine relay.
//!
//! A deliberately dumb store-and-forward box for opaque ciphertext:
//!
//! | Method | Path               | Body / header                 | Result                       |
//! |--------|--------------------|-------------------------------|------------------------------|
//! | PUT    | `/v1/mb/{id}`      | 32-byte delete-token hash     | 201 created · 409 exists     |
//! | POST   | `/v1/mb/{id}`      | blob (≤ 66 000 bytes)         | 200 `seq` · 404 · 410        |
//! | GET    | `/v1/mb/{id}?since=N` | –                          | 200 frames · 404 · 410       |
//! | DELETE | `/v1/mb/{id}`      | `X-Delete-Token: <hex>`       | 204 · 403 · 404 · 410        |
//!
//! GET returns frames `seq(u64 BE) ‖ len(u32 BE) ‖ blob`, at most 256 per call.
//!
//! What it does **not** do: log anything, look at headers, store timestamps,
//! know who anyone is. Run it as a Tor onion service and it never even sees IPs.
//! Everything lives in RAM; a restart wipes all rooms.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use axum::Router;
use clap::Parser;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MAX_BLOB: usize = 66_000;
const MAX_BLOBS_PER_BOX: usize = 20_000;
const MAX_BOXES: usize = 100_000;
const MAX_FRAMES_PER_FETCH: usize = 256;

#[derive(Parser, Debug)]
#[command(
    name = "ghostline-relay",
    version,
    about = "GhostLine relay (keeps no logs)"
)]
struct Args {
    /// Address to listen on. Keep it on loopback and publish it as an onion service.
    #[arg(long, default_value = "127.0.0.1:7878")]
    listen: SocketAddr,
    /// Delete rooms with no activity for this many hours.
    #[arg(long, default_value_t = 168)]
    ttl_hours: u64,
}

struct Mailbox {
    delete_hash: [u8; 32],
    blobs: Vec<(u64, Bytes)>,
    next_seq: u64,
    last_active: Instant,
}

struct Relay {
    boxes: Mutex<HashMap<[u8; 32], Mailbox>>,
    /// Burned mailbox ids, so a burned room cannot be resurrected.
    tombstones: Mutex<HashSet<[u8; 32]>>,
}

type Shared = Arc<Relay>;

fn parse_id(s: &str) -> Option<[u8; 32]> {
    let v = hex::decode(s).ok()?;
    v.try_into().ok()
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

async fn create(State(r): State<Shared>, Path(id): Path<String>, body: Bytes) -> Response {
    let Some(id) = parse_id(&id) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let Ok(delete_hash) = <[u8; 32]>::try_from(body.as_ref()) else {
        return status(StatusCode::BAD_REQUEST);
    };
    if r.tombstones.lock().unwrap().contains(&id) {
        return status(StatusCode::GONE);
    }
    let mut boxes = r.boxes.lock().unwrap();
    if boxes.contains_key(&id) {
        return status(StatusCode::CONFLICT);
    }
    if boxes.len() >= MAX_BOXES {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    }
    boxes.insert(
        id,
        Mailbox {
            delete_hash,
            blobs: Vec::new(),
            next_seq: 0,
            last_active: Instant::now(),
        },
    );
    status(StatusCode::CREATED)
}

fn missing(r: &Relay, id: &[u8; 32]) -> Response {
    if r.tombstones.lock().unwrap().contains(id) {
        status(StatusCode::GONE)
    } else {
        status(StatusCode::NOT_FOUND)
    }
}

async fn append(State(r): State<Shared>, Path(id): Path<String>, body: Bytes) -> Response {
    let Some(id) = parse_id(&id) else {
        return status(StatusCode::BAD_REQUEST);
    };
    if body.is_empty() || body.len() > MAX_BLOB {
        return status(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let mut boxes = r.boxes.lock().unwrap();
    let Some(mb) = boxes.get_mut(&id) else {
        drop(boxes);
        return missing(&r, &id);
    };
    if mb.blobs.len() >= MAX_BLOBS_PER_BOX {
        return status(StatusCode::INSUFFICIENT_STORAGE);
    }
    let seq = mb.next_seq;
    mb.next_seq += 1;
    mb.blobs.push((seq, body));
    mb.last_active = Instant::now();
    (StatusCode::OK, seq.to_string()).into_response()
}

#[derive(Deserialize)]
struct Since {
    #[serde(default)]
    since: u64,
}

async fn fetch(
    State(r): State<Shared>,
    Path(id): Path<String>,
    Query(q): Query<Since>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let boxes = r.boxes.lock().unwrap();
    let Some(mb) = boxes.get(&id) else {
        drop(boxes);
        return missing(&r, &id);
    };
    let start = mb.blobs.partition_point(|(s, _)| *s < q.since);
    let mut out = Vec::new();
    for (seq, blob) in mb.blobs[start..].iter().take(MAX_FRAMES_PER_FETCH) {
        out.extend_from_slice(&seq.to_be_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        out.extend_from_slice(blob);
    }
    (StatusCode::OK, out).into_response()
}

async fn burn(State(r): State<Shared>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let Some(id) = parse_id(&id) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let Some(token) = headers
        .get("x-delete-token")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| hex::decode(v).ok())
    else {
        return status(StatusCode::FORBIDDEN);
    };
    let hash: [u8; 32] = Sha256::digest(&token).into();
    let mut boxes = r.boxes.lock().unwrap();
    match boxes.get(&id) {
        None => {
            drop(boxes);
            missing(&r, &id)
        }
        Some(mb) if mb.delete_hash != hash => status(StatusCode::FORBIDDEN),
        Some(_) => {
            boxes.remove(&id); // ciphertext dropped from memory right here
            drop(boxes);
            r.tombstones.lock().unwrap().insert(id);
            status(StatusCode::NO_CONTENT)
        }
    }
}

fn app(state: Shared) -> Router {
    Router::new()
        .route(
            "/v1/mb/:id",
            put(create).post(append).get(fetch).delete(burn),
        )
        .layer(DefaultBodyLimit::max(MAX_BLOB))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let state: Shared = Arc::new(Relay {
        boxes: Mutex::new(HashMap::new()),
        tombstones: Mutex::new(HashSet::new()),
    });

    let ttl = Duration::from_secs(args.ttl_hours * 3600);
    let gc = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(600));
        loop {
            tick.tick().await;
            gc.boxes
                .lock()
                .unwrap()
                .retain(|_, mb| mb.last_active.elapsed() < ttl);
        }
    });

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .unwrap_or_else(|e| {
            eprintln!("cannot listen on {}: {e}", args.listen);
            std::process::exit(1);
        });
    eprintln!(
        "ghostline-relay listening on http://{} — RAM only, no logs, rooms expire after {}h idle",
        args.listen, args.ttl_hours
    );
    if !args.listen.ip().is_loopback() {
        eprintln!("warning: not on loopback. Prefer 127.0.0.1 + a Tor onion service.");
    }
    axum::serve(listener, app(state))
        .await
        .expect("server error");
}
