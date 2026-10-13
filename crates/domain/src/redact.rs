//! Best-effort secret redaction for free text.
//!
//! Free text is a common place for a credential to leak by accident: a command
//! echoes an environment value, an HTTP client logs a URL with embedded
//! userinfo, a stack trace prints an `Authorization` header. This module applies
//! a conservative, pattern-based scrub before text is stored, returned to an
//! interface, or surfaced in an error (`AGENTS.md` §31).
//!
//! It lives in the domain because the "secrets must never be exposed" rule is a
//! domain invariant (`AGENTS.md` §31) shared by every layer: the log hub, the
//! build and migration adapters, and the process description all reuse it rather
//! than each carrying its own copy.
//!
//! # It is not perfect, and does not pretend to be
//!
//! There is no reliable way to detect every secret in free text. Redaction
//! therefore targets *obvious* shapes — secret-looking `key=value` pairs,
//! credential-bearing URLs, and `Bearer`/`Basic` scheme tokens — and the system
//! additionally avoids logging sensitive inputs in the first place.

use crate::environment::REDACTED;

/// Substrings that mark a key as secret-bearing.
const SECRET_KEY_MARKERS: [&str; 15] = [
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "access_key",
    "accesskey",
    "private_key",
    "privatekey",
    "credential",
    "authorization",
    "bearer",
];

/// Redacts obvious secret material from a message.
///
/// Whitespace between words is preserved; only the suspicious values are
/// replaced with [`REDACTED`](crate::REDACTED).
#[must_use]
pub fn redact(message: &str) -> String {
    let mut result = String::with_capacity(message.len());
    let mut word = String::new();
    let mut mask_next = false;

    for ch in message.chars() {
        if ch.is_whitespace() {
            if !word.is_empty() {
                append_word(&mut result, &word, mask_next);
                mask_next = is_scheme_keyword(&word);
                word.clear();
            }
            result.push(ch);
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        append_word(&mut result, &word, mask_next);
    }
    result
}

/// Returns `true` when a message contains nothing that redaction would change.
#[must_use]
pub fn is_clean(message: &str) -> bool {
    redact(message) == message
}

fn append_word(out: &mut String, word: &str, mask: bool) {
    if mask {
        out.push_str(REDACTED);
        return;
    }
    out.push_str(&redact_word(word));
}

/// Redacts a single whitespace-delimited token.
fn redact_word(word: &str) -> String {
    if let Some(masked) = mask_assignment(word) {
        return masked;
    }
    if let Some(masked) = mask_url_userinfo(word) {
        return masked;
    }
    word.to_owned()
}

/// Masks the value of a `key=value` pair when the key looks secret-bearing.
fn mask_assignment(word: &str) -> Option<String> {
    let (key, value) = word.split_once('=')?;
    if value.is_empty() {
        return None;
    }
    let normalized = key
        .trim_start_matches('-')
        .trim_matches(|c| c == '"' || c == '\'' || c == '{')
        .to_ascii_lowercase();
    if normalized.is_empty() || !is_secret_key(&normalized) {
        return None;
    }
    Some(format!("{key}={REDACTED}"))
}

/// Masks the userinfo of a URL (`scheme://user:pass@host`).
fn mask_url_userinfo(word: &str) -> Option<String> {
    let scheme_end = word.find("://")?;
    let rest = &word[scheme_end + 3..];
    let at = rest.find('@')?;
    Some(format!(
        "{}{}@{}",
        &word[..scheme_end + 3],
        REDACTED,
        &rest[at + 1..]
    ))
}

/// Returns `true` when a key name indicates a secret value.
fn is_secret_key(key: &str) -> bool {
    SECRET_KEY_MARKERS.iter().any(|marker| key.contains(marker))
}

/// Returns `true` when a word introduces a credential-bearing scheme token.
fn is_scheme_keyword(word: &str) -> bool {
    let normalized = word.trim_end_matches(':').to_ascii_lowercase();
    normalized == "bearer" || normalized == "basic"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_secret_assignments() {
        assert_eq!(
            redact("DB_PASSWORD=hunter2"),
            format!("DB_PASSWORD={REDACTED}")
        );
        assert_eq!(
            redact("export API_KEY=abc123"),
            format!("export API_KEY={REDACTED}")
        );
        assert_eq!(
            redact("--access-token=deadbeef"),
            format!("--access-token={REDACTED}")
        );
    }

    #[test]
    fn preserves_non_secret_assignments() {
        assert_eq!(redact("APP_ENV=local PORT=8081"), "APP_ENV=local PORT=8081");
    }

    #[test]
    fn masks_url_userinfo() {
        let masked = redact("cloning https://user:s3cr3t@github.com/acme/repo.git");
        assert!(masked.contains(&format!("https://{REDACTED}@github.com")));
        assert!(!masked.contains("s3cr3t"));
    }

    #[test]
    fn masks_bearer_tokens() {
        assert_eq!(
            redact("Authorization: Bearer abcdef.token.value"),
            format!("Authorization: Bearer {REDACTED}")
        );
    }

    #[test]
    fn preserves_whitespace_and_plain_text() {
        assert_eq!(redact("a  b\tc"), "a  b\tc");
        assert!(is_clean("nothing to hide here"));
        assert!(!is_clean("password=hunter2"));
    }
}
