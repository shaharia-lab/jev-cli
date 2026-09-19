//! The API key, held so that it is hard to leak by accident.

use std::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop};

/// What is shown in place of a secret wherever one could otherwise be printed.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// A TypeSafe API key.
///
/// The key is wiped from memory when the value is dropped, and nothing in this crate prints it:
/// `Debug` shows a placeholder, and there is deliberately no `Display`, `Serialize` or `PartialEq`.
/// The only way to read it is [`ApiKey::expose`], which keeps every use easy to find and review.
///
/// Wiping is best effort. The HTTP stack keeps its own copy of the `Authorization` header while a
/// transport is alive, and that copy is outside this type's control.
///
/// ```
/// use jev_client::ApiKey;
///
/// let key = ApiKey::new(String::from("example-key\n")).unwrap();
/// assert_eq!(key.expose(), "example-key");
/// assert_eq!(format!("{key:?}"), "ApiKey([REDACTED])");
/// ```
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key, trimming surrounding whitespace such as the newline a file or a pipe leaves.
    ///
    /// The string is taken by value so that the caller's copy is consumed; the original buffer is
    /// wiped before this returns.
    ///
    /// # Errors
    ///
    /// Fails when the key is empty, or contains anything other than visible ASCII characters,
    /// which could not be sent in an HTTP header. The error never contains the key.
    pub fn new(mut key: String) -> Result<Self, InvalidApiKey> {
        let trimmed = key.trim().to_owned();
        key.zeroize();

        let mut candidate = Self(trimmed);
        if candidate.0.is_empty() {
            return Err(InvalidApiKey::Empty);
        }
        if !candidate.0.bytes().all(|byte| byte.is_ascii_graphic()) {
            candidate.zeroize();
            return Err(InvalidApiKey::InvalidCharacters);
        }
        Ok(candidate)
    }

    /// The key itself. Call this only at the point where the key is handed to the HTTP stack or
    /// to a credential store, and never pass the result to a log, an error or a formatter.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Replaces every occurrence of the key in `text` with a placeholder.
    ///
    /// Used on text that came from the server, which could echo a header back in an error message.
    pub(crate) fn scrub(&self, text: &str) -> String {
        text.replace(self.0.as_str(), REDACTED)
    }

    /// [`ApiKey::scrub`] for a body that may not be UTF-8. Bytes are returned unchanged unless the
    /// key is among them.
    pub(crate) fn scrub_bytes(&self, bytes: Vec<u8>) -> Vec<u8> {
        let key = self.0.as_bytes();
        if !bytes.windows(key.len()).any(|window| window == key) {
            return bytes;
        }
        let mut scrubbed = Vec::with_capacity(bytes.len());
        let mut rest = bytes.as_slice();
        while !rest.is_empty() {
            if rest.starts_with(key) {
                scrubbed.extend_from_slice(REDACTED.as_bytes());
                rest = rest.get(key.len()..).unwrap_or_default();
            } else if let Some((first, tail)) = rest.split_first() {
                scrubbed.push(*first);
                rest = tail;
            }
        }
        scrubbed
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ApiKey({REDACTED})")
    }
}

/// Why a string was rejected as an API key. The message never contains the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum InvalidApiKey {
    /// The key was empty, or only whitespace.
    #[error("the API key is empty")]
    Empty,
    /// The key held a space, a control character or a non-ASCII character.
    #[error("the API key contains characters that cannot be sent in an HTTP header")]
    InvalidCharacters,
}

#[cfg(test)]
mod tests {
    use super::{ApiKey, InvalidApiKey};

    const SENTINEL: &str = "sentinel-key-0123456789";

    #[test]
    fn trims_the_whitespace_a_file_or_pipe_leaves_behind() {
        let key = ApiKey::new(format!("  {SENTINEL}\r\n")).unwrap();

        assert_eq!(key.expose(), SENTINEL);
    }

    #[test]
    fn debug_never_shows_the_key() {
        let key = ApiKey::new(SENTINEL.to_owned()).unwrap();

        for rendered in [
            format!("{key:?}"),
            format!("{key:#?}"),
            format!("{:?}", Some(&key)),
        ] {
            assert!(!rendered.contains(SENTINEL), "{rendered}");
            assert!(rendered.contains("[REDACTED]"), "{rendered}");
        }
    }

    #[test]
    fn rejects_keys_that_cannot_be_sent_without_echoing_them() {
        let cases = [
            ("", InvalidApiKey::Empty),
            (" \n\t", InvalidApiKey::Empty),
            ("sentinel key", InvalidApiKey::InvalidCharacters),
            ("sentinel\nkey", InvalidApiKey::InvalidCharacters),
            ("sentinel-ключ", InvalidApiKey::InvalidCharacters),
        ];

        for (input, expected) in cases {
            let error = ApiKey::new(input.to_owned()).unwrap_err();

            assert_eq!(error, expected, "{input:?}");
            assert!(!error.to_string().contains("sentinel"), "{error}");
            assert!(!format!("{error:?}").contains("sentinel"), "{error:?}");
        }
    }

    #[test]
    fn scrubs_the_key_out_of_text_that_echoes_it() {
        let key = ApiKey::new(SENTINEL.to_owned()).unwrap();

        let scrubbed = key.scrub(&format!("bad header: Bearer {SENTINEL} ({SENTINEL})"));

        assert_eq!(scrubbed, "bad header: Bearer [REDACTED] ([REDACTED])");
    }

    #[test]
    fn scrubs_the_key_out_of_bytes_and_leaves_other_bytes_alone() {
        let key = ApiKey::new(SENTINEL.to_owned()).unwrap();
        let mut body = format!("{{\"echo\":\"Bearer {SENTINEL}\",\"k\":\"{SENTINEL}").into_bytes();
        body.extend_from_slice(b"\xff\"}");

        let scrubbed = key.scrub_bytes(body);

        assert_eq!(
            scrubbed,
            b"{\"echo\":\"Bearer [REDACTED]\",\"k\":\"[REDACTED]\xff\"}".to_vec()
        );
        let untouched = b"no key here \xfe".to_vec();
        assert_eq!(key.scrub_bytes(untouched.clone()), untouched);
    }
}
