//! Session token: 128 random bits, required on every API call.

use anyhow::Result;

/// 16 random bytes as 32 lowercase hex chars. Runtime only: never stored in the config.
pub fn new_token() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow::anyhow!("random token: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Constant-time comparison (no early exit on the first differing byte).
pub fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false; // the length (always 32) is not secret
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Value of `key` in a raw query string (`a=1&t=abc`). No percent-decoding: tokens are hex.
pub fn query_param<'a>(query: Option<&'a str>, key: &str) -> Option<&'a str> {
    query?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then_some(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_128_bit_hex_and_random() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    #[test]
    fn token_eq_matches_only_exact() {
        assert!(token_eq("abcd", "abcd"));
        assert!(!token_eq("abcd", "abce"));
        assert!(!token_eq("abcd", "abc"));
        assert!(!token_eq("abc", "abcd"));
        assert!(!token_eq("", "abcd"));
        assert!(!token_eq("abcd", ""));
    }

    #[test]
    fn query_param_finds_key() {
        assert_eq!(query_param(None, "t"), None);
        assert_eq!(query_param(Some(""), "t"), None);
        assert_eq!(query_param(Some("t=abc"), "t"), Some("abc"));
        assert_eq!(query_param(Some("x=1&t=abc&y=2"), "t"), Some("abc"));
        assert_eq!(query_param(Some("tt=abc"), "t"), None);
        assert_eq!(query_param(Some("t"), "t"), Some(""));
    }
}
