//! Room identifiers.
//!
//! A room ID is a purely numeric string, like a Mullvad account number, but with
//! a **random length**: 15–22 random digits + 1 Damm check digit = 16–23 digits.
//!
//! - The first digit is never `0`, so the ID survives being pasted into
//!   spreadsheets / number fields.
//! - The Damm check digit catches **every** single-digit typo and **every**
//!   adjacent transposition (`…37…` → `…73…`), so a mistyped ID is rejected
//!   locally instead of silently "joining" an empty room.
//! - The ID is *not* the secret. ~50+ bits of entropy are enough to avoid
//!   collisions, but confidentiality comes from the password (see `kdf`).

use core::fmt;
use rand::rngs::OsRng;
use rand::Rng;

use crate::Error;

/// Minimum number of random payload digits.
pub const MIN_PAYLOAD_DIGITS: usize = 15;
/// Maximum number of random payload digits.
pub const MAX_PAYLOAD_DIGITS: usize = 22;
/// Minimum total length (payload + check digit).
pub const MIN_LEN: usize = MIN_PAYLOAD_DIGITS + 1;
/// Maximum total length (payload + check digit).
pub const MAX_LEN: usize = MAX_PAYLOAD_DIGITS + 1;

/// Damm algorithm quasigroup table (totally anti-symmetric, order 10).
const DAMM: [[u8; 10]; 10] = [
    [0, 3, 1, 7, 5, 9, 8, 6, 4, 2],
    [7, 0, 9, 2, 1, 5, 4, 8, 6, 3],
    [4, 2, 0, 6, 8, 7, 1, 3, 5, 9],
    [1, 7, 5, 0, 9, 8, 3, 4, 2, 6],
    [6, 1, 2, 3, 0, 4, 5, 9, 7, 8],
    [3, 6, 7, 4, 2, 0, 9, 5, 8, 1],
    [5, 8, 6, 9, 7, 2, 0, 1, 3, 4],
    [8, 9, 4, 5, 3, 6, 2, 0, 1, 7],
    [9, 4, 3, 8, 6, 1, 7, 2, 0, 5],
    [2, 5, 8, 1, 4, 3, 6, 7, 9, 0],
];

fn damm_interim(digits: &[u8]) -> u8 {
    digits
        .iter()
        .fold(0u8, |acc, &d| DAMM[acc as usize][d as usize])
}

/// Compute the Damm check digit for `digits` (each 0–9).
pub fn damm_check_digit(digits: &[u8]) -> u8 {
    damm_interim(digits)
}

/// A validated room identifier. Stored canonically (digits only).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RoomId(String);

impl RoomId {
    /// Generate a new random room ID using the OS CSPRNG.
    pub fn generate() -> Self {
        let mut rng = OsRng;
        let payload_len = rng.gen_range(MIN_PAYLOAD_DIGITS..=MAX_PAYLOAD_DIGITS);

        let mut digits = Vec::with_capacity(payload_len + 1);
        digits.push(rng.gen_range(1..=9u8)); // no leading zero
        for _ in 1..payload_len {
            digits.push(rng.gen_range(0..=9u8));
        }
        digits.push(damm_check_digit(&digits));

        RoomId(digits.iter().map(|d| char::from(b'0' + d)).collect())
    }

    /// Parse user input. Spaces and dashes are ignored, so `4821 0937 …` works.
    pub fn parse(input: &str) -> Result<Self, Error> {
        let mut digits = Vec::with_capacity(MAX_LEN);
        for c in input.chars() {
            match c {
                '0'..='9' => digits.push(c as u8 - b'0'),
                ' ' | '-' | '\u{00A0}' => {}
                _ => return Err(Error::RoomIdNotNumeric),
            }
        }
        if !(MIN_LEN..=MAX_LEN).contains(&digits.len()) {
            return Err(Error::RoomIdLength {
                min: MIN_LEN,
                max: MAX_LEN,
                got: digits.len(),
            });
        }
        if damm_interim(&digits) != 0 {
            return Err(Error::RoomIdChecksum);
        }
        Ok(RoomId(
            digits.iter().map(|d| char::from(b'0' + d)).collect(),
        ))
    }

    /// Canonical form: digits only. This is what goes into the KDF.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Human-friendly form, grouped in blocks of 4: `4821 0937 5512 0086 43`.
    pub fn display_grouped(&self) -> String {
        let mut out = String::with_capacity(self.0.len() + self.0.len() / 4);
        for (i, c) in self.0.chars().enumerate() {
            if i > 0 && i % 4 == 0 {
                out.push(' ');
            }
            out.push(c);
        }
        out
    }
}

impl fmt::Display for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_grouped())
    }
}

impl fmt::Debug for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RoomId({})", self.display_grouped())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damm_known_vector() {
        // Classic example: 572 -> check digit 4.
        assert_eq!(damm_check_digit(&[5, 7, 2]), 4);
        assert!(RoomId::parse("5724").is_err()); // too short, but checksum logic:
        assert_eq!(damm_interim(&[5, 7, 2, 4]), 0);
    }

    #[test]
    fn generated_ids_are_valid_and_vary_in_length() {
        let mut lengths = std::collections::HashSet::new();
        for _ in 0..2000 {
            let id = RoomId::generate();
            assert!((MIN_LEN..=MAX_LEN).contains(&id.as_str().len()));
            assert!(!id.as_str().starts_with('0'));
            assert_eq!(RoomId::parse(id.as_str()).unwrap(), id);
            assert_eq!(RoomId::parse(&id.display_grouped()).unwrap(), id);
            lengths.insert(id.as_str().len());
        }
        assert_eq!(
            lengths.len(),
            MAX_LEN - MIN_LEN + 1,
            "all lengths should appear"
        );
    }

    #[test]
    fn catches_every_single_digit_typo() {
        let id = RoomId::generate();
        let s: Vec<u8> = id.as_str().bytes().collect();
        for i in 0..s.len() {
            for d in b'0'..=b'9' {
                if d == s[i] {
                    continue;
                }
                let mut t = s.clone();
                t[i] = d;
                let t = String::from_utf8(t).unwrap();
                assert_eq!(RoomId::parse(&t), Err(Error::RoomIdChecksum), "{t}");
            }
        }
    }

    #[test]
    fn catches_every_adjacent_transposition() {
        for _ in 0..200 {
            let id = RoomId::generate();
            let s: Vec<u8> = id.as_str().bytes().collect();
            for i in 0..s.len() - 1 {
                if s[i] == s[i + 1] {
                    continue;
                }
                let mut t = s.clone();
                t.swap(i, i + 1);
                let t = String::from_utf8(t).unwrap();
                assert_eq!(RoomId::parse(&t), Err(Error::RoomIdChecksum), "{t}");
            }
        }
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(RoomId::parse("12ab"), Err(Error::RoomIdNotNumeric));
        assert!(matches!(
            RoomId::parse("123"),
            Err(Error::RoomIdLength { got: 3, .. })
        ));
    }
}
