//! Desktop notifications that can be replaced and withdrawn.
//!
//! Every message or call notification belongs to a chat. The
//! [`NotificationCenter`] remembers the notification server's id for each
//! chat, so that:
//! - a newer message *replaces* the chat's notification instead of stacking
//!   another one (the title then carries the count, like "Alice (3 messages)");
//! - opening the chat, marking it read (here or on the phone) or clicking one
//!   of its notifications *closes* every notification of that chat.
//!
//! The server is behind [`NotificationServer`]: the freedesktop D-Bus service
//! on Linux (`notify_dbus.rs`), the Tauri plugin elsewhere, a recorder in tests.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

/// Most untracked (automation, summary) notification ids remembered so a click
/// on them can still bring the window up.
const MISC_IDS: usize = 32;

/// One notification as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNote {
    pub title: String,
    pub body: String,
}

impl DesktopNote {
    /// Build from title and body.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
        }
    }
}

/// The desktop notification service.
#[async_trait]
pub trait NotificationServer: Send + Sync {
    /// Show `note`. When `replaces` is non-zero the server updates that
    /// notification in place. Returns the server's id for the notification, or
    /// 0 when it could not be shown or cannot be tracked (it is then never
    /// replaced or closed).
    async fn show(&self, note: &DesktopNote, replaces: u32) -> u32;
    /// Withdraw a notification; a no-op when it is already gone.
    async fn close(&self, id: u32);
}

/// A signal from the notification server about one of its notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerEvent {
    /// The user clicked the notification (freedesktop `ActionInvoked`).
    Activated(u32),
    /// The notification went away: expired, dismissed or closed by us
    /// (freedesktop `NotificationClosed`).
    Closed(u32),
}

/// What a click on a notification should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activation {
    /// Not one of ours (another application's notification).
    Unknown,
    /// Ours but not tied to a chat: bring the window up.
    App,
    /// Open this chat.
    Chat(String),
}

#[derive(Debug, Default)]
struct ChatNotes {
    /// (server id, messages counted in it).
    message: Option<(u32, u32)>,
    /// Ringing / missed call notification.
    call: Option<u32>,
}

impl ChatNotes {
    fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.message.map(|(id, _)| id).into_iter().chain(self.call)
    }
    fn is_empty(&self) -> bool {
        self.message.is_none() && self.call.is_none()
    }
}

#[derive(Debug, Default)]
struct State {
    chats: HashMap<String, ChatNotes>,
    /// "N new messages in M chats": server id and the chats it covers.
    summary: Option<(u32, BTreeMap<String, u32>)>,
    misc: VecDeque<u32>,
}

impl State {
    fn chat_of(&self, id: u32) -> Option<String> {
        self.chats
            .iter()
            .find(|(_, n)| n.ids().any(|i| i == id))
            .map(|(jid, _)| jid.clone())
    }

    /// Forget `id` wherever it is tracked. True when it was ours.
    fn forget(&mut self, id: u32) -> bool {
        let mut found = false;
        self.chats.retain(|_, n| {
            if n.message.is_some_and(|(m, _)| m == id) {
                n.message = None;
                found = true;
            }
            if n.call == Some(id) {
                n.call = None;
                found = true;
            }
            !n.is_empty()
        });
        if self.summary.as_ref().is_some_and(|(s, _)| *s == id) {
            self.summary = None;
            found = true;
        }
        if let Some(i) = self.misc.iter().position(|m| *m == id) {
            self.misc.remove(i);
            found = true;
        }
        found
    }

    fn remember_misc(&mut self, id: u32) {
        if id == 0 {
            return;
        }
        if self.misc.len() >= MISC_IDS {
            self.misc.pop_front();
        }
        self.misc.push_back(id);
    }
}

/// Title for a chat notification that now stands for `count` messages.
pub fn counted_title(title: &str, count: u32) -> String {
    if count > 1 {
        format!("{title} ({count} messages)")
    } else {
        title.to_string()
    }
}

/// Body of the summary notification.
pub fn summary_body(chats: &BTreeMap<String, u32>) -> String {
    let n: u32 = chats.values().sum();
    let c = chats.len();
    format!(
        "{n} new message{} in {c} chat{}",
        if n == 1 { "" } else { "s" },
        if c == 1 { "" } else { "s" }
    )
}

/// Tracks shown notifications per chat (see the module docs).
pub struct NotificationCenter {
    server: Arc<dyn NotificationServer>,
    /// Async so a show and the replace that follows it never interleave.
    state: Mutex<State>,
}

impl NotificationCenter {
    /// A center over `server`.
    pub fn new(server: Arc<dyn NotificationServer>) -> Self {
        Self {
            server,
            state: Mutex::new(State::default()),
        }
    }

    /// Show `added` new messages of `chat_jid`, replacing the chat's previous
    /// message notification (the count accumulates).
    pub async fn message(&self, chat_jid: &str, title: &str, body: &str, added: u32) {
        let mut st = self.state.lock().await;
        let prev = st.chats.get(chat_jid).and_then(|n| n.message);
        let (replaces, count) = prev.map_or((0, 0), |(id, c)| (id, c));
        let count = count + added.max(1);
        let note = DesktopNote::new(counted_title(title, count), body);
        let id = self.server.show(&note, replaces).await;
        let entry = st.chats.entry(chat_jid.to_string()).or_default();
        entry.message = (id != 0).then_some((id, count));
        if entry.is_empty() {
            st.chats.remove(chat_jid);
        }
    }

