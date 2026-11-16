//! Small, misuse-resistant wrappers around the primitives we use.
//!
//! - AEAD: XChaCha20-Poly1305 (192-bit random nonces — safe to generate randomly)
//! - KDF:  HKDF-SHA256
//! - Padding: length-prefixed, padded to fixed buckets to hide message sizes

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;

use crate::Error;

/// Padding buckets. Every ciphertext on the relay has one of these sizes (+40 bytes).
pub const BUCKETS: [usize; 5] = [256, 1024, 4096, 16384, 65536];

/// Encrypt `pt` under `key` with a fresh random nonce.
pub fn seal(key: &[u8; 32], aad: &[u8], pt: &[u8]) -> ([u8; 24], Vec<u8>) {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: pt, aad })
        .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory buffers");
    (nonce, ct)
}

/// Decrypt and authenticate.
pub fn open(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], ct: &[u8]) -> Result<Vec<u8>, Error> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| Error::Decrypt)
}

/// HKDF-Expand with a 32-byte PRK and a multi-part info string.
pub fn expand(prk: &[u8; 32], info: &[&[u8]]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::from_prk(prk).expect("32-byte PRK is valid for SHA-256");
    let mut out = [0u8; 32];
    hk.expand_multi_info(info, &mut out)
        .expect("32 bytes is a valid HKDF output length");
    out
}

/// 32 random bytes from the OS CSPRNG.
pub fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    b
}

/// 16 random bytes from the OS CSPRNG.
pub fn random16() -> [u8; 16] {
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b
}

/// `u32 length ‖ data ‖ zeros` up to the next bucket.
pub fn pad(data: &[u8]) -> Result<Vec<u8>, Error> {
    let need = 4 + data.len();
    let bucket = *BUCKETS
        .iter()
        .find(|&&b| b >= need)
        .ok_or(Error::TooLarge)?;
    let mut out = Vec::with_capacity(bucket);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    out.resize(bucket, 0);
    Ok(out)
}

/// Inverse of [`pad`].
pub fn unpad(buf: &[u8]) -> Result<&[u8], Error> {
    if buf.len() < 4 {
        return Err(Error::Malformed);
    }
    let len = u32::from_be_bytes(buf[..4].try_into().expect("4 bytes")) as usize;
    buf.get(4..4 + len).ok_or(Error::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip_and_tamper() {
        let k = random32();
        let (n, mut ct) = seal(&k, b"aad", b"hello");
        assert_eq!(open(&k, &n, b"aad", &ct).unwrap(), b"hello");
        assert_eq!(open(&k, &n, b"AAD", &ct), Err(Error::Decrypt));
        ct[0] ^= 1;
        assert_eq!(open(&k, &n, b"aad", &ct), Err(Error::Decrypt));
    }

    #[test]
    fn padding_hides_length() {
        assert_eq!(pad(b"a").unwrap().len(), 256);
        assert_eq!(pad(&[7u8; 300]).unwrap().len(), 1024);
        assert_eq!(unpad(&pad(b"xyz").unwrap()).unwrap(), b"xyz");
        assert_eq!(pad(&[0u8; 70_000]), Err(Error::TooLarge));
        assert_eq!(unpad(&[0, 0, 1, 0, 1]), Err(Error::Malformed));
    }
}
