//! GhostLine relay — placeholder.
//!
//! Planned behaviour (see docs/ARCHITECTURE.md):
//! - Listens ONLY as a Tor onion service (no clearnet socket at all).
//! - Stores opaque ciphertext blobs keyed by a 32-byte mailbox id.
//! - Never logs: no IPs (it cannot see them), no user agents (there are none),
//!   no timestamps beyond a coarse TTL bucket.
//! - Deletes a mailbox instantly when presented with the preimage of its
//!   registered delete-token hash.

fn main() {
    eprintln!("ghostline-relay: not implemented yet (milestone M3)");
}
