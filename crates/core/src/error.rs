//! Error type for ghostline-core.

/// All errors produced by ghostline-core.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum Error {
    /// Room ID contains something other than digits / spaces / dashes.
    #[error("room ID may only contain digits")]
    RoomIdNotNumeric,
    /// Room ID has the wrong number of digits.
    #[error("room ID must be {min}–{max} digits long, got {got}")]
    RoomIdLength {
        /// Minimum allowed length.
        min: usize,
        /// Maximum allowed length.
        max: usize,
        /// Actual length.
        got: usize,
    },
    /// Damm check digit mismatch — almost always a typo.
    #[error("room ID check digit is wrong (typo?)")]
    RoomIdChecksum,
    /// User-chosen password too short or too long.
    #[error("password must be {min}–{max} characters long")]
    PasswordLength {
        /// Minimum allowed length.
        min: usize,
        /// Maximum allowed length.
        max: usize,
    },
    /// Nickname too short / too long after trimming.
    #[error("nickname must be {min}–{max} characters long")]
    NicknameLength {
        /// Minimum allowed length.
        min: usize,
        /// Maximum allowed length.
        max: usize,
    },
    /// Nickname contains a forbidden character (control, bidi override, zero-width, …).
    #[error("nickname contains a forbidden character: U+{0:04X}")]
    NicknameForbiddenChar(u32),
    /// Argon2 / HKDF failure (should never happen with valid params).
    #[error("key derivation failed")]
    Kdf,
    /// AEAD authentication failed (wrong key or tampered ciphertext).
    #[error("decryption failed")]
    Decrypt,
    /// Bytes could not be parsed as a protocol message.
    #[error("malformed message")]
    Malformed,
    /// Payload exceeds the largest padding bucket.
    #[error("message too large")]
    TooLarge,
    /// Ed25519 signature invalid.
    #[error("invalid signature")]
    BadSignature,
    /// Message refers to a sender or key we do not have.
    #[error("unknown sender or key")]
    UnknownKey,
    /// Blob comes from an older, incompatible GhostLine version.
    #[error("message from an older GhostLine version")]
    OldVersion,
    /// libsignal reported an error.
    #[error("signal protocol error: {0}")]
    Signal(String),
}

impl From<libsignal_protocol::SignalProtocolError> for Error {
    fn from(e: libsignal_protocol::SignalProtocolError) -> Self {
        Error::Signal(e.to_string())
    }
}
