use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;
use wa_automation::{Engine, WATERMARK_KEY, validate_rule};
use wa_core::config::Paths;
use wa_core::time::now_epoch;
use wa_core::traits::*;
use wa_core::types::*;
use wa_core::{AppError, ErrorKind, Result};

const ME: &str = "919900000000";
const ALICE: &str = "919811111111@s.whatsapp.net";
const GROUP: &str = "120363000000000001@g.us";

// ---------- mocks ----------

#[derive(Default)]
struct MemApp {
    settings: Mutex<Settings>,
    rules: Mutex<Vec<Rule>>,
    kv: Mutex<HashMap<String, String>>,
    fired: Mutex<Vec<(i64, i64)>>,
    log: Mutex<Vec<AutomationEvent>>,
}

impl AppStore for MemApp {
    fn get_settings(&self) -> Result<Settings> {
        Ok(self.settings.lock().clone())
    }
    fn put_settings(&self, s: &Settings) -> Result<()> {
        *self.settings.lock() = s.clone();
        Ok(())
    }
    fn list_rules(&self) -> Result<Vec<Rule>> {
        Ok(self.rules.lock().clone())
    }
    fn save_rule(&self, r: &Rule) -> Result<Rule> {
        Ok(r.clone())
    }
    fn delete_rule(&self, _: i64) -> Result<()> {
        Ok(())
    }
    fn record_rule_fired(&self, id: i64, at: i64) -> Result<()> {
        self.fired.lock().push((id, at));
        Ok(())
    }
    fn log_automation(&self, ev: &AutomationEvent) -> Result<()> {
        self.log.lock().push(ev.clone());
        Ok(())
    }
    fn recent_automation(&self, _: u32) -> Result<Vec<AutomationEvent>> {
        Ok(self.log.lock().clone())
    }
    fn get_kv(&self, k: &str) -> Result<Option<String>> {
        Ok(self.kv.lock().get(k).cloned())
    }
    fn set_kv(&self, k: &str, v: &str) -> Result<()> {
        self.kv.lock().insert(k.into(), v.into());
        Ok(())
    }
    fn insert_job(&self, _: &JobRecord) -> Result<i64> {
        Ok(1)
    }
    fn update_job(&self, _: &JobRecord) -> Result<()> {
        Ok(())
    }
    fn list_jobs(&self, _: u32) -> Result<Vec<JobRecord>> {
        Ok(vec![])
    }
    fn fail_interrupted_jobs(&self) -> Result<u32> {
        Ok(0)
    }
}

#[derive(Default)]
struct MockBridge {
    sends: Mutex<Vec<BridgeSend>>,
    downloads: Mutex<Vec<(String, bool)>>,
    fail_send: Mutex<Option<AppError>>,
}

