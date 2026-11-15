//! Minimal scrubbing for bridge error bodies (no regex dependency).

/// Maximum characters of a bridge error body kept in an error detail.
pub const MAX_BODY: usize = 500;

/// Replace the token itself and any hex run of ≥ 32 chars, then truncate.
pub fn scrub(body: &str, token: Option<&str>) -> String {
    let mut s = body.to_string();
    if let Some(t) = token.filter(|t| t.len() >= 8) {
        s = s.replace(t, "[redacted]");
    }
    let s = mask_hex_runs(&s);
    truncate(s.trim(), MAX_BODY)
}

fn mask_hex_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 32 {
            out.push_str("[redacted]");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if c.is_ascii_hexdigit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Truncate to `max` chars, appending "…" when cut.
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

    #[test]
    fn scrubs_token_and_hex() {
        let tok = "abcdef0123456789abcdef0123456789";
        let s = scrub(&format!("bad {tok} and {}", "f".repeat(40)), Some(tok));
        assert!(!s.contains(tok));
        assert!(!s.contains(&"f".repeat(40)));
        assert_eq!(scrub("deadbeef ok", None), "deadbeef ok");
    }

    #[test]
    fn truncates_long_bodies() {
        let s = scrub(&"x".repeat(2000), None);
        assert_eq!(s.chars().count(), MAX_BODY + 1);
        assert!(s.ends_with('…'));
    }
}
