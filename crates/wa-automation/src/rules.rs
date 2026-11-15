//! Rule validation, matching checks and reply templating (pure functions).

use std::sync::LazyLock;

use chrono::{Local, NaiveTime, TimeZone, Timelike};
use regex::{Regex, RegexBuilder};
use wa_core::types::{Action, Message, Rule};
use wa_core::{AppError, Result};

/// Longest accepted regex pattern (characters).
pub const MAX_REGEX_LEN: usize = 500;
/// Compiled-size cap for rule regexes.
pub const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// Longest reply text (characters).
pub const MAX_REPLY: usize = 4096;
const MAX_ACTIONS: usize = 10;
/// Default per-chat cooldown when a rule has 0.
pub const DEFAULT_COOLDOWN: u32 = 600;

// Literal patterns, covered by tests.
static JID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(\+?\d{7,15}|[0-9][0-9\-]{4,40}(:\d+)?@(s\.whatsapp\.net|g\.us|lid|c\.us|newsletter))$",
    )
    .expect("static regex")
});

/// Compile a rule regex with size limits.
pub fn compile_regex(p: &str) -> Result<Regex> {
    if p.chars().count() > MAX_REGEX_LEN {
        return Err(AppError::invalid(format!(
            "regex is longer than {MAX_REGEX_LEN} characters"
        )));
    }
    RegexBuilder::new(p)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| AppError::invalid("invalid regex").with_detail(e.to_string()))
}

/// Parse strict "HH:MM".
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b':' {
        return None;
    }
    let h: u32 = s[..2].parse().ok()?;
    let m: u32 = s[3..].parse().ok()?;
    NaiveTime::from_hms_opt(h, m, 0)
}

/// True if `s` looks like a chat JID or phone number.
pub fn looks_like_jid(s: &str) -> bool {
    JID.is_match(s.trim())
}

/// Reject rules that could misbehave (bad regex/time/URL, empty actions, …).
pub fn validate_rule(rule: &Rule) -> Result<()> {
    let name = rule.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::invalid("rule name must be 1–100 characters"));
    }
    if rule.actions.is_empty() {
        return Err(AppError::invalid("a rule needs at least one action"));
    }
    if rule.actions.len() > MAX_ACTIONS {
        return Err(AppError::invalid("a rule may have at most 10 actions"));
    }
    if rule.cooldown_secs > 7 * 86_400 {
        return Err(AppError::invalid("cooldown must be at most 7 days"));
    }
    let t = &rule.trigger;
    if let Some(r) = &t.regex {
        compile_regex(r)?;
    }
    match (&t.active_from, &t.active_to) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            if parse_hhmm(a).is_none() || parse_hhmm(b).is_none() {
                return Err(AppError::invalid("active times must be HH:MM"));
            }
        }
        _ => return Err(AppError::invalid("set both active times or neither")),
    }
    if t.keywords.iter().any(|k| k.trim().is_empty()) {
        return Err(AppError::invalid("keywords must not be empty"));
    }
    if let Some(bad) = t.chat_jids.iter().find(|j| !looks_like_jid(j)) {
        return Err(AppError::invalid(format!("not a chat id: {bad}")));
    }
    if t.include_groups && !t.chat_jids.iter().any(|j| j.ends_with("@g.us")) {
        return Err(AppError::invalid(
            "list the groups this rule may run in (include groups needs explicit groups)",
        ));
    }
    for a in &rule.actions {
        validate_action(a)?;
    }
    Ok(())
}

fn validate_action(a: &Action) -> Result<()> {
    match a {
        Action::Reply { text } => {
            if text.trim().is_empty() {
                return Err(AppError::invalid("reply text must not be empty"));
            }
            if text.chars().count() > MAX_REPLY {
                return Err(AppError::invalid(
                    "reply text is longer than 4096 characters",
                ));
            }
        }
        Action::Forward { to_jid } => {
            if !looks_like_jid(to_jid) {
                return Err(AppError::invalid(
                    "forward target must be a phone number or chat id",
                ));
            }
        }
        Action::Webhook { url } => {
            let u = reqwest::Url::parse(url)
                .map_err(|_| AppError::invalid("webhook URL is not valid"))?;
            if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
                return Err(AppError::invalid("webhook URL must be http(s)"));
            }
        }
        Action::Tag { label } => {
            if label.trim().is_empty() || label.chars().count() > 64 {
                return Err(AppError::invalid("tag must be 1–64 characters"));
            }
        }
        Action::DownloadMedia | Action::Notify { .. } => {}
    }
    Ok(())
}