#[async_trait]
impl BridgeApi for MockBridge {
    async fn connection(&self) -> ConnectionState {
        unimplemented!()
    }
    async fn reconnect(&self) -> Result<()> {
        Ok(())
    }
    async fn send(&self, req: BridgeSend) -> Result<String> {
        if let Some(e) = self.fail_send.lock().clone() {
            return Err(e);
        }
        self.sends.lock().push(req);
        Ok("sent".into())
    }
    async fn react(&self, _: &str, _: &str, _: &str, _: bool, _: Option<&str>) -> Result<()> {
        Ok(())
    }
    async fn download(&self, _: &str, id: &str, no_retry: bool) -> Result<DownloadOutcome> {
        self.downloads.lock().push((id.into(), no_retry));
        Ok(DownloadOutcome {
            path: PathBuf::from("/s/x"),
            filename: format!("{id}.jpg"),
        })
    }
    async fn mark_read(&self, _: &str, _: &[String], _: Option<&str>) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Notes(Mutex<Vec<(String, String)>>);
impl Notifier for Notes {
    fn notify(&self, t: &str, b: &str) {
        self.0.lock().push((t.into(), b.into()));
    }
}

#[derive(Default)]
struct Issues(Mutex<Vec<(ErrorKind, String)>>);
impl IssueSink for Issues {
    fn record(&self, k: ErrorKind, _: &str, title: &str, _: Option<&str>) {
        self.0.lock().push((k, title.into()));
    }
}

#[derive(Default)]
struct Store(Mutex<Vec<MediaInfo>>);
impl MessageStore for Store {
    fn max_rowid(&self) -> Result<i64> {
        Ok(0)
    }
    fn messages_after(&self, _: i64, _: u32) -> Result<Vec<IncomingMessage>> {
        Ok(vec![])
    }
    fn get_message(&self, _: &str, _: &str) -> Result<Option<Message>> {
        Ok(None)
    }
    fn media_info(&self, c: &str, id: &str) -> Result<Option<MediaInfo>> {
        Ok(self
            .0
            .lock()
            .iter()
            .find(|m| m.chat_jid == c && m.message_id == id)
            .cloned())
    }
    fn media_rows(&self, _: Option<i64>, _: &[MediaKind]) -> Result<Vec<MediaInfo>> {
        Ok(vec![])
    }
}

// ---------- helpers ----------

struct T {
    engine: Engine,
    app: Arc<MemApp>,
    bridge: Arc<MockBridge>,
    notes: Arc<Notes>,
    issues: Arc<Issues>,
    clock: Arc<AtomicI64>,
}

fn rule(id: i64, actions: Vec<Action>) -> Rule {
    Rule {
        id: Some(id),
        name: format!("rule {id}"),
        enabled: true,
        dry_run: false,
        trigger: Trigger::default(),
        actions,
        cooldown_secs: 600,
        fired_count: 0,
        last_fired_at: None,
    }
}

fn reply(text: &str) -> Action {
    Action::Reply { text: text.into() }
}

fn setup(rules: Vec<Rule>) -> T {
    let app = Arc::new(MemApp::default());
    app.settings.lock().automation_enabled = true;
    *app.rules.lock() = rules;
    app.kv.lock().insert(WATERMARK_KEY.into(), "0".into());
    let bridge = Arc::new(MockBridge::default());
    let notes = Arc::new(Notes::default());
    let issues = Arc::new(Issues::default());
    let clock = Arc::new(AtomicI64::new(now_epoch()));
    let c = clock.clone();
    let engine = Engine::new(
        app.clone(),
        bridge.clone(),
        notes.clone(),
        vec![format!("{ME}@s.whatsapp.net"), "111111111111111".into()],
    )
    .with_issues(issues.clone())
    .with_clock(Arc::new(move || c.load(Ordering::SeqCst)));
    T {
        engine,
        app,
        bridge,
        notes,
        issues,
        clock,
    }
}

fn msg_at(id: &str, chat: &str, sender: &str, content: &str, ts: i64) -> Message {
    Message {
        id: id.into(),
        chat_jid: chat.into(),
        sender_jid: Some(sender.into()),
        sender_name: "Alice".into(),
        is_from_me: false,
        ts,
        content: content.into(),
        media: None,
        quoted: None,
        reactions: vec![],
        deleted: false,
    }
}

impl T {
    fn inc(&self, rowid: i64, chat: &str, content: &str) -> IncomingMessage {
        let now = self.clock.load(Ordering::SeqCst);
        IncomingMessage {
            rowid,
            message: msg_at(&format!("M{rowid}"), chat, ALICE, content, now),
            is_group: chat.ends_with("@g.us"),
            chat_name: if chat.ends_with("@g.us") {
                "ML CSE".into()
            } else {
                "Alice".into()
            },
            raw_media_type: String::new(),
            is_status: false,
        }
    }
    fn sends(&self) -> usize {
        self.bridge.sends.lock().len()
    }
}

fn chat(n: usize) -> String {
    format!("9198{n:08}@s.whatsapp.net")
}

// ---------- guards ----------

#[tokio::test]
async fn master_switch_off_does_nothing() {
    let t = setup(vec![rule(1, vec![reply("hi")])]);
    t.app.settings.lock().automation_enabled = false;
    assert!(
        t.engine
            .process(&[t.inc(1, ALICE, "hello")])
            .await
            .is_empty()
    );
    assert_eq!(t.sends(), 0);
}

#[tokio::test]
async fn basic_reply_fires_and_is_logged() {
    let t = setup(vec![rule(1, vec![reply("hi")])]);
    let ev = t.engine.process(&[t.inc(5, ALICE, "hello")]).await;
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].outcomes, ["replied"]);
    assert!(!ev[0].dry_run);
    let s = t.bridge.sends.lock()[0].clone();
    assert_eq!(s.recipient, ALICE);
    assert_eq!(s.quoted_message_id.as_deref(), Some("M5"));
    assert_eq!(t.app.fired.lock().len(), 1);
    assert_eq!(t.app.log.lock()[0], ev[0]);
    assert_eq!(t.engine.watermark(), Some(5));
}

