# GhostLine architecture

## Components

```
┌────────────────────────── Desktop client (Tauri 2) ──────────────────────────┐
│  UI (HTML/CSS, matrix theme)  ⇄  ghostline-core (Rust)                        │
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

## 2. Joining and sessions (M2)

1. The joiner derives the secrets and fetches the mailbox.
   Empty mailbox → "Room not found or wrong password". These two cases cannot be told apart.
2. The joiner generates a fresh, **per-room** Signal identity key. There are no
   long-term identities, so rooms cannot be linked to each other.
3. The joiner posts a `JOIN` envelope encrypted with `room_key`
   (XChaCha20-Poly1305): nickname, identity key and prekey bundle.
4. Existing members set up pairwise Signal sessions with the joiner and send it their
   current **Sender Key** over those sessions. Group messages use Sender Keys; the
   pairwise sessions carry control messages and key rotation.
5. When a member leaves or the room is re-keyed, everyone rotates their Sender Key.
   New members never get old keys, so they cannot read history.

`room_key` protects only the join handshake. Message content is protected by the
ratchets, so a leaked password does not decrypt past traffic.

### Envelope format

```
mailbox_id (32) ‖ msg_id (16, random) ‖ ciphertext (padded to 256/1k/4k/16k buckets)
```

Sender, nickname, timestamp and type all live **inside** the ciphertext.

### Delivery status

| Mark | Meaning | How |
|---|---|---|
| ✓ | stored on relay | relay ACK on POST |
| ✓✓ | delivered | encrypted `RECEIPT{delivered,msg_id}` from the recipient (for groups: from all members) |
| ✓✓ (green, bright) | read | encrypted `RECEIPT{read,msg_id}`; off switch per user |

## 3. Relay (M3)

- Hosted only as a v3 onion service. It has no clearnet listener.
- API: `PUT /mb/{id}` (create, with `delete_hash`), `POST /mb/{id}`, `GET /mb/{id}?since=`,
  `DELETE /mb/{id}` with header `X-Delete-Token`. The relay checks
  `SHA-256(token) == delete_hash` and drops everything immediately.
- Storage: an in-memory map by default. Optionally an encrypted on-disk store with a
  TTL (default 7 days) and coarse hour buckets instead of timestamps.
- Logging: compiled out. No request logs and no access logs. There are no IPs or user
  agents to log anyway, because traffic arrives over Tor and the client sends no UA header.
- Anyone can run their own relay. The relay address is part of the room invite (optional
  field; defaults to the project relay).

## 4. Transport (M4)

- Arti is embedded as a library (`arti-client`), and the client has its own Tor instance.
- **Stream isolation:** each room gets its own isolation token, so separate rooms never share a circuit.
- **Rotation:** a new circuit every *N* minutes (default 10, configurable), on top of
  Tor's own rotation.
- HTTP over Tor uses a fixed, minimal request set: no `User-Agent`, no cookies and no
  compression (to avoid leaking information through compressed sizes).
- Bridges and pluggable transports (obfs4, snowflake) are available in settings.

## 5. Desktop app (M5)

- Tauri 2, so the Rust core runs in-process and the UI is HTML/CSS (matrix theme).
- **Screen-capture exclusion:** on Windows, `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`.
  On Linux it is not enforceable, so the app shows a warning banner.
- Local store: SQLCipher. Each room has its own key, which is wrapped by a device key.
  Optionally an app PIN unlocks the device key, using Argon2id.
- Screens: **Chat oluştur** (nickname, ID, auto password, *Şifre öner*), **Odaya katıl**,
  chat view, **Odayı sil** (hold for 1.5 s, then it happens at once with no further confirmation).

## 6. Burn room (M6)

1. Send an encrypted `BURN` control message to all members.
2. `DELETE /mb/{id}` with the delete token.
3. Locally: destroy the room key, the Sender Keys and the sessions, delete the
   SQLCipher database file, and zeroize memory.
4. Each honest client does the same thing when it receives `BURN` or when it sees that the mailbox is gone.
