//! Scrubbing of secrets and personal data from text that leaves the process
//! (issue titles/details, log views, report bundles).
//!
//! - hex runs of ≥ 32 chars (bridge tokens, keys) → `[redacted]`
//! - `Bearer <value>` → `Bearer [redacted]`
//! - phone numbers / JID users (11–15 digits) → `91•••••1234` (last 4 kept)
//! - paths under `$HOME` → `~/…`

use std::sync::LazyLock;

use regex::{Captures, Regex};

// The patterns are literals that are known to compile; a failure here is a
// programming error caught by the unit tests.
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(bearer\s+)[A-Za-z0-9._~+/=\-]+").expect("static regex"));
static HEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[0-9a-fA-F]{32,}\b").expect("static regex"));
static PHONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{11,15})\b").expect("static regex"));

/// Mask a digit string, keeping the first 2 and last 4 digits.
pub fn mask_digits(d: &str) -> String {
    let n = d.chars().count();
    if n <= 6 {
        return "•".repeat(n);
    }
    let head: String = d.chars().take(2).collect();
    let tail: String = d.chars().skip(n - 4).collect();
    format!("{head}{}{tail}", "•".repeat(n - 6))
}

/// Redact using the current user's home directory.
pub fn redact(s: &str) -> String {
    let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned());
    redact_with_home(s, home.as_deref())
}

/// Redact with an explicit home directory (None = leave paths alone).
pub fn redact_with_home(s: &str, home: Option<&str>) -> String {
    let s = BEARER.replace_all(s, "${1}[redacted]");
    let s = HEX.replace_all(&s, "[redacted]");
    let s = PHONE.replace_all(&s, |c: &Captures| mask_digits(&c[1]));
    match home {
        Some(h) if h.len() > 1 => replace_home(&s, h.trim_end_matches('/')),
        _ => s.into_owned(),
    }
}

fn replace_home(s: &str, home: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(home) {
        let after = &rest[i + home.len()..];
        let boundary = after
            .chars()
            .next()
            .is_none_or(|c| c == '/' || !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'));
        out.push_str(&rest[..i]);
        out.push_str(if boundary { "~" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Truncate to `max` chars (appending "…" when cut).
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> String {
        redact_with_home(s, Some("/home/rishi"))
    }

    #[test]
    fn hex_tokens() {
        let tok = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(r(&format!("token={tok} end")), "token=[redacted] end");
        // Short hex (ids, hashes prefixes) stays.
        assert_eq!(r("id 3EB0ABC12"), "id 3EB0ABC12");
    }

    #[test]
    fn bearer_values() {
        assert_eq!(
            r("Authorization: Bearer abc.def-123"),
            "Authorization: Bearer [redacted]"
        );
        assert_eq!(r("bearer xyz"), "bearer [redacted]");
    }

    #[test]
    fn phones_and_jids() {
        assert_eq!(r("from 919811111234"), "from 91••••••1234");
        assert_eq!(
            r("91981111234@s.whatsapp.net"),
            "91•••••1234@s.whatsapp.net"
        );
        assert_eq!(
            r("919811111234:5@s.whatsapp.net"),
            "91••••••1234:5@s.whatsapp.net"
        );
        assert_eq!(r("lid 222222222222222@lid"), "lid 22•••••••••2222@lid");
        // Epoch seconds and group ids are not phone numbers.
        assert_eq!(r("ts=1794764079"), "ts=1794764079");
        assert_eq!(r("120363000000000001@g.us"), "120363000000000001@g.us");
    }

    #[test]
    fn home_paths() {
        assert_eq!(
            r("open /home/rishi/Documents/a.pdf"),
            "open ~/Documents/a.pdf"
        );
        assert_eq!(r("/home/rishi"), "~");
        assert_eq!(r("/home/rishikesh/x"), "/home/rishikesh/x");
        assert_eq!(redact_with_home("/home/rishi/x", None), "/home/rishi/x");
    }

    #[test]
    fn masking_short_and_unicode_safe() {
        assert_eq!(mask_digits("1234"), "••••");
        assert_eq!(truncate("नमस्ते", 2).chars().count(), 3);
        assert_eq!(r("नमस्ते 919811111234"), "नमस्ते 91••••••1234");
    }
}