#[tokio::test]
async fn never_fires_on_own_reaction_deleted_status_or_self_sender() {
    let t = setup(vec![rule(1, vec![reply("hi")])]);
    let mut own = t.inc(1, ALICE, "x");
    own.message.is_from_me = true;
    let mut react = t.inc(2, ALICE, "👍");
    react.raw_media_type = "reaction".into();
    let mut del = t.inc(3, ALICE, "x");
    del.message.deleted = true;
    let mut status = t.inc(4, "status@broadcast", "x");
    status.is_status = true;
    let mut status2 = t.inc(5, ALICE, "x");
    status2.is_status = true;
    let mut self_sender = t.inc(6, ALICE, "x");
    self_sender.message.sender_jid = Some(format!("{ME}:7@s.whatsapp.net"));
    let mut lid_self = t.inc(7, ALICE, "x");
    lid_self.message.sender_jid = Some("111111111111111@lid".into());
    let ev = t
        .engine
        .process(&[own, react, del, status, status2, self_sender, lid_self])
        .await;
    assert!(ev.is_empty(), "{ev:?}");
    assert_eq!(t.sends(), 0);
    assert_eq!(t.engine.watermark(), Some(7), "watermark still advances");
}

#[tokio::test]
async fn watermark_and_age_guards() {
    let t = setup(vec![rule(1, vec![Action::Tag { label: "x".into() }])]);
    t.app.kv.lock().insert(WATERMARK_KEY.into(), "10".into());
    assert!(
        t.engine
            .process(&[t.inc(9, ALICE, "a"), t.inc(10, ALICE, "b")])
            .await
            .is_empty()
    );
    let mut old = t.inc(11, ALICE, "c");
    old.message.ts -= 601;
    assert!(t.engine.process(&[old]).await.is_empty());
    let mut fresh = t.inc(12, ALICE, "d");
    fresh.message.ts -= 599;
    assert_eq!(t.engine.process(&[fresh]).await.len(), 1);
}

#[tokio::test]
async fn missing_watermark_starts_from_now() {
    let t = setup(vec![rule(1, vec![reply("hi")])]);
    t.app.kv.lock().clear();
    assert!(t.engine.process(&[t.inc(40, ALICE, "x")]).await.is_empty());
    assert_eq!(t.engine.watermark(), Some(40));
    assert_eq!(t.engine.process(&[t.inc(41, ALICE, "x")]).await.len(), 1);
}

#[tokio::test]
async fn enable_sets_watermark_and_resets_cooldowns() {
    let t = setup(vec![rule(1, vec![Action::Tag { label: "x".into() }])]);
    assert_eq!(t.engine.process(&[t.inc(1, ALICE, "a")]).await.len(), 1);
    assert!(
        t.engine.process(&[t.inc(2, ALICE, "a")]).await.is_empty(),
        "cooldown"
    );
    t.engine.enable(100).unwrap();
    assert_eq!(t.engine.watermark(), Some(100));
    assert!(t.engine.process(&[t.inc(100, ALICE, "a")]).await.is_empty());
    assert_eq!(t.engine.process(&[t.inc(101, ALICE, "a")]).await.len(), 1);
}

