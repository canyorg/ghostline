<p align="center"><b>English</b> · <a href="README.tr.md">Türkçe</a> · <a href="README.ru.md">Русский</a></p>

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

> ⚠️ **Status: pre-alpha, not audited.** The building blocks are established (Tor, Argon2id,
> X25519, Ed25519, XChaCha20-Poly1305), but this implementation has not been reviewed
> independently. Do not rely on it where your safety depends on it until an audit is done.
> See [Known limitations](#known-limitations).

<p align="center"><img src="docs/img/chat.png" alt="GhostLine desktop app" width="820"></p>

---

## Why GhostLine? Zero trust

GhostLine is built so that you **do not have to trust anyone**: no company, no app vendor,
no server and no mobile carrier.

- **Carriers:** SMS and carrier services can be intercepted directly — by the operator,
  through SS7 weaknesses, or by anyone with access to the network. A phone number is also
  an identity. GhostLine uses neither.
- **WhatsApp is a closed box.** It says it uses the Signal Protocol, but its client is
  closed-source, so nobody outside the company can verify this for certain. On top of that,
  the operator still sees metadata: who talks to whom, when and how often. Many people
  simply do not trust Meta.
- **Zero trust** says: never trust, always verify. For private communication that means
  open code, no accounts, no phone numbers, end-to-end encryption and a network path that
  hides who you are. Following this principle, a tool like this becomes unavoidable.
- **Don't trust us either.** The code is open (AGPL-3.0); read it, build it yourself, and run
  your own relay. The relay only ever holds ciphertext, so even a hostile relay learns nothing
  useful.

**It can be improved, and we'd like to hear your ideas.** Open an
[issue](../../issues), start a discussion or send a pull request.

## Recommended setup: GhostLine + Mullvad VPN

GhostLine already hides your IP from the relay through Tor. For real privacy you also want
to hide from your **ISP** the fact that you use Tor and GhostLine at all. We recommend
[Mullvad VPN](https://mullvad.net). It needs no e-mail or personal data (just an account
number) and its apps are open-source. Before you start GhostLine, turn these on in Mullvad:

| Setting | Why |
|---|---|
| **Kill switch** (always on) + **Lockdown mode** | No traffic ever leaves your machine outside the VPN, not even while it reconnects or is disconnected |
| **DAITA** | Adds padding and decoy traffic so AI-based traffic analysis cannot fingerprint what you do |
| **Multihop → Always** | Traffic enters through one Mullvad server and exits from another one, so no single server sees both ends |

```
you ──► Mullvad (2 hops, DAITA) ──► Tor (3 hops) ──► relay.onion (3 hops)
        ISP sees: Mullvad, nothing else     relay sees: no IP, only ciphertext
```

This combination is the ideal setup. The VPN is not required, since Tor alone already hides
your IP from the relay. Mullvad adds a layer against your ISP and local network.

## How it stays private

### Why the relay cannot read your messages

End-to-end means the keys exist **only on the members' devices**:

1. The password never leaves your computer. Argon2id turns it into the room key on your device.
2. Every member creates fresh secret keys on their own device. Each member's message keys are
   handed to the others individually, encrypted with X25519, so even someone who knows the room
   password cannot unwrap them.
3. Every message is encrypted on the sender's device and decrypted only on the recipients'
   devices. In between, through Tor and on the relay, it is just a padded, random-looking blob.

The relay never sees a password, a key, a nickname or a room ID. It stores unreadable bytes
under a random mailbox address. A hostile relay operator, a hacker on the server, or a police
seizure of the server gets nothing readable. Because used keys are deleted right away, even
stealing a device later does not reveal earlier messages (forward secrecy).

### How Tor hides your IP address: 3 relays that cannot see each other

```
you ──► [1] guard ──► [2] middle ──► [3] rendezvous ◄── 3 more hops ◄── GhostLine relay (.onion)
        knows your IP    knows neither   knows neither your IP
        not the target   end              nor the relay's
```

Your traffic is wrapped in 3 layers of encryption (an "onion"). Each relay removes exactly one
layer and only learns the hop before it and the hop after it:

- **[1] Guard** sees your IP address but not where the traffic is going.
- **[2] Middle** sees neither you nor the destination.
- **[3] Last relay** sees the next step but not your IP.

No single relay can see both ends, which is why your IP address does not leak. The GhostLine
relay runs as an onion service behind its own 3 hops, so the full path is 6 hops with no exit
node, and the relay never learns any IP.

**Risk:** if the **1st and 3rd relay (or your guard and the relay's side) belong to the same
operator**, that operator can match the timing and size of the traffic on both ends and link
you to the room (a *traffic correlation* attack). Tor reduces this by keeping long-lived guards
and not putting relays of the same declared family or /16 network in one circuit, but it cannot
rule it out against a well-resourced attacker. **This is exactly why Mullvad in front of Tor
makes sense:** your guard then only sees a Mullvad server, not your IP, and DAITA scrambles the
timing and size patterns that correlation depends on.

## Maximum OPSEC: run it in a VM

For maximum operational security, run GhostLine inside a **Linux virtual machine (Fedora or
Kali)** rather than directly on Windows. Even stronger options are Whonix, which routes the whole
VM through Tor, and Tails.

**Why not Windows?** Windows sends telemetry tied to a persistent device identifier.
In 2026 the FBI unmasked an alleged Scattered Spider hacker partly through his Windows **Global
Device Identifier (GDID)**: the same Windows installation was linked across all the VPNs and IPs
he used, and finally to his home address
([The Register](https://www.theregister.com/a/5267953), [CyberInsider](https://cyberinsider.com/microsoft-windows-telemetry-identified-hacker-despite-vpn-use/)). Microsoft
has also confirmed that it hands BitLocker recovery keys stored in Microsoft accounts to
authorities under a legal order ([Tom's Hardware](https://www.tomshardware.com/tech-industry/cyber-security/microsoft-gave-customers-bitlocker-encryption-keys-to-the-fbi-redmond-confirms-that-it-provides-recovery-keys-to-government-agencies-with-valid-legal-orders)). End-to-end
encryption cannot protect you from your own operating system. **On Windows, Microsoft's
telemetry is itself an OPSEC risk.**

VM checklist: Linux guest, full-disk encryption, no shared clipboard or folders with the host,
Mullvad (lockdown mode + DAITA + multihop) on the host or in the VM, Tor in the VM, and a
snapshot you can roll back after use.

## Features

| | |
|---|---|
| 🔐 **End-to-end encryption** | **Now:** Signal-style *Sender Keys*. Each member has a ratcheting chain (forward secrecy), keys are handed out pairwise over X25519, every message is Ed25519-signed, and everyone re-keys when someone leaves. Uses XChaCha20-Poly1305 and HKDF-SHA256. **Next:** full `libsignal` (PQXDH + Double Ratchet). |
| 🧅 **Tor-only transport** | **Now:** routes through your Tor (`--tor`) and refuses a non-local relay without Tor. The relay is a `.onion` service, so the path has 6 hops and the server never sees an IP. **Next:** Tor embedded via Arti with timed circuit rotation. |
| 👻 **Zero metadata** | No accounts, no IPs, no user agents, no contact lists. The relay stores only padded ciphertext under a random mailbox address, in RAM. Nicknames exist only inside encrypted payloads. |
| 🔢 **Numeric room IDs** | Like a crypto wallet or a Mullvad account number: 16–23 digits with a **random length**. A Damm check digit catches every single-digit typo and every swap of two neighbouring digits. |
| 🔑 **Room password** | Set your own, or get one generated automatically. **"Suggest password"** produces 16+ random characters (~123 bits by default). The ID and password never change. |
| 🏷️ **Nickname per room** | Chosen when you enter a room, shown as `nick#fingerprint`. Unicode is allowed; invisible and bidi spoofing characters are blocked. |
| ✓✓ **Delivery status** | ⏳ sending · ✓ on relay · ✓✓ delivered to everyone · **cyan ✓✓** read by everyone. Receipts are encrypted and read receipts can be turned off. |
| 🚫 **Screenshot protection** | Desktop app on Windows: the window is excluded from screenshots, recordings and screen sharing. Linux and the terminal client cannot do this; see limitations. |
| 💥 **Burn room** | `/burn`: the relay deletes the mailbox instantly (and tombstones it), and every member's client wipes its keys. |
| 🟩 **Hacker UI** | **Desktop app** (Linux + Windows) with matrix rain, chat bubbles and hold-to-burn, in English, Turkish and Russian. Plus a green-on-black **terminal client**. Both talk to each other. |
| 🧱 **No web engine** | The desktop app is pure Rust (egui): no WebView, no JavaScript, no HTML. Message text is only ever drawn as plain text, and nothing is written to disk. |

## Quick start

You need [Rust](https://rustup.rs). It works on Linux (Fedora, Debian, Arch…), Windows 10/11
(PowerShell, Windows Terminal) and macOS.

```bash
git clone https://github.com/canyorg/ghostline && cd ghostline
cargo build --release
```

The binaries end up in `target/release/` (`.exe` on Windows):

| Binary | What it is |
|---|---|
| `ghostline-gui` | desktop app |
| `ghostline` | terminal client |
| `ghostline-relay` | relay server |

### Desktop app

```bash
./target/release/ghostline-relay &        # for a local test; use an .onion relay for real
./target/release/ghostline-gui
```

**CREATE CHAT** → nickname → password (generated automatically, or your own; **Suggest password**
gives a new one) → you get the room ID + password with copy buttons → **Enter room**.
**JOIN ROOM** → room ID (typos are caught as you type) → password → nickname.

In the room: 👥 members, room info (ID + password to share again), **Leave**, and
**🔥 BURN ROOM**: hold it for 1.5 s and the room is gone for everyone. Relay and Tor settings
are under **Network** on the start screen. The language follows your system (EN/TR/RU) and
can be switched there.

<p align="center"><img src="docs/img/home.png" width="400"> <img src="docs/img/created.png" width="400"></p>

### Terminal client

Try it locally in 3 terminals:

```bash
# terminal 1 — the relay (RAM only, logs nothing)
./target/release/ghostline-relay

# terminal 2 — create a room (press Enter at the password prompt to auto-generate one)
./target/release/ghostline create --nick "Zero Cool"

# terminal 3 — join with the ID + password shown in terminal 2
./target/release/ghostline join 1076 7380 0491 6080 894 --nick "Acid Burn"
```

On Windows use `.\target\release\ghostline.exe` with the same arguments.

Inside a room: `/help` · `/who` · `/info` · `/quit` (Ctrl+C) · `/burn`

### Over Tor (real use)

1. **Run the relay as an onion service** on any Linux machine:

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

2. **Clients** need Tor running locally. On Linux: `sudo dnf install tor && sudo systemctl start tor`
   (port 9050). On Windows or Linux you can also just open **Tor Browser** (port 9150).

   ```bash
   ghostline create --relay http://abcd…xyz.onion
   ghostline join <ID> --relay http://abcd…xyz.onion --tor-port 9150   # with Tor Browser
   ```

   Tip: `export GHOSTLINE_RELAY=http://abcd…xyz.onion` (PowerShell: `$env:GHOSTLINE_RELAY="…"`).

## How a room works

```
CREATE                                   JOIN
nickname: Zero Cool                      room id : 3414 7650 6754 3639 1733 042
room id : 3414 7650 6754 3639 1733 042   password: ********************
password: [auto]  [suggest]              nickname: Acid Burn
        └──► Argon2id(password, id) ──► room key · mailbox id · delete token
```

The ID alone is useless: the mailbox address is derived from the **password**, so without it
you cannot even find the room. A wrong password and a non-existent room look exactly the same.

Details: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md)

## Known limitations

A security tool should be upfront about what it **cannot** do:

1. **Deletion is only guaranteed for unmodified clients.** "Burn room" wipes the relay and
   every honest client. A modified client, or someone who copied the text earlier, can keep
   messages. No messenger can prevent this.
2. **Screenshot protection is limited.** The terminal client has none. The Windows desktop
   app hides the window from screen capture, though VMs or capture cards can get around it.
   On Linux it is not possible (X11, compositor-dependent on Wayland). No software can stop
   someone from photographing the screen.
3. **A compromised device defeats everything.** Malware or someone at your unlocked computer
   sees what you see.
4. **The room password is a shared secret.** Anyone with the ID and password can join and can
   burn the room. Thanks to forward secrecy they cannot read past messages.
5. **Weak passwords can be brute-forced offline** by someone who has the relay's data.
   Argon2id (128 MiB) slows this down a lot, but use the generated passwords.
6. **Traffic correlation.** If the same party controls your 1st Tor relay and the 3rd one
   (or the relay's side), or simply watches both ends of the network, it can match timing and
   sizes and link you to the room. Tor makes this hard but not impossible. Use Mullvad
   (multihop + DAITA) in front of Tor, and turn off read receipts if needed.
   See [How it stays private](#how-it-stays-private).
7. **The relay operator sees** mailbox IDs, padded sizes and arrival times. It does **not** see
   IPs, nicknames, room IDs, passwords or content.
8. **In countries that block Tor** you will need bridges (or Mullvad in front of Tor).
9. **Memory hygiene is best-effort.** Keys are zeroized after use, but swap, hibernation files
   and crash dumps can still hold secrets. Use full-disk encryption.
10. **Interim group crypto.** v0 Sender Keys give forward secrecy but no post-compromise
    security. libsignal is on the roadmap.
11. **Fingerprints are short.** `#1a2b3c` is 24 bits: enough to notice a swap, not enough to
    prove identity.
12. **Not audited yet.**
13. **Your operating system can betray you.** Windows telemetry ties activity to a persistent
    device ID. Run GhostLine in a Linux VM; see [Maximum OPSEC](#maximum-opsec-run-it-in-a-vm).

## Roadmap

- [x] **M1** Room IDs (Damm), password generator, nickname validation, Argon2id/HKDF
- [x] **M2a** E2E group messaging (Sender Keys), signatures, padding, ✓/✓✓ receipts, re-key on leave
- [x] **M3** Relay: RAM only, no logs, TTL, instant delete + tombstone
- [x] **CLI** Terminal client for Linux and Windows, Tor via SOCKS
- [x] **M6a** Burn room
- [x] **M5** Desktop app (Linux + Windows): matrix UI, bubbles, ✔/✔✔, hold-to-burn, screen-capture exclusion on Windows, EN/TR/RU
- [ ] **M2b** `libsignal` (PQXDH + Double Ratchet)
- [ ] **M4** Embedded Tor (Arti): circuit rotation, stream isolation, bridges
- [ ] **M6b** Panic wipe, optional encrypted local history
- [ ] **M7** Android
- [ ] **M8** External security audit

## Contributing

Ideas, bug reports, translations, code reviews and pull requests are all welcome. Please
report security problems privately, as described in [SECURITY.md](SECURITY.md).

```bash
cargo test                                   # unit + protocol simulation tests
cargo clippy --all-targets -- -D warnings
```

```
crates/core    protocol core: IDs, KDF, crypto, wire format, sans-IO session
crates/relay   relay server (RAM only, no logs)
crates/client  shared runtime: relay transport (Tor/SOCKS), create/join, room driver
crates/cli     terminal client `ghostline`
crates/gui     desktop app `ghostline-gui` (egui)
docs/          architecture and threat model
```

## License

AGPL-3.0-or-later.
