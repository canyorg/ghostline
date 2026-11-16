//! Nicknames.
//!
//! Every user picks a nickname when entering a room. It is **per room**, never
//! tied to an account (there are no accounts), and it only ever travels inside
//! end-to-end encrypted payloads — the relay never sees it.
//!
//! Validation is about anti-spoofing: invisible and direction-changing Unicode
//! characters are rejected, so nobody can render as `admin` while actually
//! being `adm\u{202E}ni`.

use core::fmt;

use crate::Error;

/// Minimum nickname length (characters, after trimming).
pub const MIN_LEN: usize = 2;
/// Maximum nickname length (characters, after trimming).
pub const MAX_LEN: usize = 24;

/// A validated nickname.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Nickname(String);

fn is_forbidden(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00AD}'              // soft hyphen
            | '\u{034F}'            // combining grapheme joiner
            | '\u{061C}'            // arabic letter mark
            | '\u{115F}' | '\u{1160}' | '\u{3164}' | '\u{FFA0}' // hangul fillers
            | '\u{180E}'            // mongolian vowel separator
            | '\u{200B}'..='\u{200F}' // zero-width + LRM/RLM
            | '\u{2028}'..='\u{202E}' // separators + bidi embeddings/overrides
            | '\u{2060}'..='\u{206F}' // word joiner, bidi isolates, invisible ops
            | '\u{FE00}'..='\u{FE0F}' // variation selectors
            | '\u{FEFF}'            // BOM / ZWNBSP
            | '\u{FFF9}'..='\u{FFFB}' // interlinear annotations
            | '\u{E0000}'..='\u{E007F}' // tag characters
        )
}

fn is_allowed(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.')
}

impl Nickname {
    /// Validate and normalise (trim, collapse inner whitespace runs) a nickname.
    pub fn new(input: &str) -> Result<Self, Error> {
        for c in input.chars() {
            if is_forbidden(c) {
                return Err(Error::NicknameForbiddenChar(c as u32));
            }
        }
        let collapsed = input.split_whitespace().collect::<Vec<_>>().join(" ");
        for c in collapsed.chars() {
            if !is_allowed(c) {
                return Err(Error::NicknameForbiddenChar(c as u32));
            }
        }
        let n = collapsed.chars().count();
        if !(MIN_LEN..=MAX_LEN).contains(&n) {
            return Err(Error::NicknameLength {
                min: MIN_LEN,
                max: MAX_LEN,
            });
        }
        Ok(Nickname(collapsed))
    }

    /// The nickname as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Nickname {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Nickname {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Nickname({:?})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_names() {
        assert_eq!(Nickname::new("  neo  ").unwrap().as_str(), "neo");
        assert_eq!(Nickname::new("Zero   Cool").unwrap().as_str(), "Zero Cool");
        assert!(Nickname::new("Yılmaz_ç").is_ok());
        assert!(Nickname::new("Нео-1").is_ok());
    }

    #[test]
    fn rejects_spoofing_chars() {
        assert_eq!(
            Nickname::new("adm\u{202E}ni"),
            Err(Error::NicknameForbiddenChar(0x202E))
        );
        assert!(Nickname::new("ne\u{200B}o").is_err());
        assert!(Nickname::new("a\u{0007}b").is_err());
        assert!(Nickname::new("x\u{FEFF}y").is_err());
    }

    #[test]
    fn rejects_symbols_and_bad_length() {
        assert!(Nickname::new("<script>").is_err());
        assert!(Nickname::new("a").is_err());
        assert!(Nickname::new(&"a".repeat(25)).is_err());
    }
}
