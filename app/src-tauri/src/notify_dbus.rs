//! Desktop notifications through the freedesktop notification service
//! (`org.freedesktop.Notifications` on the session bus, as used by GNOME and
//! KDE). Unlike `tauri-plugin-notification` it keeps the server's ids, so a
//! chat's notification can be replaced (`replaces_id`) and withdrawn
//! (`CloseNotification`), and it reports clicks (`ActionInvoked`) and closes
//! (`NotificationClosed`) as [`ServerEvent`]s.
//!
//! Every call is a short blocking D-Bus round trip with a 3 s timeout, run on
//! the blocking pool. Signals are read on one dedicated thread.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::Value;

use crate::core::{DesktopNote, NotificationServer, ServerEvent};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";
/// Longest wait for the notification service.
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
/// Name used as the sender and for the `desktop-entry` hint (wa-desk.desktop).
const APP: &str = "wa-desk";

/// Notifications over the session bus.
pub struct DbusNotifications {
    conn: Connection,
}

impl DbusNotifications {
    /// Connect to the session bus and start the signal reader, which forwards
    /// clicks and closes to `events`. `None` when there is no session bus or
    /// no notification service.
    pub fn connect(events: UnboundedSender<ServerEvent>) -> Option<Self> {
        let conn = match zbus::blocking::connection::Builder::session()
            .map(|b| b.method_timeout(CALL_TIMEOUT))
            .and_then(|b| b.build())
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "no session bus; notifications cannot be withdrawn");
                return None;
            }
        };
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(IFACE)
            .and_then(|b| b.path(PATH))
            .map(|b| b.build());
        let iter = match rule.and_then(|r| MessageIterator::for_match_rule(r, &conn, Some(256))) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(error = %e, "cannot subscribe to notification signals");
                return None;
            }
        };
        let spawned = std::thread::Builder::new()
            .name("notify-signals".into())
            .spawn(move || read_signals(iter, events));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "cannot start the notification signal reader");
            return None;
        }
        tracing::info!("desktop notifications via org.freedesktop.Notifications");
        Some(Self { conn })
    }

    fn proxy(&self) -> zbus::Result<Proxy<'static>> {
        Proxy::new(&self.conn, DEST, PATH, IFACE)
    }

    fn notify_blocking(&self, note: &DesktopNote, replaces: u32) -> zbus::Result<u32> {
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("desktop-entry", Value::from(APP));
        hints.insert("category", Value::from("im.received"));
        // "default" is the click on the notification body.
        let actions: &[&str] = &["default", "Open"];
        self.proxy()?.call(
            "Notify",
            &(
                APP,
                replaces,
                APP,
                note.title.as_str(),
                escape_markup(&note.body).as_str(),
                actions,
                hints,
                -1i32,
            ),
        )
    }
}

/// Servers with `body-markup` (GNOME) parse the body as a small HTML subset,
/// so message text like "a < b & c" must be escaped to show as typed.
pub fn escape_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Forward `ActionInvoked` / `NotificationClosed` until the receiver is gone.
fn read_signals(iter: MessageIterator, events: UnboundedSender<ServerEvent>) {
    for msg in iter {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        let ev = match header.member().map(|m| m.as_str()) {
            Some("ActionInvoked") => msg
                .body()
                .deserialize::<(u32, String)>()
                .ok()
                .map(|(id, _action)| ServerEvent::Activated(id)),
            Some("NotificationClosed") => msg
                .body()
                .deserialize::<(u32, u32)>()
                .ok()
                .map(|(id, _reason)| ServerEvent::Closed(id)),
            _ => None,
        };
        if let Some(ev) = ev
            && events.send(ev).is_err()
        {
            return;
        }
    }
}

#[async_trait]
impl NotificationServer for DbusNotifications {
    async fn show(&self, note: &DesktopNote, replaces: u32) -> u32 {
        let (conn, note) = (self.conn.clone(), note.clone());
        let r = tokio::task::spawn_blocking(move || {
            DbusNotifications { conn }.notify_blocking(&note, replaces)
        })
        .await;
        match r {
            Ok(Ok(id)) => id,
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "notification failed");
                0
            }
            Err(e) => {
                tracing::debug!(error = %e, "notification task failed");
                0
            }
        }
    }

    async fn close(&self, id: u32) {
        if id == 0 {
            return;
        }
        let conn = self.conn.clone();
        let r = tokio::task::spawn_blocking(move || {
            Proxy::new(&conn, DEST, PATH, IFACE)?.call::<_, _, ()>("CloseNotification", &(id,))
        })
        .await;
        if let Ok(Err(e)) = r {
            // Already gone (expired or dismissed) is not worth more than debug.
            tracing::debug!(error = %e, "close notification failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::escape_markup;

    #[test]
    fn body_markup_is_escaped() {
        assert_eq!(escape_markup("a <b> & c"), "a &lt;b&gt; &amp; c");
        assert_eq!(escape_markup("📷 Photo"), "📷 Photo");
    }

    /// Manual check against the real session bus (briefly shows a notification):
    /// `cargo test -p wa-desk --lib live_show_replace_close -- --ignored`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a desktop session"]
    async fn live_show_replace_close() {
        use super::DbusNotifications;
        use crate::core::{DesktopNote, NotificationServer, ServerEvent};
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let d = DbusNotifications::connect(tx).expect("session bus");
        let id = d
            .show(&DesktopNote::new("wa-desk test", "first & <one>"), 0)
            .await;
        assert_ne!(id, 0);
        let again = d
            .show(&DesktopNote::new("wa-desk test (2)", "second"), id)
            .await;
        assert_eq!(again, id, "replaced in place");
        d.close(id).await;
        let ev = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Some(ServerEvent::Closed(c)) if c == id => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await;
        assert_eq!(ev, Ok(true), "NotificationClosed for our id");
    }
}