#[tokio::test]
async fn disabled_and_invalid_rules_are_ignored() {
    let mut off = rule(1, vec![reply("x")]);
    off.enabled = false;
    let mut bad = rule(2, vec![reply("x")]);
    bad.trigger.regex = Some("(".into());
    let mut unsaved = rule(3, vec![reply("x")]);
    unsaved.id = None;
    let t = setup(vec![off, bad, unsaved]);
    assert!(t.engine.process(&[t.inc(1, ALICE, "x")]).await.is_empty());
}

// ---------- groups ----------

#[tokio::test]
async fn groups_need_opt_in_listing_and_mentions_for_replies() {
    let mut notify_rule = rule(1, vec![Action::Notify { title: None }]);
    notify_rule.trigger.include_groups = true;
    notify_rule.trigger.chat_jids = vec![GROUP.into()];
    let mut reply_rule = rule(2, vec![reply("on it")]);
    reply_rule.trigger.include_groups = true;
    reply_rule.trigger.chat_jids = vec![GROUP.into()];
    let direct_only = rule(3, vec![Action::Tag { label: "dm".into() }]);
    let t = setup(vec![notify_rule, reply_rule, direct_only]);

    let ev = t.engine.process(&[t.inc(1, GROUP, "exam tomorrow")]).await;
    assert_eq!(
        ev.iter().map(|e| e.rule_id).collect::<Vec<_>>(),
        [1],
        "only notify, no mention"
    );
    assert_eq!(t.notes.0.lock()[0].0, "rule 1");
    assert!(t.notes.0.lock()[0].1.starts_with("Alice in ML CSE: exam"));

    let ev = t
        .engine
        .process(&[t.inc(2, GROUP, &format!("@{ME} can you share?"))])
        .await;
    assert_eq!(
        ev.iter().map(|e| e.rule_id).collect::<Vec<_>>(),
        [2],
        "rule 1 in cooldown"
    );
    assert_eq!(t.sends(), 1);

    // Unlisted group never fires.
    let ev = t
        .engine
        .process(&[t.inc(3, "120363999999999999@g.us", &format!("@{ME}"))])
        .await;
    assert!(ev.is_empty());
}

#[tokio::test]
async fn mention_only_and_lid_mention() {
    let mut r = rule(1, vec![Action::Tag { label: "m".into() }]);
    r.trigger.include_groups = true;
    r.trigger.chat_jids = vec![GROUP.into()];
    r.trigger.mention_only = true;
    let t = setup(vec![r]);
    assert!(
        t.engine
            .process(&[t.inc(1, GROUP, "hello all")])
            .await
            .is_empty()
    );
    assert_eq!(
        t.engine
            .process(&[t.inc(2, GROUP, "@111111111111111 ping")])
            .await
            .len(),
        1
    );
}

// ---------- templating, rates ----------

#[tokio::test]
async fn reply_templating() {
    let t = setup(vec![rule(
        1,
        vec![reply(
            "Hi {sender_name}! Re \"{text}\" in {chat_name} at {time} {unknown} {",
        )],
    )]);
    t.engine
        .process(&[t.inc(1, ALICE, "call {sender_name} later")])
        .await;
    let sent = t.bridge.sends.lock()[0].message.clone().unwrap();
    let hhmm = wa_automation::rules::local_hhmm(t.clock.load(Ordering::SeqCst));
    assert_eq!(
        sent,
        format!("Hi Alice! Re \"call {{sender_name}} later\" in Alice at {hhmm} {{unknown}} {{")
    );
}

#[tokio::test]
async fn cooldown_per_rule_per_chat() {
    let mut r = rule(1, vec![reply("auto")]);
    r.cooldown_secs = 120;
    let t = setup(vec![r]);
    assert_eq!(
        t.engine
            .process(&[t.inc(1, ALICE, "a"), t.inc(2, ALICE, "b")])
            .await
            .len(),
        1
    );
    assert_eq!(
        t.engine.process(&[t.inc(3, &chat(1), "c")]).await.len(),
        1,
        "other chat"
    );
    t.clock.fetch_add(119, Ordering::SeqCst);
    assert!(t.engine.process(&[t.inc(4, ALICE, "d")]).await.is_empty());
    t.clock.fetch_add(2, Ordering::SeqCst);
    assert_eq!(t.engine.process(&[t.inc(5, ALICE, "e")]).await.len(), 1);
    assert_eq!(t.sends(), 3);
}

