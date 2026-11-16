//! `forward_messages`: re-send text and media to up to five chats.

use std::path::Path;

use wa_core::traits::BridgeSend;
use wa_core::types::{ForwardReq, Message, SendResult};
use wa_core::{AppError, Result};

use super::{AppCore, blocking, friendly_bridge, recipient_jid, require};

/// Most target chats per call (WhatsApp's own limit).
pub const FORWARD_MAX_TARGETS: usize = 5;
/// Most messages forwarded per call.
pub const FORWARD_MAX_MESSAGES: usize = 50;

fn failed(msg: impl Into<String>) -> SendResult {
    SendResult {
        ok: false,
        message: msg.into(),
    }
}

impl AppCore {
    /// `forward_messages`: one `SendResult` per message × target, in order
    /// (messages ascending, then targets as given).
    pub async fn forward_messages(&self, req: ForwardReq) -> Result<Vec<SendResult>> {
        self.writable()?;
        require("chat", &req.from_chat)?;
        let mut targets: Vec<String> = Vec::new();
        for t in &req.to {
            let j = recipient_jid(t)?;
            if !targets.contains(&j) {
                targets.push(j);
            }
        }
        if targets.is_empty() {
            return Err(AppError::invalid("Choose at least one chat to forward to"));
        }
        if targets.len() > FORWARD_MAX_TARGETS {
            return Err(AppError::invalid(
                "You can forward to at most 5 chats at a time",
            ));
        }
        if req.message_ids.is_empty() {
            return Err(AppError::invalid("Select at least one message"));
        }
        if req.message_ids.len() > FORWARD_MAX_MESSAGES {
            return Err(AppError::invalid(
                "You can forward at most 50 messages at a time",
            ));
        }
        let (db, chat, ids) = (
            self.db.clone(),
            req.from_chat.clone(),
            req.message_ids.clone(),
        );
        let msgs = blocking(move || db.messages_by_ids(&chat, &ids)).await?;
        if msgs.is_empty() {
            return Err(AppError::not_found("Message not found"));
        }
        let mut out = Vec::with_capacity(msgs.len() * targets.len());
        for m in &msgs {
            for t in &targets {
                out.push(self.forward_one(m, t).await);
            }
        }
        let ok = out.iter().filter(|r| r.ok).count();
        tracing::info!(
            messages = msgs.len(),
            targets = targets.len(),
            ok,
            "messages forwarded"
        );
        Ok(out)
    }

    async fn forward_one(&self, m: &Message, target: &str) -> SendResult {
        if m.deleted {
            return failed("A deleted message can't be forwarded");
        }
        let text = Some(m.content.clone()).filter(|t| !t.trim().is_empty());
        let mut send = BridgeSend {
            recipient: target.to_string(),
            message: text,
            ..Default::default()
        };
        if m.media.is_some() {
            let src = match self.local_media(&m.chat_jid, &m.id).await {
                Ok((_, p)) => p,
                Err(e) => return failed(format!("The file is not available: {}", e.message)),
            };
            let paths = self.paths.clone();
            match blocking(move || wa_bridge::outbox::stage(&paths, &src)).await {
                Ok(p) => send.media_path = Some(p),
                Err(e) => return failed(format!("Could not prepare the file: {}", e.message)),
            }
        } else if send.message.is_none() {
            return failed("Nothing to forward");
        }
        let staged = send.media_path.clone();
        match self.bridge.send(send).await {
            Ok(message) => SendResult { ok: true, message },
            Err(e) => {
                if let Some(dir) = staged.as_deref().and_then(Path::parent)
                    && dir.starts_with(&self.paths.outbox)
                    && dir != self.paths.outbox
                {
                    let _ = std::fs::remove_dir_all(dir);
                }
                failed(friendly_bridge(e, "the message was not forwarded").message)
            }
        }
    }
}
