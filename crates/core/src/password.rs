//! Room passwords.
//!
//! - [`generate`] backs the **"Şifre öner"** button and the automatic password
//!   assigned when a user creates a room without typing one.
//! - [`validate_user_password`] enforces a minimum for user-chosen passwords.
//!
//! Alphabet: 72 symbols without look-alikes (`0 O o 1 l I |`), so a password read
//! aloud or copied by hand still works. 20 chars ≈ 123 bits of entropy.

use rand::rngs::OsRng;
use rand::seq::SliceRandom;
use zeroize::Zeroizing;

use crate::Error;

/// Length used for auto-generated passwords.
pub const DEFAULT_LENGTH: usize = 20;
/// Shortest password the generator will produce ("+15 hane").
pub const MIN_GENERATED_LENGTH: usize = 16;
/// Minimum length accepted for a user-chosen password.
pub const MIN_USER_LENGTH: usize = 12;
/// Maximum accepted length (prevents KDF DoS with megabyte "passwords").
pub const MAX_LENGTH: usize = 128;

const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789!@#$%^&*-_=+?~:.";

/// Generate a random password of `len` characters (clamped to 16–128).
///
/// Uses the OS CSPRNG and uniform sampling (no modulo bias).
pub fn generate(len: usize) -> Zeroizing<String> {
    let len = len.clamp(MIN_GENERATED_LENGTH, MAX_LENGTH);
    let mut rng = OsRng;
    let mut out = Zeroizing::new(String::with_capacity(len));
    for _ in 0..len {
        let b = *ALPHABET.choose(&mut rng).expect("alphabet is non-empty");
        out.push(b as char);
    }
    out
}

/// Entropy in bits of a generated password of `len` characters.
pub fn generated_entropy_bits(len: usize) -> f64 {
    len as f64 * (ALPHABET.len() as f64).log2()
}

/// Validate a password the user typed themselves.
pub fn validate_user_password(p: &str) -> Result<(), Error> {
    let n = p.chars().count();
    if !(MIN_USER_LENGTH..=MAX_LENGTH).contains(&n) {
        return Err(Error::PasswordLength {
            min: MIN_USER_LENGTH,
            max: MAX_LENGTH,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphabet_has_no_lookalikes_or_dupes() {
        for c in b"0Oo1lI|" {
            assert!(!ALPHABET.contains(c), "{}", *c as char);
        }
        let mut v = ALPHABET.to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), ALPHABET.len());
    }

    #[test]
    fn generated_length_and_charset() {
        let p = generate(DEFAULT_LENGTH);
        assert_eq!(p.len(), DEFAULT_LENGTH);
        assert!(p.bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(generate(3).len(), MIN_GENERATED_LENGTH);
        assert!(generated_entropy_bits(DEFAULT_LENGTH) > 120.0);
    }

    #[test]
    fn generated_passwords_differ() {
        assert_ne!(*generate(20), *generate(20));
    }

    #[test]
    fn user_password_policy() {
        assert!(validate_user_password("short").is_err());
        assert!(validate_user_password("correct horse battery").is_ok());
        assert!(validate_user_password(&"x".repeat(129)).is_err());
    }
}