    /// Show (or replace) the chat's call notification.
    pub async fn call(&self, chat_jid: &str, note: &DesktopNote) {
        let mut st = self.state.lock().await;
        let replaces = st.chats.get(chat_jid).and_then(|n| n.call).unwrap_or(0);
        let id = self.server.show(note, replaces).await;
        let entry = st.chats.entry(chat_jid.to_string()).or_default();
        entry.call = (id != 0).then_some(id);
        if entry.is_empty() {
            st.chats.remove(chat_jid);
        }
    }

    /// Withdraw the chat's call notification (the call stopped ringing).
    pub async fn close_call(&self, chat_jid: &str) {
        let mut st = self.state.lock().await;
        let Some(n) = st.chats.get_mut(chat_jid) else {
            return;
        };
        let Some(id) = n.call.take() else { return };
        if n.is_empty() {
            st.chats.remove(chat_jid);
        }
        self.server.close(id).await;
    }

    /// One "N new messages in M chats" notification for a burst across many
    /// chats; it accumulates and replaces itself.
    pub async fn summary(&self, counts: &[(String, u32)]) {
        let mut st = self.state.lock().await;
        let (replaces, mut chats) = st.summary.take().unwrap_or_default();
        for (jid, n) in counts {
            *chats.entry(jid.clone()).or_default() += n;
        }
        let id = self
            .server
            .show(&DesktopNote::new("wa-desk", summary_body(&chats)), replaces)
            .await;
        st.summary = (id != 0).then_some((id, chats));
    }

    /// A notification not tied to a chat (automations). Never replaced.
    pub async fn other(&self, note: &DesktopNote) {
        let id = self.server.show(note, 0).await;
        self.state.lock().await.remember_misc(id);
    }

    /// Close every notification of `chat_jid` and drop it from the summary
    /// (the summary closes once none of its chats is left). Returns how many
    /// notifications were closed.
    pub async fn dismiss_chat(&self, chat_jid: &str) -> usize {
        let mut st = self.state.lock().await;
        let mut ids: Vec<u32> = st
            .chats
            .remove(chat_jid)
            .map(|n| n.ids().collect())
            .unwrap_or_default();
        if let Some((sid, chats)) = st.summary.as_mut()
            && chats.remove(chat_jid).is_some()
            && chats.is_empty()
        {
            ids.push(*sid);
            st.summary = None;
        }
        for id in &ids {
            self.server.close(*id).await;
        }
        ids.len()
    }

    /// Chats that currently have a notification (summary included).
    pub async fn active_chats(&self) -> Vec<String> {
        let st = self.state.lock().await;
        let mut out: Vec<String> = st.chats.keys().cloned().collect();
        if let Some((_, chats)) = &st.summary {
            out.extend(chats.keys().filter(|j| !st.chats.contains_key(*j)).cloned());
        }
        out.sort();
        out
    }

