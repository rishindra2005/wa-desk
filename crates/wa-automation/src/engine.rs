//! The automation engine: guards, rate limits and action execution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use regex::Regex;
use serde_json::json;
use wa_core::config::Paths;
use wa_core::time::now_epoch;
use wa_core::traits::{
    AppStore, BridgeApi, BridgeSend, IncomingMessage, IssueSink, MessageStore, Notifier,
};
use wa_core::types::{Action, AutomationEvent, Message, Rule, RuleEval, Settings};
use wa_core::{AppError, ErrorKind, Result};

use crate::rules::{
    DEFAULT_COOLDOWN, MatchCtx, check, compile_regex, describe, render, truncate, user_part,
    validate_rule,
};

/// kv key holding the last processed rowid.
pub const WATERMARK_KEY: &str = "automation.watermark";
/// Messages older than this (seconds) never fire rules.
pub const MAX_AGE_SECS: i64 = 600;
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(5);

/// Clock returning Unix seconds (injectable for tests).
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Token bucket refilled continuously at `cap` tokens per hour.
#[derive(Debug, Clone)]
struct Bucket {
    cap: u32,
    tokens: f64,
    last: i64,
}

impl Bucket {
    fn take(&mut self, cap: u32, now: i64) -> bool {
        if cap != self.cap {
            self.tokens = self.tokens.min(f64::from(cap));
            self.cap = cap;
        }
        let elapsed = (now - self.last).max(0) as f64;
        self.tokens = (self.tokens + elapsed * f64::from(cap) / 3600.0).min(f64::from(cap));
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Default)]
struct RateState {
    /// (rule id, chat) → last firing.
    cooldowns: HashMap<(i64, String), i64>,
    bucket: Option<Bucket>,
}

/// Evaluates incoming messages against the saved rules.
pub struct Engine {
    store: Arc<dyn AppStore>,
    bridge: Arc<dyn BridgeApi>,
    notifier: Arc<dyn Notifier>,
    me: Vec<String>,
    issues: Option<Arc<dyn IssueSink>>,
    messages: Option<Arc<dyn MessageStore>>,
    paths: Option<Paths>,
    http: reqwest::Client,
    clock: Clock,
    rate: Mutex<RateState>,
    regexes: Mutex<HashMap<String, Regex>>,
}

impl Engine {
    /// `me`: our own phone and LID user parts (e.g. `["919900000000", "1111…"]`).
    pub fn new(
        store: Arc<dyn AppStore>,
        bridge: Arc<dyn BridgeApi>,
        notifier: Arc<dyn Notifier>,
        me: Vec<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(WEBHOOK_TIMEOUT)
            .connect_timeout(WEBHOOK_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            store,
            bridge,
            notifier,
            me: me
                .iter()
                .map(|m| user_part(m).to_string())
                .filter(|m| !m.is_empty())
                .collect(),
            issues: None,
            messages: None,
            paths: None,
            http,
            clock: Arc::new(now_epoch),
            rate: Mutex::new(RateState::default()),
            regexes: Mutex::new(HashMap::new()),
        }
    }

    /// Record action failures as issues.
    pub fn with_issues(mut self, sink: Arc<dyn IssueSink>) -> Self {
        self.issues = Some(sink);
        self
    }

    /// Needed to forward media (local path lookup) …
    pub fn with_messages(mut self, m: Arc<dyn MessageStore>) -> Self {
        self.messages = Some(m);
        self
    }

    /// … together with the paths (outbox staging).
    pub fn with_paths(mut self, p: Paths) -> Self {
        self.paths = Some(p);
        self
    }

    /// Override the clock (tests).
    pub fn with_clock(mut self, c: Clock) -> Self {
        self.clock = c;
        self
    }

    /// Start firing only for rows after `current_max_rowid`; resets rate state.
    pub fn enable(&self, current_max_rowid: i64) -> Result<()> {
        self.store
            .set_kv(WATERMARK_KEY, &current_max_rowid.to_string())?;
        *self.rate.lock() = RateState::default();
        Ok(())
    }

