# GhostLine architecture

## Components

```
┌──────────────────────── Desktop app (egui) / terminal client ─────────────────┐
│  UI (matrix theme)  ⇄  ghostline-client  ⇄  ghostline-core (Rust)             │
│                                   ├─ room_id / password / nickname / kdf      │
│                                   ├─ signal   (libsignal sessions, sender keys)│
│                                   ├─ store    (SQLCipher, per-room keys)       │
│                                   └─ transport (Arti, onion client)            │
└──────────────────────────────────────┬───────────────────────────────────────┘
                                       │ Tor, 3 hops (client side)
                                       ▼
                              ┌─────────────────┐  3 hops (service side)
                              │  relay (.onion) │  ciphertext blobs only
                              └─────────────────┘
```

No component ever opens a clearnet socket. The client refuses to start any network
activity until Tor has bootstrapped.

## 1. Room credentials (M1, done)

- **Room ID:** 15–22 random digits (first digit not 0) plus a Damm check digit, so
  16–23 digits in total. Generated with the OS CSPRNG. Shown in groups of 4.
- **Password:** user-chosen (12–128 characters) or generated (16–128, default 20, from a
  72-character alphabet without look-alike characters; about 123 bits).
- Both are fixed for the room's lifetime.

```
salt        = SHA-256("ghostline/v1/room-salt" ‖ id)[..16]
master      = Argon2id(password, salt, m=128 MiB, t=3, p=4)
room_key    = HKDF(master, "ghostline/v1/room-key")
mailbox_id  = HKDF(master, "ghostline/v1/mailbox-id")
delete_tok  = HKDF(master, "ghostline/v1/delete-token")
```

The relay learns only `mailbox_id` and `SHA-256(delete_tok)`.

## 2. Joining and messaging (M2a, implemented — `crates/core/src/session.rs`)

Every blob on the relay is `nonce ‖ XChaCha20-Poly1305(room_key, pad(Inner))`. The outer
layer hides type, sender and size class from the relay. Inside are three signed messages:

| Inner | Purpose |
|---|---|
| `Hello{member, dh, nick}` | announces a fresh **per-room** Ed25519 identity + X25519 key, self-signed |
| `KeyDist{from, to, epoch, ct}` | sender's chain key, encrypted to one member with `HKDF(X25519(from,to))` |
| `Msg{from, epoch, index, ct}` | content encrypted with the sender's ratchet message key |

1. The joiner derives the secrets and fetches the mailbox.
   Empty/absent mailbox → "room not found or wrong password". The two are indistinguishable.
2. The joiner posts `Hello`. Everyone who sees a new `Hello` sends that member a `KeyDist`
   of their own **current** chain position; the joiner does the same for every `Hello` in history.
3. Sending: `mk = HKDF(ck,"msg")`, `ck' = HKDF(ck,"next")`, old keys deleted (forward secrecy).
   Receivers keep at most 4000 skipped keys for out-of-order delivery; replays are rejected.
4. Newcomers get the chain at its current index → **cannot read history**.
5. `Leave` → every remaining member starts a new epoch and re-distributes keys → the
   leaver cannot read on.
6. Every `Msg` is Ed25519-signed by the sender's room identity; a password holder can
   neither read nor forge.

`room_key` (from the password) only protects the outer layer and the act of joining.

**Planned (M2b):** replace KeyDist with libsignal pairwise sessions (PQXDH + Double
Ratchet) and rotate sender chains periodically, adding post-compromise security.

### Content types (inside `Msg`)

`Text{id, text, ts}` · `Receipt{Delivered|Read, ids}` · `Leave` · `Burn`

### Delivery status

| Mark | Meaning | How |
|---|---|---|
| ⏳ | sending | – |
| ✓ | stored on relay | relay 200 on POST |
| ✓✓ | delivered | `Receipt{Delivered}` from **every** member present when it was sent |
| ✓✓ (cyan) | read | `Receipt{Read}` from every member. Terminal client sends it on the next key press; `--no-read-receipts` disables |

## 3. Relay (M3, implemented — `crates/relay`)

- Hosted only as a v3 onion service. It has no clearnet listener.
- API: `PUT /mb/{id}` (create, with `delete_hash`), `POST /mb/{id}`, `GET /mb/{id}?since=`,
  `DELETE /mb/{id}` with header `X-Delete-Token`. The relay checks
  `SHA-256(token) == delete_hash` and drops everything immediately.
- Storage: RAM only. Idle rooms expire after a TTL (default 7 days). A restart wipes everything.
- Burned mailbox ids are kept as tombstones (hash only) so a burned room cannot be re-created.
- Logging: compiled out. No request logs and no access logs. There are no IPs or user
  agents to log anyway, because traffic arrives over Tor and the client sends no UA header.
- Anyone can run their own relay. The relay address is part of the room invite (optional
  field; defaults to the project relay).

## 4. Transport (M4a, implemented — `crates/client/src/tor.rs`, `net.rs`)

- Arti (`arti-client`, rustls, onion-service client, vanguards) runs **in-process**; no
  Tor Browser or system tor needed. The GUI starts it in the background as soon as a
  non-local relay is configured and shows bootstrap progress.
- HTTP/1.1 (hyper) runs directly over Arti `DataStream`s to the `.onion` relay.
- **Per-room isolation:** every `Relay` gets its own `isolated_client()`, so rooms never share circuits.
- **Rotation:** every 10 minutes the room switches to a fresh isolated client (new circuits).
- External Tor (SOCKS5h: 9050 tor service / 9150 Tor Browser) and direct mode remain
  available. A non-local relay is never contacted without Tor unless explicitly allowed.
- Requests carry no `User-Agent`, no cookies, no compression.
- State: guard + directory cache in the user data dir (like Tor Browser). With
  `GHOSTLINE_AMNESIC=1`, a temporary directory is used and deleted on exit.
- Not yet: bridges / pluggable transports (M4b).

## 5. Desktop app (M5, implemented — `crates/gui`)

- Pure Rust: egui/eframe with the glow (OpenGL) backend. There is no web view, no JS and no
  HTML parser, so there is no XSS-style attack surface: message text is only drawn as text.
- No persistence of app data: the eframe `persistence` feature is off (no settings,
  history or messages on disk). Only Tor's own state is kept, see §4.
- **Screen-capture exclusion:** on Windows, `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`.
  Linux cannot enforce this, so the app shows a warning.
- Screens: start (CREATE CHAT / JOIN ROOM, network settings, EN/TR/RU), create (nickname,
  auto/own password, *suggest password*), credentials card with copy buttons, join (live
  check-digit validation), chat (bubbles, ✔/✔✔/cyan ✔✔, members, info, leave,
  hold-to-burn for 1.5 s).
- Closing the window while in a room sends `Leave` first, so the others re-key.
- Both front-ends share `crates/client` (transport + room driver), so the GUI and the
  terminal client interoperate in the same room.

## 6. Burn room (M6)

1. Send an encrypted `BURN` control message to all members.
2. `DELETE /mb/{id}` with the delete token.
3. Locally: destroy the room key, the Sender Keys and the sessions, delete the
   SQLCipher database file, and zeroize memory.
4. Each honest client does the same thing when it receives `BURN` or when it sees that the mailbox is gone.
