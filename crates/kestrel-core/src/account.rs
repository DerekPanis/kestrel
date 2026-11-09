//! PSN Account ID handling.
//!
//! PS5 pairing needs the 8-byte PSN *account* ID (not the visible Online ID).
//! Users usually have it as base64 (what chiaki-ng and the common lookup
//! tools show); Sony's API returns it as a decimal user_id, which is encoded
//! as 8 little-endian bytes — the same convention as chiaki-ng's
//! scripts/psn-account-id.py.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountId(pub [u8; 8]);

impl AccountId {
    /// Accepts either the base64 form (12 chars, e.g. `AbCdEfGhIjI=`) or the
    /// decimal user_id returned by Sony's account API.
    pub fn parse(input: &str) -> Result<Self> {
        let s = input.trim();
        if s.is_empty() {
            return Err(Error::InvalidInput("PSN Account ID is empty".into()));
        }
        if s.chars().all(|c| c.is_ascii_digit()) && s.len() > 12 {
            let id: u64 = s.parse().map_err(|_| {
                Error::InvalidInput("decimal PSN Account ID is out of range".into())
            })?;
            return Ok(AccountId(id.to_le_bytes()));
        }
        let bytes = STANDARD
            .decode(s)
            .map_err(|_| Error::InvalidInput("PSN Account ID is not valid base64".into()))?;
        let arr: [u8; 8] = bytes.try_into().map_err(|v: Vec<u8>| {
            Error::InvalidInput(format!(
                "PSN Account ID must decode to 8 bytes, got {}",
                v.len()
            ))
        })?;
        Ok(AccountId(arr))
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0)
    }
}

// The account ID is not a password, but it is personal; keep it out of logs.
impl std::fmt::Debug for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountId(<redacted>)")
    }
}

/// Parses the 8-digit Link Device PIN shown on the PS5.
pub fn parse_link_pin(input: &str) -> Result<u32> {
    let digits: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if digits.len() != 8 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::InvalidInput(
            "the Link Device PIN must be exactly 8 digits".into(),
        ));
    }
    digits
        .parse()
        .map_err(|_| Error::InvalidInput("invalid PIN".into()))
}

/// Parses the optional 4-digit PS5 login passcode. Empty → 0 (none).
pub fn parse_console_pin(input: &str) -> Result<u32> {
    let s = input.trim();
    if s.is_empty() {
        return Ok(0);
    }
    if s.len() != 4 || !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::InvalidInput(
            "the console passcode must be 4 digits".into(),
        ));
    }
    s.parse()
        .map_err(|_| Error::InvalidInput("invalid passcode".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        let id = AccountId([1, 2, 3, 4, 5, 6, 7, 8]);
        let b = id.to_base64();
        assert_eq!(b, "AQIDBAUGBwg=");
        assert_eq!(AccountId::parse(&b).unwrap(), id);
    }

    #[test]
    fn decimal_is_little_endian() {
        // Same result as chiaki-ng's psn-account-id.py: user_id.to_bytes(8, "little")
        let id = AccountId::parse("1234567890123456789").unwrap();
        assert_eq!(id.0, 1234567890123456789u64.to_le_bytes());
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(AccountId::parse("AQID").is_err());
        assert!(AccountId::parse("").is_err());
        assert!(AccountId::parse("not base64!!").is_err());
    }

    #[test]
    fn debug_is_redacted() {
        let id = AccountId([9; 8]);
        assert!(!format!("{id:?}").contains('9'));
    }

    #[test]
    fn pins() {
        assert_eq!(parse_link_pin("1234 5678").unwrap(), 12345678);
        assert_eq!(parse_link_pin("0001-0002").unwrap(), 10002);
        assert!(parse_link_pin("1234567").is_err());
        assert!(parse_link_pin("abcdefgh").is_err());
        assert_eq!(parse_console_pin("").unwrap(), 0);
        assert_eq!(parse_console_pin("0420").unwrap(), 420);
        assert!(parse_console_pin("12345").is_err());
    }
}