    /// Last processed rowid, if set.
    pub fn watermark(&self) -> Option<i64> {
        self.store
            .get_kv(WATERMARK_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse().ok())
    }

    /// Validate a rule before saving.
    pub fn validate_rule(&self, rule: &Rule) -> Result<()> {
        validate_rule(rule)
    }

    fn regex_for(&self, rule: &Rule) -> Option<Regex> {
        let p = rule.trigger.regex.as_ref()?;
        let mut cache = self.regexes.lock();
        if let Some(r) = cache.get(p) {
            return Some(r.clone());
        }
        let r = compile_regex(p).ok()?;
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(p.clone(), r.clone());
        Some(r)
    }

    /// Reason a message is never automated, if any.
    fn guard(&self, m: &Message, is_status: bool, raw_media_type: &str) -> Option<&'static str> {
        if m.is_from_me {
            return Some("own message");
        }
        if m.sender_jid
            .as_deref()
            .is_some_and(|s| self.me.iter().any(|me| me == user_part(s)))
        {
            return Some("sent by our own account");
        }
        if raw_media_type == "reaction" {
            return Some("reaction");
        }
        if m.deleted {
            return Some("deleted message");
        }
        if is_status || m.chat_jid == "status@broadcast" {
            return Some("status update");
        }
        None
    }

    /// Dry evaluation for the rule editor (no watermark/age/rate checks).
    pub fn test_rule(&self, rule: &Rule, sample: &Message) -> RuleEval {
        if let Err(e) = validate_rule(rule) {
            return RuleEval {
                matched: false,
                reasons: vec![format!("✗ rule is invalid: {}", e.message)],
                planned: vec![],
            };
        }
        if let Some(why) = self.guard(sample, false, "") {
            return RuleEval {
                matched: false,
                reasons: vec![format!("✗ never automated: {why}")],
                planned: vec![],
            };
        }
        let is_group = sample.chat_jid.ends_with("@g.us");
        let ctx = MatchCtx {
            message: sample,
            is_group,
            me: &self.me,
        };
        let (matched, reasons) = check(rule, &ctx, self.regex_for(rule).as_ref());
        let planned = if matched {
            rule.actions
                .iter()
                .map(|a| describe(a, sample, &sample.chat_jid))
                .collect()
        } else {
            vec![]
        };
        RuleEval {
            matched,
            reasons,
            planned,
        }
    }

    /// Process newly arrived rows. Returns the firings (also logged in the store).
    pub async fn process(&self, batch: &[IncomingMessage]) -> Vec<AutomationEvent> {
        let Ok(settings) = self.store.get_settings() else {
            return vec![];
        };
        if !settings.automation_enabled || batch.is_empty() {
            return vec![];
        }
        let max_rowid = batch.iter().map(|m| m.rowid).max().unwrap_or(0);
        let Some(watermark) = self.watermark() else {
            // Enabled without a watermark: start from here, never from history.
            let _ = self.store.set_kv(WATERMARK_KEY, &max_rowid.to_string());
            return vec![];
        };
        let rules: Vec<Rule> = match self.store.list_rules() {
            Ok(r) => r
                .into_iter()
                .filter(|r| r.enabled && r.id.is_some() && validate_rule(r).is_ok())
                .collect(),
            Err(e) => {
                self.issue(e.kind, "could not load automation rules", &e.message);
                return vec![];
            }
        };
        let now = (self.clock)();
        let mut events = Vec::new();
        for inc in batch {
            if inc.rowid <= watermark {
                continue;
            }
            let m = &inc.message;
            if let Some(why) = self.guard(m, inc.is_status, &inc.raw_media_type) {
                tracing::trace!(rowid = inc.rowid, why, "automation skipped");
                continue;
            }
            if now - m.ts > MAX_AGE_SECS {
                continue;
            }
            for rule in &rules {
                if let Some(ev) = self.try_rule(rule, inc, &settings, now).await {
                    events.push(ev);
                }
            }
        }
        if max_rowid > watermark {
            let _ = self.store.set_kv(WATERMARK_KEY, &max_rowid.to_string());
        }
        events
    }

    async fn try_rule(
        &self,
        rule: &Rule,
        inc: &IncomingMessage,
        settings: &Settings,
        now: i64,
    ) -> Option<AutomationEvent> {
        let id = rule.id?;
        let m = &inc.message;
        let ctx = MatchCtx {
            message: m,
            is_group: inc.is_group,
            me: &self.me,
        };
        let (matched, _) = check(rule, &ctx, self.regex_for(rule).as_ref());
        if !matched {
            return None;
        }
        let cooldown = i64::from(if rule.cooldown_secs == 0 {
            DEFAULT_COOLDOWN
        } else {
            rule.cooldown_secs
        });
        {
            let mut rate = self.rate.lock();
            let key = (id, m.chat_jid.clone());
            if rate
                .cooldowns
                .get(&key)
                .is_some_and(|&t| now - t < cooldown)
            {
                tracing::debug!(rule = id, "rule in cooldown for this chat");
                return None;
            }
            rate.cooldowns.insert(key, now);
        }
        let mut outcomes = Vec::new();
        for a in &rule.actions {
            let o = if rule.dry_run {
                format!("would {}", describe(a, m, &inc.chat_name))
            } else {
                self.execute(rule, a, inc, settings, now).await
            };
            outcomes.push(o);
        }
        let ev = AutomationEvent {
            rule_id: id,
            rule_name: rule.name.clone(),
            chat_jid: m.chat_jid.clone(),
            message_id: m.id.clone(),
            dry_run: rule.dry_run,
            outcomes,
            at: now,
        };
        tracing::info!(
            rule = id,
            dry_run = rule.dry_run,
            actions = rule.actions.len(),
            "automation fired"
        );
        if let Err(e) = self.store.record_rule_fired(id, now) {
            tracing::warn!(error = %e, "could not record rule firing");
        }
        if let Err(e) = self.store.log_automation(&ev) {
            tracing::warn!(error = %e, "could not log automation event");
        }
        Some(ev)
    }

    fn take_send(&self, settings: &Settings, now: i64) -> bool {
        let cap = settings.automation_max_per_hour;
        let mut rate = self.rate.lock();
        let b = rate.bucket.get_or_insert(Bucket {
            cap,
            tokens: f64::from(cap),
            last: now,
        });
        b.take(cap, now)
    }

    fn issue(&self, kind: ErrorKind, title: &str, detail: &str) {
        if let Some(s) = &self.issues {
            s.record(kind, "automation", title, Some(detail));
        }
    }

    fn failed(&self, rule: &Rule, what: &str, e: &AppError) -> String {
        self.issue(
            e.kind,
            &format!("Automation '{}': {what} failed", rule.name),
            &e.message,
        );
        format!("{what} failed: {}", e.message)
    }

    async fn execute(
        &self,
        rule: &Rule,
        a: &Action,
        inc: &IncomingMessage,
        settings: &Settings,
        now: i64,
    ) -> String {
        let m = &inc.message;
        match a {
            Action::Reply { text } => {
                if !self.take_send(settings, now) {
                    return "skipped reply: hourly limit reached".into();
                }
                let req = BridgeSend {
                    recipient: m.chat_jid.clone(),
                    message: Some(render(text, m, &inc.chat_name)),
                    quoted_message_id: Some(m.id.clone()),
                    quoted_sender_jid: m.sender_jid.clone(),
                    quoted_content: Some(truncate(&m.content, 200)),
                    ..Default::default()
                };
                match self.bridge.send(req).await {
                    Ok(_) => "replied".into(),
                    Err(e) => self.failed(rule, "reply", &e),
                }
            }
            Action::DownloadMedia => {
                if m.media.is_none() {
                    return "no media to download".into();
                }
                match self.bridge.download(&m.chat_jid, &m.id, false).await {
                    Ok(d) => format!("downloaded {}", d.filename),
                    Err(e) => self.failed(rule, "download", &e),
                }
            }
            Action::Notify { title } => {
                let t = title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| rule.name.clone());
                let body = format!(
                    "{} in {}: {}",
                    m.sender_name,
                    inc.chat_name,
                    truncate(&m.content, 200)
                );
                self.notifier.notify(&t, &body);
                "notified".into()
            }
            Action::Forward { to_jid } => self.forward(rule, inc, to_jid, settings, now).await,
            Action::Tag { label } => format!("tagged '{label}'"),
            Action::Webhook { url } => self.webhook(rule, inc, url).await,
        }
    }

    async fn forward(
        &self,
        rule: &Rule,
        inc: &IncomingMessage,
        to: &str,
        settings: &Settings,
        now: i64,
    ) -> String {
        let m = &inc.message;
        let mut media_path = None;
        if m.media.is_some()
            && let (Some(store), Some(paths)) = (&self.messages, &self.paths)
            && let Ok(Some(info)) = store.media_info(&m.chat_jid, &m.id)
            && let Some(local) = info.local_path.filter(|p| p.is_file())
        {
            let paths = paths.clone();
            match tokio::task::spawn_blocking(move || wa_bridge::outbox::stage(&paths, &local))
                .await
            {
                Ok(Ok(p)) => media_path = Some(p),
                Ok(Err(e)) => return self.failed(rule, "forward", &e),
                Err(_) => return "forward failed: staging crashed".into(),
            }
        }
        let text = (!m.content.is_empty()).then(|| m.content.clone());
        if text.is_none() && media_path.is_none() {
            return "nothing to forward".into();
        }
        if !self.take_send(settings, now) {
            return "skipped forward: hourly limit reached".into();
        }
        let has_media = media_path.is_some();
        let req = BridgeSend {
            recipient: to.to_string(),
            message: text,
            media_path,
            ..Default::default()
        };
        match self.bridge.send(req).await {
            Ok(_) if has_media => format!("forwarded with media to {to}"),
            Ok(_) => format!("forwarded to {to}"),
            Err(e) => self.failed(rule, "forward", &e),
        }
    }

    async fn webhook(&self, rule: &Rule, inc: &IncomingMessage, url: &str) -> String {
        let body = json!({
            "rule": {"id": rule.id, "name": rule.name},
            "message": inc.message,
            "chat_name": inc.chat_name,
            "is_group": inc.is_group,
        });
        match self.http.post(url).json(&body).send().await {
            Ok(r) if r.status().is_success() => format!("webhook HTTP {}", r.status().as_u16()),
            Ok(r) if r.status().is_redirection() => {
                format!(
                    "webhook HTTP {} (redirect not followed)",
                    r.status().as_u16()
                )
            }
            Ok(r) => {
                let e = AppError::external(format!("HTTP {}", r.status().as_u16()));
                self.failed(rule, "webhook", &e)
            }
            Err(e) => {
                let kind = if e.is_timeout() {
                    ErrorKind::Timeout
                } else {
                    ErrorKind::External
                };
                self.failed(
                    rule,
                    "webhook",
                    &AppError::new(
                        kind,
                        if e.is_timeout() {
                            "timed out".to_string()
                        } else {
                            "could not connect".to_string()
                        },
                    ),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_refills_over_time() {
        let mut b = Bucket {
            cap: 2,
            tokens: 2.0,
            last: 0,
        };
        assert!(b.take(2, 0));
        assert!(b.take(2, 0));
        assert!(!b.take(2, 0));
        assert!(!b.take(2, 1000), "half a token after 1000 s");
        assert!(b.take(2, 1800));
        let mut z = Bucket {
            cap: 0,
            tokens: 0.0,
            last: 0,
        };
        assert!(!z.take(0, 100_000));
        assert!(!b.take(1, 1800), "lowered cap clamps tokens");
    }
}
