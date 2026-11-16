//! Archive, pin and mute: `archive_count`, `set_archived`, `set_pinned`,
//! `set_muted` and `resync_chat_state`.
//!
//! The bridge syncs changes to the phone and records them in `chat_state`;
//! wa-desk only reads that table. After each successful write the chat is
//! announced with `db:changed` so the UI re-reads it.

use wa_core::types::{DbChangedEvent, events};
use wa_core::{AppError, ErrorKind, Result};

use super::{AppCore, blocking, emit_typed, friendly_bridge, require};

/// WhatsApp lets you pin at most this many chats.
pub const MAX_PINNED: u32 = 3;

impl AppCore {
    /// `archive_count`: archived chats (for the "Archived" row).
    pub async fn archive_count(&self) -> Result<u32> {
        let db = self.db.clone();
        blocking(move || db.archive_count()).await
    }

    /// `set_archived`: archive or unarchive a chat on the phone too.
    pub async fn set_archived(&self, jid: String, archived: bool) -> Result<()> {
        self.writable()?;
        require("chat", &jid)?;
        self.bridge
            .set_archived(&jid, archived)
            .await
            .map_err(|e| friendly_bridge(e, "the chat was not archived"))?;
        self.announce_chat(jid);
        Ok(())
    }

    /// `set_pinned`: at most [`MAX_PINNED`] chats (`Invalid` beyond that).
    pub async fn set_pinned(&self, jid: String, pinned: bool) -> Result<()> {
        self.writable()?;
        require("chat", &jid)?;
        if pinned {
            let db = self.db.clone();
            let j = jid.clone();
            let (count, already) =
                blocking(move || Ok((db.pinned_count()?, db.get_chat(&j).is_ok_and(|c| c.pinned))))
                    .await?;
            if !already && count >= MAX_PINNED {
                return Err(AppError::invalid("You can only pin up to 3 chats"));
            }
        }
        self.bridge
            .set_pinned(&jid, pinned)
            .await
            .map_err(|e| friendly_bridge(e, "the chat was not pinned"))?;
        self.announce_chat(jid);
        Ok(())
    }

    /// `set_muted`: `until` epoch seconds, `-1` always, `None` unmute.
    pub async fn set_muted(&self, jid: String, until: Option<i64>) -> Result<()> {
        self.writable()?;
        require("chat", &jid)?;
        self.bridge
            .set_muted(&jid, until)
            .await
            .map_err(|e| friendly_bridge(e, "the chat was not muted"))?;
        self.announce_chat(jid);
        Ok(())
    }

    /// `resync_chat_state`: replay app state; returns how many chats changed.
    pub async fn resync_chat_state(&self) -> Result<u32> {
        self.writable()?;
        let stats = self
            .bridge
            .resync_chat_state()
            .await
            .map_err(|e| friendly_bridge(e, "chat state can't be synced right now"))?;
        tracing::info!(
            changed = stats.changed,
            cached = stats.cached,
            "chat state resynced"
        );
        if stats.changed > 0 {
            self.announce_all().await?;
        }
        Ok(stats.changed)
    }

    /// One-time startup resync: a down, old or read-only bridge is not an error.
    pub async fn resync_state_tick(&self) -> Result<()> {
        match self.resync_chat_state().await {
            Ok(_) => Ok(()),
            Err(e)
                if matches!(
                    e.kind,
                    ErrorKind::BridgeDown
                        | ErrorKind::NotFound
                        | ErrorKind::Timeout
                        | ErrorKind::Forbidden
                ) =>
            {
                tracing::info!(kind = ?e.kind, "chat state resync skipped");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn announce_chat(&self, jid: String) {
        emit_typed(
            &*self.emitter,
            events::DB_CHANGED,
            &DbChangedEvent {
                chat_jids: vec![jid],
            },
        );
    }

    /// `db:changed` for every chat (archived ones included).
    async fn announce_all(&self) -> Result<()> {
        let db = self.db.clone();
        let jids = blocking(move || {
            let mut out = Vec::new();
            for archived in [false, true] {
                let mut q = wa_core::types::ChatQuery {
                    limit: Some(500),
                    archived,
                    ..Default::default()
                };
                loop {
                    let p = db.list_chats(&q)?;
                    out.extend(p.items.into_iter().map(|c| c.jid));
                    match p.next_cursor {
                        Some(c) => q.cursor = Some(c),
                        None => break,
                    }
                }
            }
            Ok(out)
        })
        .await?;
        emit_typed(
            &*self.emitter,
            events::DB_CHANGED,
            &DbChangedEvent { chat_jids: jids },
        );
        Ok(())
    }
}