#[tokio::test]
async fn zero_cooldown_means_default() {
    let mut r = rule(1, vec![Action::Tag { label: "x".into() }]);
    r.cooldown_secs = 0;
    let t = setup(vec![r]);
    assert_eq!(t.engine.process(&[t.inc(1, ALICE, "a")]).await.len(), 1);
    t.clock.fetch_add(599, Ordering::SeqCst);
    assert!(t.engine.process(&[t.inc(2, ALICE, "a")]).await.is_empty());
}

#[tokio::test]
async fn hourly_cap_limits_sends_and_refills() {
    let t = setup(vec![rule(
        1,
        vec![
            reply("auto"),
            Action::Tag {
                label: "seen".into(),
            },
        ],
    )]);
    t.app.settings.lock().automation_max_per_hour = 3;
    let batch: Vec<_> = (0..10)
        .map(|i| t.inc(i + 1, &chat(i as usize), "hi"))
        .collect();
    let ev = t.engine.process(&batch).await;
    assert_eq!(ev.len(), 10, "rule fired (and tagged) every time");
    assert_eq!(t.sends(), 3);
    assert_eq!(
        ev.iter()
            .filter(|e| e.outcomes[0] == "skipped reply: hourly limit reached")
            .count(),
        7
    );
    assert!(ev.iter().all(|e| e.outcomes[1] == "tagged 'seen'"));
    // 20 minutes later one more token is available.
    t.clock.fetch_add(1200, Ordering::SeqCst);
    let batch: Vec<_> = (0..3)
        .map(|i| t.inc(100 + i, &chat(50 + i as usize), "hi"))
        .collect();
    t.engine.process(&batch).await;
    assert_eq!(t.sends(), 4);
}

#[tokio::test]
async fn burst_of_fifty_is_bounded() {
    let t = setup(vec![rule(1, vec![reply("auto")])]);
    let same: Vec<_> = (0..50).map(|i| t.inc(i + 1, ALICE, "spam")).collect();
    t.engine.process(&same).await;
    assert_eq!(t.sends(), 1, "one chat → cooldown");
    let many: Vec<_> = (0..50)
        .map(|i| t.inc(100 + i, &chat(i as usize), "spam"))
        .collect();
    t.engine.process(&many).await;
    assert_eq!(t.sends(), 20, "default cap 20/hour");
}

#[tokio::test]
async fn dry_run_executes_nothing() {
    let mut r = rule(
        1,
        vec![
            reply("Hello {sender_name}"),
            Action::DownloadMedia,
            Action::Notify {
                title: Some("T".into()),
            },
            Action::Forward {
                to_jid: "919822222222".into(),
            },
            Action::Webhook {
                url: "http://127.0.0.1:9/hook".into(),
            },
        ],
    );
    r.dry_run = true;
    let t = setup(vec![r]);
    let ev = t.engine.process(&[t.inc(1, ALICE, "x")]).await;
    assert!(ev[0].dry_run);
    assert_eq!(ev[0].outcomes[0], "would reply \"Hello Alice\"");
    assert!(ev[0].outcomes.iter().all(|o| o.starts_with("would ")));
    assert_eq!(t.sends(), 0);
    assert!(t.bridge.downloads.lock().is_empty() && t.notes.0.lock().is_empty());
    assert_eq!(t.app.log.lock().len(), 1);
}

// ---------- actions ----------

#[tokio::test]
async fn failures_are_outcomes_and_issues_not_panics() {
    let t = setup(vec![
        rule(
            1,
            vec![
                reply("x"),
                Action::Tag {
                    label: "after".into(),
                },
            ],
        ),
        rule(
            2,
            vec![Action::Tag {
                label: "other rule".into(),
            }],
        ),
    ]);
    *t.bridge.fail_send.lock() = Some(AppError::bridge_down("WhatsApp is not connected"));
    let ev = t.engine.process(&[t.inc(1, ALICE, "x")]).await;
    assert_eq!(ev.len(), 2);
    assert_eq!(
        ev[0].outcomes,
        ["reply failed: WhatsApp is not connected", "tagged 'after'"]
    );
    assert_eq!(ev[1].outcomes, ["tagged 'other rule'"]);
    let issues = t.issues.0.lock();
    assert_eq!(issues[0].0, ErrorKind::BridgeDown);
    assert!(issues[0].1.contains("rule 1"));
}