/// User part of a JID: digits before ':' / '@', without a leading '+'.
pub fn user_part(jid: &str) -> &str {
    let u = jid.split('@').next().unwrap_or(jid);
    let u = u.split(':').next().unwrap_or(u);
    u.trim_start_matches('+')
}

/// Local HH:MM of an epoch.
pub fn local_hhmm(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn in_window(ts: i64, from: NaiveTime, to: NaiveTime) -> bool {
    let Some(t) = Local.timestamp_opt(ts, 0).single() else {
        return false;
    };
    let now = NaiveTime::from_hms_opt(t.hour(), t.minute(), 0).unwrap_or_default();
    if from <= to {
        now >= from && now <= to
    } else {
        now >= from || now <= to
    }
}

/// The message-dependent context a rule is checked against.
pub struct MatchCtx<'a> {
    pub message: &'a Message,
    pub is_group: bool,
    /// Our own user parts (phone + LID) for @mention detection.
    pub me: &'a [String],
}

/// True if the message @mentions one of our user parts.
pub fn mentions_me(content: &str, me: &[String]) -> bool {
    me.iter()
        .filter(|m| !m.is_empty())
        .any(|m| content.contains(&format!("@{m}")))
}

/// Run every trigger check. Returns (matched, reasons).
pub fn check(rule: &Rule, ctx: &MatchCtx<'_>, regex: Option<&Regex>) -> (bool, Vec<String>) {
    let t = &rule.trigger;
    let m = ctx.message;
    let mut ok = true;
    let mut reasons = Vec::new();
    let mut push = |pass: bool, text: String| {
        ok &= pass;
        reasons.push(format!("{} {text}", if pass { "✓" } else { "✗" }));
    };

    if ctx.is_group {
        let listed = t.chat_jids.iter().any(|j| j == &m.chat_jid);
        push(
            t.include_groups && listed,
            if t.include_groups && listed {
                "chat: group is listed".into()
            } else {
                "chat: groups need 'include groups' and the group listed".into()
            },
        );
    } else if t.chat_jids.is_empty() {
        push(true, "chat: any direct chat".into());
    } else {
        let listed = t
            .chat_jids
            .iter()
            .any(|j| user_part(j) == user_part(&m.chat_jid));
        push(
            listed,
            format!(
                "chat: {}",
                if listed {
                    "listed"
                } else {
                    "not in the rule's chats"
                }
            ),
        );
    }

    if !t.senders.is_empty() {
        let s = m.sender_jid.as_deref().map(user_part).unwrap_or_default();
        let hit = !s.is_empty() && t.senders.iter().any(|x| user_part(x) == s);
        push(
            hit,
            format!(
                "sender: {}",
                if hit { "matches" } else { "not in the list" }
            ),
        );
    }

    if !t.keywords.is_empty() {
        let c = m.content.to_lowercase();
        let hit = t.keywords.iter().find(|k| c.contains(&k.to_lowercase()));
        match hit {
            Some(k) => push(true, format!("keyword: found \"{k}\"")),
            None => push(false, "keyword: none found".into()),
        }
    }

    if t.regex.is_some() {
        match regex {
            Some(r) => {
                let hit = r.is_match(&m.content);
                push(
                    hit,
                    format!("regex: {}", if hit { "matches" } else { "no match" }),
                );
            }
            None => push(false, "regex: invalid".into()),
        }
    }

    if !t.media_kinds.is_empty() {
        let k = m.media.as_ref().map(|x| x.kind);
        let hit = k.is_some_and(|k| t.media_kinds.contains(&k));
        push(
            hit,
            format!(
                "media: {}",
                if hit {
                    "kind matches"
                } else {
                    "kind not selected"
                }
            ),
        );
    }

    if let (Some(a), Some(b)) = (&t.active_from, &t.active_to) {
        match (parse_hhmm(a), parse_hhmm(b)) {
            (Some(f), Some(to)) => {
                let hit = in_window(m.ts, f, to);
                push(hit, format!("time: {} {}–{}", local_hhmm(m.ts), a, b));
            }
            _ => push(false, "time: invalid window".into()),
        }
    }

    let has_reply = rule
        .actions
        .iter()
        .any(|a| matches!(a, Action::Reply { .. }));
    if ctx.is_group && (t.mention_only || has_reply) {
        let hit = mentions_me(&m.content, ctx.me);
        push(
            hit,
            format!(
                "mention: {}",
                if hit {
                    "we are @mentioned"
                } else {
                    "we are not @mentioned"
                }
            ),
        );
    }
    (ok, reasons)
}

