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
| 🔐 **End-to-end encryption** | Signal Protocol (X3DH/PQXDH + Double Ratchet, Sender Keys for groups) via `libsignal`. Forward secrecy and post-compromise security. |
| 🧅 **Tor-only transport** | Tor is embedded (Arti, the Rust Tor implementation). Every connection goes through 3 hops; the relay itself is a `.onion` service, so the path is 6 hops and the server never learns anyone's IP. Circuits rotate automatically on a timer, and every room uses an isolated circuit. |
| 👻 **Zero metadata** | No accounts, no IPs, no user agents, no contact lists. The relay only stores opaque ciphertext under a random mailbox address. Nicknames exist only inside encrypted payloads. |
| 🔢 **Numeric room IDs** | Like a crypto wallet or a Mullvad account number: 16–23 digits, **random length**, with a Damm check digit that catches every single typo and swapped neighbour. |
| 🔑 **Room password** | Set your own, or get one automatically. **"Şifre öner"** generates 16+ random characters (~123 bits by default). ID and password never change. |
| 🏷️ **Nickname per room** | Chosen when you enter a room. Unicode allowed; invisible/bidi spoofing characters blocked. |
| ✓✓ **Delivery status** | ✓ reached relay · ✓✓ delivered to device · **✓✓** read (encrypted receipts, can be turned off). |
| 🚫 **Screenshot protection** | Windows: the window is excluded from screen capture. Linux: see limitations. |
| 💥 **Burn room** | One action: the relay drops the mailbox instantly and every member's client wipes the room's keys and local history. |
| 🟩 **Hacker UI** | Green-on-black matrix interface. |

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
10. **Not audited yet** (see the top of this file).

## Roadmap

- [x] **M1** Core: room IDs (Damm), password generator, nickname validation, Argon2id/HKDF derivation
- [ ] **M2** Signal Protocol sessions (`libsignal`), padding, encrypted receipts
- [ ] **M3** Relay: onion-only store-and-forward, TTL, instant delete
- [ ] **M4** Embedded Tor (Arti): circuit rotation, per-room stream isolation, bridges
- [ ] **M5** Desktop app (Tauri 2) for Linux and Windows: matrix UI, screen-capture exclusion, encrypted local store
- [ ] **M6** Burn room end-to-end and panic wipe
- [ ] **M7** Android
- [ ] **M8** External security audit

## Build

```bash
git clone https://github.com/canyorg/ghostline && cd ghostline
cargo test
cargo run --release -p ghostline-core --example new_room -- "Zero Cool"
```

## Layout

```
crates/core    protocol core (IDs, KDF, later Signal sessions + Tor transport)
crates/relay   onion-only relay server
app/           Tauri 2 desktop client (Linux, Windows)
docs/          architecture and threat model
```

## License

AGPL-3.0-or-later (compatible with `libsignal`, which is AGPL-3.0).
Security issues: see [SECURITY.md](SECURITY.md).