#[tokio::test]
async fn download_media_asks_phone_and_skips_text() {
    let t = setup(vec![rule(1, vec![Action::DownloadMedia])]);
    let mut m = t.inc(1, ALICE, "");
    m.message.media = Some(MediaRef {
        kind: MediaKind::Image,
        filename: None,
        size: None,
        mime: None,
        source: MediaSource::Missing,
        url: None,
        thumb_url: None,
    });
    let ev = t.engine.process(&[m]).await;
    assert_eq!(ev[0].outcomes, ["downloaded M1.jpg"]);
    assert_eq!(t.bridge.downloads.lock()[0], ("M1".to_string(), false));
    let ev = t.engine.process(&[t.inc(2, &chat(1), "text only")]).await;
    assert_eq!(ev[0].outcomes, ["no media to download"]);
}

#[tokio::test]
async fn forward_text_and_media_via_outbox_and_no_loops() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under_root(dir.path(), "http://127.0.0.1:8080".into());
    let photo = dir.path().join("photo.jpg");
    std::fs::write(&photo, b"jpeg").unwrap();
    let store = Arc::new(Store::default());
    store.0.lock().push(MediaInfo {
        chat_jid: ALICE.into(),
        message_id: "M2".into(),
        kind: MediaKind::Image,
        filename: None,
        file_length: None,
        file_sha256: None,
        raw_ts: String::new(),
        ts: 0,
        local_path: Some(photo),
        source: MediaSource::Store,
    });
    let mut t = setup(vec![rule(
        1,
        vec![Action::Forward {
            to_jid: "919822222222@s.whatsapp.net".into(),
        }],
    )]);
    t.engine = t.engine.with_messages(store).with_paths(paths.clone());
    let ev = t.engine.process(&[t.inc(1, ALICE, "fwd me")]).await;
    assert_eq!(ev[0].outcomes, ["forwarded to 919822222222@s.whatsapp.net"]);
    t.clock.fetch_add(700, Ordering::SeqCst);
    let mut m = t.inc(2, ALICE, "");
    m.message.media = Some(MediaRef {
        kind: MediaKind::Image,
        filename: None,
        size: None,
        mime: None,
        source: MediaSource::Store,
        url: None,
        thumb_url: None,
    });
    let ev = t.engine.process(&[m]).await;
    assert_eq!(
        ev[0].outcomes,
        ["forwarded with media to 919822222222@s.whatsapp.net"]
    );
    let sent = t.bridge.sends.lock()[1].clone();
    let mp = sent.media_path.unwrap();
    assert!(mp.starts_with(std::fs::canonicalize(&paths.outbox).unwrap()));
    assert!(sent.message.is_none());
    // The forwarded copy shows up as our own message in the target chat: ignored.
    let mut echo = t.inc(3, "919822222222@s.whatsapp.net", "fwd me");
    echo.message.is_from_me = true;
    assert!(t.engine.process(&[echo]).await.is_empty());
    assert_eq!(t.sends(), 2);
}

// ---------- webhook ----------

async fn hook_server() -> (String, Arc<Mutex<Vec<Value>>>, Arc<AtomicUsize>) {
    use axum::routing::post;
    let got: Arc<Mutex<Vec<Value>>> = Arc::default();
    let hits = Arc::new(AtomicUsize::new(0));
    let (g, h) = (got.clone(), hits.clone());
    let app = axum::Router::new()
        .route(
            "/hook",
            post(move |axum::Json(v): axum::Json<Value>| {
                let (g, h) = (g.clone(), h.clone());
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    g.lock().push(v);
                    "ok"
                }
            }),
        )
        .route(
            "/redir",
            post(|| async { axum::response::Redirect::temporary("/hook") }),
        )
        .route(
            "/fail",
            post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "no") }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, got, hits)
}