/// Fill `{sender_name}`, `{chat_name}`, `{time}`, `{text}` in one pass
/// (substituted values are never re-expanded).
pub fn render(template: &str, m: &Message, chat_name: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let sub = [
            ("{sender_name}", m.sender_name.clone()),
            ("{chat_name}", chat_name.to_string()),
            ("{time}", local_hhmm(m.ts)),
            ("{text}", m.content.clone()),
        ]
        .into_iter()
        .find(|(k, _)| after.starts_with(k));
        match sub {
            Some((k, v)) => {
                out.push_str(&v);
                rest = &after[k.len()..];
            }
            None => {
                out.push('{');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One-line description of an action (for `planned` / dry-run outcomes).
pub fn describe(a: &Action, m: &Message, chat_name: &str) -> String {
    match a {
        Action::Reply { text } => {
            format!("reply \"{}\"", truncate(&render(text, m, chat_name), 80))
        }
        Action::DownloadMedia => "download the media".into(),
        Action::Notify { title } => {
            format!("notify \"{}\"", title.as_deref().unwrap_or("rule name"))
        }
        Action::Forward { to_jid } => format!("forward to {to_jid}"),
        Action::Tag { label } => format!("tag '{label}'"),
        Action::Webhook { url } => format!("POST webhook {}", url_host(url)),
    }
}

fn url_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "?".into())
}

/// Truncate to `max` chars with "…".
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
    fn hhmm_strict() {
        assert!(parse_hhmm("09:30").is_some());
        assert!(parse_hhmm("23:59").is_some());
        for bad in ["9:30", "24:00", "12:60", "12-30", "ab:cd", "12:300", ""] {
            assert!(parse_hhmm(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn jid_shapes() {
        for ok in [
            "919811111111",
            "+919811111111",
            "919811111111@s.whatsapp.net",
            "919811111111:5@s.whatsapp.net",
            "120363000000000001@g.us",
            "222222222222222@lid",
        ] {
            assert!(looks_like_jid(ok), "{ok}");
        }
        for bad in ["alice", "12", "a@b.c", "91981@evil.com", "", "@g.us"] {
            assert!(!looks_like_jid(bad), "{bad}");
        }
    }

    #[test]
    fn user_parts() {
        assert_eq!(user_part("919811111111:5@s.whatsapp.net"), "919811111111");
        assert_eq!(user_part("+9198"), "9198");
        assert_eq!(user_part("x@lid"), "x");
    }

    #[test]
    fn regex_limits() {
        assert!(compile_regex("(?i)invoice #\\d+").is_ok());
        assert!(compile_regex("(").is_err());
        assert!(compile_regex(&"a".repeat(MAX_REGEX_LEN + 1)).is_err());
        assert!(compile_regex("(\\w{1,1000}){1000}").is_err(), "size limit");
    }

    #[test]
    fn wrapping_window() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let now = Local::now();
        let ts = now.timestamp();
        let h = now.hour();
        assert!(in_window(ts, t(h, 0), t(h, 59)));
        assert!(
            in_window(ts, t((h + 1) % 24, 0), t(h, 59)),
            "wraps midnight"
        );
        assert!(!in_window(ts, t((h + 1) % 24, 0), t((h + 1) % 24, 59)));
    }
}
