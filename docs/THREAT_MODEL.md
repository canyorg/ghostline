# Threat model

## Assets

Message content · who talks to whom · which rooms a person is in · users' network
locations (IPs) · nicknames.

## Adversaries and what they get

| Adversary | Content | IP of users | Who is in which room | Timing |
|---|---|---|---|---|
| Relay operator (malicious) | ❌ | ❌ (onion service) | ❌ (only random mailbox ids) | ✅ blob arrival times, sizes (padded) |
| Network observer near a user (ISP, Wi-Fi) | ❌ | – | ❌ (sees only Tor) | ✅ when the user is active |
| Tor relay operator (single node) | ❌ | partial (guard sees the user's IP, nothing else) | ❌ | partial |
| Global passive adversary | ❌ | ⚠️ possibly, through timing correlation | ⚠️ | ✅ |
| Someone with room ID only | ❌ | ❌ | ❌ (cannot find the mailbox) | ❌ |
| Someone with room ID + password | future messages only | ❌ | sees nicknames in the room | ✅ |
| Malicious room member | everything they receive | ❌ | nicknames | ✅ |
| Modified client | everything it receives; ignores BURN | ❌ | nicknames | ✅ |
| Malware on a user's device | ✅ | ✅ | ✅ | ✅ |

## Out of scope

- Compromised endpoints (malware, physical access to an unlocked device).
- Photographing the screen.
- Global passive adversaries. Tor's own limits apply.
- Coercion of users.

## Mitigations recap

- Argon2id (128 MiB) against offline password guessing.
- Per-room Signal identities, so rooms cannot be linked.
- Message padding buckets against size analysis.
- Receipts and typing indicators can be switched off to reduce timing leaks.
- Stream isolation per room and periodic circuit rotation.
- A Damm check digit, so typos are rejected locally and never reach the relay.
- Nickname validation that blocks bidi, zero-width and control characters, against impersonation.