#[tokio::test]
async fn webhook_posts_json_and_ignores_redirects() {
    let (base, got, hits) = hook_server().await;
    let t = setup(vec![
        rule(
            1,
            vec![Action::Webhook {
                url: format!("{base}/hook"),
            }],
        ),
        rule(
            2,
            vec![Action::Webhook {
                url: format!("{base}/redir"),
            }],
        ),
        rule(
            3,
            vec![Action::Webhook {
                url: format!("{base}/fail"),
            }],
        ),
    ]);
    let ev = t.engine.process(&[t.inc(1, ALICE, "order #42")]).await;
    assert_eq!(ev[0].outcomes, ["webhook HTTP 200"]);
    assert_eq!(ev[1].outcomes, ["webhook HTTP 307 (redirect not followed)"]);
    assert_eq!(ev[2].outcomes, ["webhook failed: HTTP 500"]);
    assert_eq!(hits.load(Ordering::SeqCst), 1, "redirect target never hit");
    let body = got.lock()[0].clone();
    assert_eq!(body["rule"]["name"], "rule 1");
    assert_eq!(body["message"]["id"], "M1");
    assert_eq!(body["message"]["content"], "order #42");
    assert_eq!(t.issues.0.lock().len(), 1);
}

#[tokio::test]
async fn webhook_unreachable_is_outcome() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let t = setup(vec![rule(
        1,
        vec![Action::Webhook {
            url: format!("http://127.0.0.1:{port}/x"),
        }],
    )]);
    let ev = t.engine.process(&[t.inc(1, ALICE, "x")]).await;
    assert_eq!(ev[0].outcomes, ["webhook failed: could not connect"]);
}

// ---------- validation & test_rule ----------

#[test]
fn invalid_rules_rejected() {
    let base = || rule(1, vec![reply("ok")]);
    assert!(validate_rule(&base()).is_ok());
    type Mutate = Box<dyn Fn(&mut Rule)>;
    let cases: Vec<(&str, Mutate)> = vec![
        (
            "bad regex",
            Box::new(|r| r.trigger.regex = Some("([a-z".into())),
        ),
        (
            "huge regex",
            Box::new(|r| r.trigger.regex = Some("x".repeat(600))),
        ),
        (
            "bad time",
            Box::new(|r| {
                r.trigger.active_from = Some("25:00".into());
                r.trigger.active_to = Some("10:00".into())
            }),
        ),
        (
            "one-sided time",
            Box::new(|r| r.trigger.active_from = Some("09:00".into())),
        ),
        (
            "ftp webhook",
            Box::new(|r| {
                r.actions = vec![Action::Webhook {
                    url: "ftp://x.org/h".into(),
                }]
            }),
        ),
        (
            "garbage webhook",
            Box::new(|r| {
                r.actions = vec![Action::Webhook {
                    url: "not a url".into(),
                }]
            }),
        ),
        ("no actions", Box::new(|r| r.actions.clear())),
        (
            "bad forward",
            Box::new(|r| {
                r.actions = vec![Action::Forward {
                    to_jid: "alice".into(),
                }]
            }),
        ),
        ("empty reply", Box::new(|r| r.actions = vec![reply("  ")])),
        (
            "long reply",
            Box::new(|r| r.actions = vec![reply(&"x".repeat(4097))]),
        ),
        ("empty name", Box::new(|r| r.name = " ".into())),
        (
            "empty keyword",
            Box::new(|r| r.trigger.keywords = vec!["".into()]),
        ),
        (
            "groups without group",
            Box::new(|r| r.trigger.include_groups = true),
        ),
        (
            "bad chat",
            Box::new(|r| r.trigger.chat_jids = vec!["x".into()]),
        ),
        (
            "empty tag",
            Box::new(|r| r.actions = vec![Action::Tag { label: "".into() }]),
        ),
    ];
    for (name, f) in cases {
        let mut r = base();
        f(&mut r);
        assert_eq!(
            validate_rule(&r).unwrap_err().kind,
            ErrorKind::Invalid,
            "{name}"
        );
    }
    let mut ok = base();
    ok.actions = vec![
        reply(&"x".repeat(4096)),
        Action::Webhook {
            url: "https://example.org/h".into(),
        },
        Action::Forward {
            to_jid: "+919822222222".into(),
        },
    ];
    ok.trigger.active_from = Some("22:00".into());
    ok.trigger.active_to = Some("06:00".into());
    assert!(validate_rule(&ok).is_ok());
}

