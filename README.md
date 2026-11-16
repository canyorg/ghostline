```
 ██████╗ ██╗  ██╗ ██████╗ ███████╗████████╗██╗     ██╗███╗   ██╗███████╗
██╔════╝ ██║  ██║██╔═══██╗██╔════╝╚══██╔══╝██║     ██║████╗  ██║██╔════╝
██║  ███╗███████║██║   ██║███████╗   ██║   ██║     ██║██╔██╗ ██║█████╗  
██║   ██║██╔══██║██║   ██║╚════██║   ██║   ██║     ██║██║╚██╗██║██╔══╝  
╚██████╔╝██║  ██║╚██████╔╝███████║   ██║   ███████╗██║██║ ╚████║███████╗
 ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝   ╚══════╝╚═╝╚═╝  ╚═══╝╚══════╝
```

# GhostLine

**Military-grade anonymous chat — Signal Protocol + Tor onion transport, zero metadata.**

No accounts. No phone numbers. No e-mail. No IP addresses. No user agents. No logs.
You get a room number and a password, you pick a nickname, you talk, you burn the room.

> ⚠️ **Status: pre-alpha, not audited.** The cryptographic building blocks are
> established (Signal Protocol, Tor, Argon2id), but this implementation has not been
> independently reviewed. Do not rely on it where your safety depends on it until an
> external audit is done. See [Known limitations](#known-limitations).

---

## Features

| | |
|---|---|
| 🔐 **End-to-end encryption** | **Now:** Signal-style *Sender Keys* — per-member ratcheting chains (forward secrecy), keys handed out pairwise over X25519, every message Ed25519-signed, re-key when someone leaves. XChaCha20-Poly1305 + HKDF-SHA256. **Next:** full `libsignal` (PQXDH + Double Ratchet) for post-compromise security. |
| 🧅 **Tor-only transport** | **Now:** routes through your Tor (`--tor`), refuses to talk to a non-local relay without Tor, relay runs as a `.onion` service → 6 hops, the server never sees an IP. **Next:** Tor embedded via Arti with timed circuit rotation and per-room stream isolation. |
| 👻 **Zero metadata** | No accounts, no IPs, no user agents, no contact lists. The relay only stores opaque ciphertext under a random mailbox address. Nicknames exist only inside encrypted payloads. |
| 🔢 **Numeric room IDs** | Like a crypto wallet or a Mullvad account number: 16–23 digits, **random length**, with a Damm check digit that catches every single typo and swapped neighbour. |
| 🔑 **Room password** | Set your own, or get one automatically. **"Şifre öner"** generates 16+ random characters (~123 bits by default). ID and password never change. |
| 🏷️ **Nickname per room** | Chosen when you enter a room. Unicode allowed; invisible/bidi spoofing characters blocked. |
| ✓✓ **Delivery status** | ✓ reached relay · ✓✓ delivered to device · **✓✓** read (encrypted receipts, can be turned off). |
| 🚫 **Screenshot protection** | Desktop app (Windows): window excluded from screen capture. Terminal client: not possible, see limitations. |
| 💥 **Burn room** | One action: the relay drops the mailbox instantly and every member's client wipes the room's keys and local history. |
| 🟩 **Hacker UI** | Green-on-black terminal client for Linux and Windows (PowerShell / Windows Terminal) today; matrix-style desktop app next. |

## Quick start (terminal client)

Requires [Rust](https://rustup.rs). Works on Linux (Fedora, Debian, Arch…), Windows 10/11 (PowerShell, Windows Terminal) and macOS.

```bash
git clone https://github.com/canyorg/ghostline && cd ghostline
cargo build --release
```

Binaries land in `target/release/` (`ghostline`, `ghostline-relay`; `.exe` on Windows).

### Try it locally (3 terminals)

```bash
# terminal 1 — the relay (RAM only, logs nothing)
./target/release/ghostline-relay

# terminal 2 — create a room (Enter at the password prompt = auto-generate)
./target/release/ghostline create --nick "Zero Cool"

# terminal 3 — join it with the ID + password shown in terminal 2
./target/release/ghostline join 1076 7380 0491 6080 894 --nick "Acid Burn"
```

On Windows use `.\target\release\ghostline.exe` (same arguments).

In the room: type and press Enter. `/help` · `/who` · `/info` · `/quit` (Ctrl+C) · `/burn`.
Status marks: ⏳ sending · ✓ on relay · ✓✓ delivered to everyone · **cyan ✓✓** read by everyone.

### Over Tor (real use)

1. **Run the relay as an onion service** (on any Linux box):

   ```bash
   sudo dnf install tor          # Debian/Ubuntu: sudo apt install tor
   sudo tee -a /etc/tor/torrc <<'TORRC'
   HiddenServiceDir /var/lib/tor/ghostline/
   HiddenServicePort 80 127.0.0.1:7878
   TORRC
   sudo systemctl restart tor
   sudo cat /var/lib/tor/ghostline/hostname     # → abcd…xyz.onion
   ./target/release/ghostline-relay              # stays on 127.0.0.1
   ```

2. **Clients** need Tor running locally: `sudo dnf install tor && sudo systemctl start tor`
   (Linux, SOCKS port 9050) or just open **Tor Browser** (Windows/Linux, port 9150).

   ```bash
   ghostline create --relay http://abcd…xyz.onion                 # tor service
   ghostline join <ID> --relay http://abcd…xyz.onion --tor-port 9150   # Tor Browser
   ```

   Tip: `export GHOSTLINE_RELAY=http://abcd…xyz.onion` (PowerShell: `$env:GHOSTLINE_RELAY="…"`).

`.onion` relays are always reached through Tor. A clearnet relay is refused unless you
pass `--tor`, or explicitly `--no-tor` (which exposes your IP to the relay).

## How a room works

```
CREATE                                   JOIN
──────                                   ────
nickname: Zero Cool                      room id : 3414 7650 6754 3639 1733 042
room id : 3414 7650 6754 3639 1733 042   password: ********************
password: [auto]  [Şifre öner]           nickname: Acid Burn
        │                                        │
        └──► Argon2id(password, id) ──► room key · mailbox id · delete token
                                                 │
                     Tor (3 hops) ──► relay.onion (3 hops) ──► encrypted mailbox
```

The ID alone is useless: the relay mailbox address is derived from the **password**,
so without it you cannot even find the room. A wrong password and a non-existent room
look exactly the same.

Full design: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · Threat model: [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md)

## Known limitations

We think a security tool should be upfront about what it **cannot** do:

1. **Deletion is only guaranteed for unmodified clients.** "Burn room" wipes the relay
   and every honest GhostLine client. A modified client, or someone who copied the text
   before you burned the room, can keep the messages. No messenger can prevent this.
2. **Screenshot protection is limited.**
   - **Terminal client:** none — the terminal emulator owns the window. Use the desktop app if this matters.
   - **Windows:** the window shows up black in screenshots, recordings and screen
     sharing (Windows 10 2004+). Other tools, VMs or capture cards can still get around this.
   - **Linux:** not possible. Any X11 client can read the screen; on Wayland it depends
     on the compositor. GhostLine only warns you.
   - **Everywhere:** nobody can stop someone from photographing the screen with a phone.
3. **A compromised device defeats everything.** Malware, keyloggers or someone with
   your unlocked computer see what you see. End-to-end encryption protects data in transit, not your device.
4. **The room password is a shared secret.** Anyone who has the ID and the password can
   join and can burn the room. Forward secrecy means a leaked password does **not**
   reveal past messages, but it does let someone into future ones.
5. **Weak passwords can be brute-forced offline** by someone who knows the room ID and
   has a copy of the relay's data. Argon2id (128 MiB) slows this down a lot, but use the
   generated passwords.
6. **Tor does not beat a global observer.** An adversary watching both ends of the
   network can correlate traffic timing. Read receipts and typing indicators leak timing,
   which is why they can be switched off.
7. **The relay operator sees** mailbox IDs, padded ciphertext sizes and when blobs arrive.
   It does **not** see IPs, nicknames, room IDs, passwords or content.
8. **In countries that block Tor** you will need bridges (pluggable transports).
9. **Memory hygiene is best-effort.** Keys are zeroized after use, but swap files,
   hibernation files and crash dumps can still hold secrets. Use full-disk encryption.
10. **Interim group crypto.** v0 uses Sender Keys: forward secrecy yes, post-compromise
    security no (a stolen chain key reads that member's messages until the next re-key).
    libsignal pairwise ratchets are on the roadmap.
11. **Fingerprints are short.** `nick#1a2b3c` is 24 bits — good for noticing a swap, not
    a proof of identity. Verify out of band if it matters.
12. **Not audited yet** (see the top of this file).

## Roadmap

- [x] **M1** Core: room IDs (Damm), password generator, nickname validation, Argon2id/HKDF derivation
- [x] **M2a** E2E group messaging (Sender Keys), signatures, padding, encrypted ✓/✓✓ receipts, re-key on leave
- [x] **M3** Relay: store-and-forward in RAM, TTL, instant delete + tombstone, no logs
- [x] **CLI** Terminal client for Linux and Windows, Tor via SOCKS
- [ ] **M2b** `libsignal` (PQXDH + Double Ratchet) pairwise sessions
- [ ] **M4** Embedded Tor (Arti): circuit rotation, per-room stream isolation, bridges
- [ ] **M5** Desktop app (Tauri 2) for Linux and Windows: matrix UI, screen-capture exclusion, encrypted local store
- [x] **M6a** Burn room (relay delete + tombstone + key wipe)
- [ ] **M6b** Panic wipe, encrypted local history (opt-in)
- [ ] **M7** Android
- [ ] **M8** External security audit

## Development

```bash
cargo test                                   # unit + protocol simulation tests
cargo clippy --all-targets -- -D warnings
```

## Layout

```
crates/core    protocol core: IDs, KDF, crypto, wire format, sans-IO session
crates/relay   relay server (RAM only, no logs)
crates/cli     terminal client `ghostline`
app/           Tauri 2 desktop client (Linux, Windows)
docs/          architecture and threat model
```

## License

AGPL-3.0-or-later (compatible with `libsignal`, which is AGPL-3.0).
Security issues: see [SECURITY.md](SECURITY.md).