    /// Handle a server signal. A click on a chat's notification closes that
    /// chat's other notifications too.
    pub async fn on_event(&self, ev: ServerEvent) -> Activation {
        match ev {
            ServerEvent::Closed(id) => {
                self.state.lock().await.forget(id);
                Activation::Unknown
            }
            ServerEvent::Activated(id) => {
                let chat = {
                    let mut st = self.state.lock().await;
                    match st.chat_of(id) {
                        Some(c) => Some(c),
                        None if st.forget(id) => return Activation::App,
                        None => return Activation::Unknown,
                    }
                };
                match chat {
                    Some(c) => {
                        self.dismiss_chat(&c).await;
                        Activation::Chat(c)
                    }
                    None => Activation::Unknown,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex as PMutex;

    /// Fake server: ids count up from 1; replacing keeps the id.
    #[derive(Default)]
    struct Fake {
        next: PMutex<u32>,
        shown: PMutex<Vec<(u32, u32, DesktopNote)>>,
        closed: PMutex<Vec<u32>>,
        broken: PMutex<bool>,
    }

    #[async_trait]
    impl NotificationServer for Fake {
        async fn show(&self, note: &DesktopNote, replaces: u32) -> u32 {
            if *self.broken.lock() {
                return 0;
            }
            let id = if replaces != 0 {
                replaces
            } else {
                let mut n = self.next.lock();
                *n += 1;
                *n
            };
            self.shown.lock().push((id, replaces, note.clone()));
            id
        }
        async fn close(&self, id: u32) {
            self.closed.lock().push(id);
        }
    }

    fn center() -> (Arc<Fake>, NotificationCenter) {
        let f = Arc::new(Fake::default());
        (f.clone(), NotificationCenter::new(f))
    }

    #[tokio::test]
    async fn messages_replace_per_chat_and_count() {
        let (f, c) = center();
        c.message("a@s", "Alice", "one", 1).await;
        c.message("b@s", "Bob", "hey", 1).await;
        c.message("a@s", "Alice", "three", 2).await;
        let shown = f.shown.lock().clone();
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[0].0, 1);
        assert_eq!(shown[1].0, 2);
        assert_eq!(
            shown[2],
            (1, 1, DesktopNote::new("Alice (3 messages)", "three"))
        );
        assert_eq!(c.active_chats().await, vec!["a@s", "b@s"]);
    }

    #[tokio::test]
    async fn dismiss_closes_message_and_call_only_for_that_chat() {
        let (f, c) = center();
        c.message("a@s", "Alice", "hi", 1).await;
        c.call("a@s", &DesktopNote::new("Missed call", "from Alice"))
            .await;
        c.message("b@s", "Bob", "yo", 1).await;
        assert_eq!(c.dismiss_chat("a@s").await, 2);
        assert_eq!(*f.closed.lock(), vec![1, 2]);
        assert_eq!(c.active_chats().await, vec!["b@s"]);
        // Nothing left to close; a new message starts a fresh notification.
        assert_eq!(c.dismiss_chat("a@s").await, 0);
        c.message("a@s", "Alice", "again", 1).await;
        let last = f.shown.lock().last().cloned().unwrap();
        assert_eq!(last, (4, 0, DesktopNote::new("Alice", "again")));
    }

    #[tokio::test]
    async fn call_notification_replaces_and_closes() {
        let (f, c) = center();
        c.call(
            "a@s",
            &DesktopNote::new("Call from Alice", "Answer on your phone"),
        )
        .await;
        c.call("a@s", &DesktopNote::new("Missed call", "from Alice"))
            .await;
        assert_eq!(f.shown.lock()[1].1, 1, "missed replaces ringing");
        c.close_call("a@s").await;
        assert_eq!(*f.closed.lock(), vec![1]);
        assert!(c.active_chats().await.is_empty());
        c.close_call("a@s").await;
        assert_eq!(f.closed.lock().len(), 1);
    }

    #[tokio::test]
    async fn click_opens_chat_and_clears_siblings() {
        let (f, c) = center();
        c.message("a@s", "Alice", "hi", 1).await;
        c.call("a@s", &DesktopNote::new("Missed call", "from Alice"))
            .await;
        assert_eq!(
            c.on_event(ServerEvent::Activated(2)).await,
            Activation::Chat("a@s".into())
        );
        assert_eq!(*f.closed.lock(), vec![1, 2]);
        assert_eq!(
            c.on_event(ServerEvent::Activated(77)).await,
            Activation::Unknown,
            "other apps' notifications are ignored"
        );
    }

    #[tokio::test]
    async fn server_closed_forgets_so_next_message_is_new() {
        let (f, c) = center();
        c.message("a@s", "Alice", "hi", 1).await;
        c.on_event(ServerEvent::Closed(1)).await;
        assert!(c.active_chats().await.is_empty());
        c.message("a@s", "Alice", "hi again", 1).await;
        let last = f.shown.lock().last().cloned().unwrap();
        assert_eq!(last, (2, 0, DesktopNote::new("Alice", "hi again")));
        assert!(f.closed.lock().is_empty());
    }

    #[tokio::test]
    async fn summary_accumulates_and_closes_with_its_last_chat() {
        let (f, c) = center();
        c.summary(&[("a@s".into(), 2), ("b@s".into(), 1)]).await;
        c.summary(&[("c@s".into(), 1)]).await;
        let shown = f.shown.lock().clone();
        assert_eq!(
            shown[1],
            (
                1,
                1,
                DesktopNote::new("wa-desk", "4 new messages in 3 chats")
            )
        );
        assert_eq!(c.active_chats().await, vec!["a@s", "b@s", "c@s"]);
        c.dismiss_chat("a@s").await;
        c.dismiss_chat("b@s").await;
        assert!(f.closed.lock().is_empty(), "still covers c@s");
        c.dismiss_chat("c@s").await;
        assert_eq!(*f.closed.lock(), vec![1]);
    }

    #[tokio::test]
    async fn untracked_server_is_harmless() {
        let (f, c) = center();
        *f.broken.lock() = true;
        c.message("a@s", "Alice", "hi", 1).await;
        c.other(&DesktopNote::new("Rule fired", "x")).await;
        assert!(c.active_chats().await.is_empty());
        assert_eq!(c.dismiss_chat("a@s").await, 0);
        assert!(f.closed.lock().is_empty());
    }

    #[tokio::test]
    async fn click_on_other_notification_shows_app() {
        let (_f, c) = center();
        c.other(&DesktopNote::new("Rule fired", "x")).await;
        assert_eq!(c.on_event(ServerEvent::Activated(1)).await, Activation::App);
    }

    #[test]
    fn text_helpers() {
        assert_eq!(counted_title("Alice", 1), "Alice");
        assert_eq!(counted_title("Alice", 2), "Alice (2 messages)");
        let mut m = BTreeMap::new();
        m.insert("a".to_string(), 1);
        assert_eq!(summary_body(&m), "1 new message in 1 chat");
    }
}