#[test]
fn test_rule_explains_each_check() {
    let t = setup(vec![]);
    let mut r = rule(
        1,
        vec![
            reply("thanks {sender_name}"),
            Action::Tag {
                label: "inv".into(),
            },
        ],
    );
    r.trigger.keywords = vec!["Invoice".into()];
    r.trigger.regex = Some(r"#\d+".into());
    r.trigger.senders = vec!["+919811111111".into()];
    let now = now_epoch();
    let m = msg_at(
        "X",
        ALICE,
        "919811111111:3@s.whatsapp.net",
        "your invoice #12",
        now,
    );
    let ev = t.engine.test_rule(&r, &m);
    assert!(ev.matched, "{ev:?}");
    assert!(ev.reasons.iter().all(|r| r.starts_with('✓')));
    assert_eq!(ev.planned, ["reply \"thanks Alice\"", "tag 'inv'"]);

    let m2 = msg_at("X", ALICE, "919811111111@s.whatsapp.net", "hello", now);
    let ev = t.engine.test_rule(&r, &m2);
    assert!(!ev.matched && ev.planned.is_empty());
    assert!(ev.reasons.contains(&"✗ keyword: none found".to_string()));
    assert!(ev.reasons.contains(&"✗ regex: no match".to_string()));

    let m3 = msg_at("X", ALICE, "919800000001@s.whatsapp.net", "invoice #1", now);
    assert!(
        t.engine
            .test_rule(&r, &m3)
            .reasons
            .contains(&"✗ sender: not in the list".to_string())
    );

    let mut own = m.clone();
    own.is_from_me = true;
    assert_eq!(
        t.engine.test_rule(&r, &own).reasons,
        ["✗ never automated: own message"]
    );
    let mut bad = r.clone();
    bad.trigger.regex = Some("(".into());
    assert!(t.engine.test_rule(&bad, &m).reasons[0].starts_with("✗ rule is invalid"));
}

#[test]
fn test_rule_media_time_and_group_checks() {
    let t = setup(vec![]);
    let now = now_epoch();
    let mut r = rule(1, vec![Action::Tag { label: "x".into() }]);
    r.trigger.media_kinds = vec![MediaKind::Document];
    let m = msg_at("X", ALICE, ALICE, "", now);
    assert!(!t.engine.test_rule(&r, &m).matched);
    let mut doc = m.clone();
    doc.media = Some(MediaRef {
        kind: MediaKind::Document,
        filename: None,
        size: None,
        mime: None,
        source: MediaSource::Missing,
        url: None,
        thumb_url: None,
    });
    assert!(t.engine.test_rule(&r, &doc).matched);

    let hh = wa_automation::rules::local_hhmm(now);
    let (h, mm) = (&hh[..2], &hh[3..]);
    let mut w = rule(2, vec![Action::Tag { label: "x".into() }]);
    w.trigger.active_from = Some(format!("{h}:00"));
    w.trigger.active_to = Some(format!("{h}:59"));
    assert!(t.engine.test_rule(&w, &m).matched, "inside {hh}");
    let next = (h.parse::<u32>().unwrap() + 1) % 24;
    w.trigger.active_from = Some(format!("{next:02}:00"));
    w.trigger.active_to = Some(format!("{next:02}:30"));
    assert!(!t.engine.test_rule(&w, &m).matched, "outside {hh} ({mm})");

    let g = msg_at("X", GROUP, ALICE, "hello", now);
    let ev = t
        .engine
        .test_rule(&rule(3, vec![Action::Tag { label: "x".into() }]), &g);
    assert!(
        ev.reasons
            .contains(&"✗ chat: groups need 'include groups' and the group listed".to_string())
    );
}
