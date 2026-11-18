//! Profiles (`get_profile`), avatars for `wa-media://localhost/a/…` and
//! `refresh_chat_names`.
//!
//! Live data comes from the bridge and is merged with local names; when the
//! bridge is down (or lacks the endpoint, or we are not a member) a local
//! fallback profile is returned instead of an error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wa_core::traits::{BridgeGroup, BridgeNewsletter, BridgeProfile};
use wa_core::types::{
    BusinessInfo, ChannelProfile, ChatRef, ChatType, ContactProfile, DbChangedEvent, GroupProfile,
    Participant, Profile, events,
};
use wa_core::{AppError, ErrorKind, Result};
use wa_db::NameCache;
use wa_db::media::avatar_url;

use super::{AppCore, blocking, emit_typed, require};

/// How long a live profile is reused.
pub const PROFILE_TTL: Duration = Duration::from_secs(600);
/// Longest wait for a live profile before falling back to local data.
pub const PROFILE_WAIT: Duration = Duration::from_secs(5);
/// Longest wait for the bridge when serving an avatar.
pub const AVATAR_WAIT: Duration = Duration::from_secs(2);
/// "No picture" answers are remembered this long.
pub const AVATAR_NONE_TTL: Duration = Duration::from_secs(30 * 60);
/// After a bridge timeout/failure the avatar is retried after this long.
pub const AVATAR_RETRY: Duration = Duration::from_secs(120);
/// Found avatars are re-checked with the bridge after this long.
pub const AVATAR_FOUND_TTL: Duration = Duration::from_secs(3600);

/// In-memory avatar cache entry.
#[derive(Debug, Clone)]
pub(crate) enum AvatarEntry {
    Found(PathBuf, Instant),
    /// The chat has no picture (until then).
    Missing(Instant),
    /// The bridge failed or timed out and nothing is cached (retry after).
    Unavailable(Instant),
}

/// `NotFound` message for a chat that has no picture (cacheable).
pub const NO_PICTURE: &str = "no picture";
/// `NotFound` message when the picture could not be fetched right now
/// (bridge offline, slow or failing; not cacheable).
pub const PICTURE_UNAVAILABLE: &str = "picture unavailable";

/// Per-process caches for profiles and avatars.
#[derive(Default)]
pub(crate) struct ProfileCaches {
    pub profiles: parking_lot::Mutex<HashMap<String, (Instant, Profile)>>,
    pub avatars: parking_lot::Mutex<HashMap<(String, bool), AvatarEntry>>,
}

/// "+91 98765 43210" for Indian mobiles, "+<digits>" otherwise.
pub fn format_phone(jid: &str) -> Option<String> {
    let (user, server) = jid.split_once('@').unwrap_or((jid, "s.whatsapp.net"));
    if server != "s.whatsapp.net" {
        return None;
    }
    let digits: String = user.split(':').next().unwrap_or(user).to_string();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if digits.len() == 12 && digits.starts_with("91") {
        return Some(format!("+91 {} {}", &digits[2..7], &digits[7..]));
    }
    Some(format!("+{digits}"))
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn contact_profile(
    names: &NameCache,
    jid: &str,
    live: Option<BridgeProfile>,
    common: Vec<ChatRef>,
) -> ContactProfile {
    let norm = names.normalize(jid).unwrap_or_else(|| jid.to_string());
    let local = names.chat_name(&norm);
    let live = live.unwrap_or_default();
    let push = nonempty(live.push_name);
    let verified = nonempty(live.verified_name);
    // A phone-number fallback is replaced by what the person or business calls itself.
    let name = if local.starts_with('+') || local == norm {
        verified.clone().or_else(|| push.clone()).unwrap_or(local)
    } else {
        local
    };
    ContactProfile {
        jid: norm.clone(),
        name,
        push_name: push,
        phone: format_phone(&norm),
        about: nonempty(live.about),
        avatar_url: avatar_url(&norm),
        is_me: names.is_own(&norm),
        business: live.business.map(|b| BusinessInfo {
            description: nonempty(b.description),
            category: nonempty(b.category),
            email: nonempty(b.email),
            website: nonempty(b.website),
            address: nonempty(b.address),
        }),
        common_groups: common,
    }
}

fn participant(names: &NameCache, p: wa_core::traits::BridgeParticipant) -> Participant {
    let key = nonempty(p.phone_jid.clone())
        .or_else(|| nonempty(Some(p.jid.clone())))
        .unwrap_or_default();
    let norm = names.normalize(&key).unwrap_or_else(|| key.clone());
    let is_me = names.is_own(&key) || names.is_own(&p.jid);
    let resolved = names.sender_name(&norm, is_me);
    let fallback = resolved.starts_with('+') || resolved == norm || resolved == key;
    let name = match nonempty(p.display_name) {
        Some(d) if fallback && !is_me => d,
        _ => resolved,
    };
    Participant {
        phone: format_phone(&norm),
        avatar_url: avatar_url(&norm),
        jid: norm,
        name,
        is_admin: p.is_admin || p.is_super_admin,
        is_super_admin: p.is_super_admin,
        is_me,
    }
}

fn group_profile(names: &NameCache, jid: &str, live: Option<BridgeGroup>) -> GroupProfile {
    let meta = names.meta(jid);
    let mut chat_type = names.chat_type(jid);
    let mut g = GroupProfile {
        jid: jid.to_string(),
        name: names.chat_name(jid),
        chat_type,
        description: meta.and_then(|m| m.description.clone()),
        created_at: None,
        created_by: None,
        participants: Vec::new(),
        participant_count: 0,
        parent: names.parent(jid),
        sub_groups: if chat_type == ChatType::Community {
            names.children(jid)
        } else {
            Vec::new()
        },
        announce_only: false,
        locked: false,
        avatar_url: avatar_url(jid),
    };
    let Some(live) = live else {
        return g;
    };
    if live.is_parent {
        chat_type = ChatType::Community;
    }
    g.chat_type = chat_type;
    let local_is_placeholder = g.name == jid.split('@').next().unwrap_or(jid)
        || g.name.starts_with("Group ")
            && g.name[6..].chars().all(|c| c.is_ascii_digit() || c == '-');
    if local_is_placeholder && let Some(n) = nonempty(live.name.clone()) {
        g.name = n;
    }
    g.description = nonempty(live.topic).or(g.description);
    g.created_at = live
        .created_at
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| d.timestamp());
    g.created_by = nonempty(live.creator).map(|c| names.sender_name(&c, names.is_own(&c)));
    let mut ps: Vec<Participant> = live
        .participants
        .into_iter()
        .map(|p| participant(names, p))
        .collect();
    ps.sort_by(|a, b| {
        (!a.is_me, !a.is_admin, a.name.to_lowercase()).cmp(&(
            !b.is_me,
            !b.is_admin,
            b.name.to_lowercase(),
        ))
    });
    g.participant_count = ps.len() as u32;
    g.participants = ps;
    if let Some(p) = nonempty(live.linked_parent_jid) {
        g.parent = Some(names.chat_ref(&p));
    }
    if live.is_parent {
        let mut subs: Vec<ChatRef> = live
            .sub_groups
            .into_iter()
            .filter(|s| !s.jid.is_empty())
            .map(|s| {
                let mut r = names.chat_ref(&s.jid);
                if r.name == s.jid.split('@').next().unwrap_or(&s.jid)
                    && let Some(n) = nonempty(s.name)
                {
                    r.name = n;
                }
                r.chat_type = ChatType::Group;
                r
            })
            .collect();
        subs.sort_by_key(|r| r.name.to_lowercase());
        if !subs.is_empty() {
            g.sub_groups = subs;
        }
    }
    g.announce_only = live.announce;
    g.locked = live.locked;
    g
}

fn channel_profile(names: &NameCache, jid: &str, live: Option<BridgeNewsletter>) -> ChannelProfile {
    let meta = names.meta(jid);
    let local = names.chat_name(jid);
    let live = live.unwrap_or_default();
    let name =
        if names.meta(jid).and_then(|m| m.name.as_ref()).is_none() && local == "WhatsApp channel" {
            nonempty(live.name).unwrap_or(local)
        } else {
            local
        };
    ChannelProfile {
        jid: jid.to_string(),
        name,
        description: nonempty(live.description)
            .or_else(|| meta.and_then(|m| m.description.clone())),
        subscribers: live.subscribers,
        verified: live.verified,
        avatar_url: avatar_url(jid),
    }
}

/// Bridge answer within `wait`, or `None` (logged) on any failure.
async fn live<T>(
    what: &str,
    wait: Duration,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Option<T> {
    match tokio::time::timeout(wait, fut).await {
        Ok(Ok(v)) => Some(v),
        Ok(Err(e)) => {
            tracing::debug!(what, kind = ?e.kind, "live profile unavailable; using local data");
            None
        }
        Err(_) => {
            tracing::debug!(what, "live profile timed out; using local data");
            None
        }
    }
}

impl AppCore {
    /// `get_profile`: contact, group/community or channel; 10 min cache.
    pub async fn get_profile(&self, jid: String) -> Result<Profile> {
        require("chat", &jid)?;
        let jid = jid.trim().to_string();
        if let Some((at, p)) = self.caches.profiles.lock().get(&jid)
            && at.elapsed() < PROFILE_TTL
        {
            return Ok(p.clone());
        }
        let db = self.db.clone();
        let names: Arc<NameCache> = blocking(move || db.names().snapshot()).await?;
        let ty = names.chat_type(&jid);
        let (profile, from_bridge) = match ty {
            ChatType::Direct => {
                let norm = names.normalize(&jid).unwrap_or_else(|| jid.clone());
                let lv = live("profile", PROFILE_WAIT, self.bridge.profile(&norm)).await;
                let ok = lv.is_some();
                let (db, n2) = (self.db.clone(), norm.clone());
                let common = blocking(move || db.common_groups(&n2))
                    .await
                    .unwrap_or_default();
                (
                    Profile::Contact(contact_profile(&names, &norm, lv, common)),
                    ok,
                )
            }
            ChatType::Group | ChatType::Community => {
                let lv = live("group", PROFILE_WAIT, self.bridge.group(&jid)).await;
                let ok = lv.is_some();
                (Profile::Group(group_profile(&names, &jid, lv)), ok)
            }
            ChatType::Channel => {
                let lv = live("channel", PROFILE_WAIT, self.bridge.newsletter(&jid)).await;
                let ok = lv.is_some();
                (Profile::Channel(channel_profile(&names, &jid, lv)), ok)
            }
            ChatType::Broadcast | ChatType::Status => {
                (Profile::Group(group_profile(&names, &jid, None)), false)
            }
        };
        if from_bridge {
            self.caches
                .profiles
                .lock()
                .insert(jid, (Instant::now(), profile.clone()));
        }
        Ok(profile)
    }

    /// Local avatar file for the protocol handler. Never waits on the bridge
    /// for more than [`AVATAR_WAIT`] per lookup; `NotFound` when there is no
    /// picture.
    ///
    /// A community group without a picture of its own shows its community's
    /// picture, as WhatsApp does for a community's announcement group (which
    /// never has one: it carries the community's name and logo).
    pub async fn avatar_file(&self, jid: &str, full: bool) -> Result<PathBuf> {
        let own = self.own_avatar_file(jid, full).await;
        match own {
            Err(e) if e.kind == ErrorKind::NotFound && e.message == NO_PICTURE => {
                match self.community_of(jid).await {
                    Some(parent) => self.own_avatar_file(&parent, full).await,
                    None => Err(e),
                }
            }
            other => other,
        }
    }

    /// The community a group belongs to, if any (from the bridge's chat_meta).
    async fn community_of(&self, jid: &str) -> Option<String> {
        if ChatType::from_jid(jid) != ChatType::Group {
            return None;
        }
        let (db, j) = (self.db.clone(), jid.to_string());
        blocking(move || Ok(db.names().snapshot()?.parent(&j).map(|p| p.jid)))
            .await
            .ok()
            .flatten()
            .filter(|p| p != jid)
    }

    /// The chat's own picture (see [`Self::avatar_file`]). A definitive "no
    /// picture" is `NotFound` with [`NO_PICTURE`]; a bridge failure without a
    /// cached file is `NotFound` with [`PICTURE_UNAVAILABLE`].
    async fn own_avatar_file(&self, jid: &str, full: bool) -> Result<PathBuf> {
        if matches!(
            ChatType::from_jid(jid),
            ChatType::Status | ChatType::Broadcast
        ) {
            // These never have a picture; don't make the UI wait for the bridge.
            return Err(AppError::not_found(NO_PICTURE));
        }
        let key = (jid.to_string(), full);
        let cached = self.caches.avatars.lock().get(&key).cloned();
        match cached {
            Some(AvatarEntry::Found(p, at)) if at.elapsed() < AVATAR_FOUND_TTL && p.is_file() => {
                return Ok(p);
            }
            Some(AvatarEntry::Missing(until)) if Instant::now() < until => {
                return Err(AppError::not_found(NO_PICTURE));
            }
            Some(AvatarEntry::Unavailable(until)) if Instant::now() < until => {
                return Err(AppError::not_found(PICTURE_UNAVAILABLE));
            }
            _ => {}
        }
        let res = tokio::time::timeout(AVATAR_WAIT, self.bridge.avatar(jid, full)).await;
        let (entry, out) = match res {
            Ok(Ok(a)) => match self.scoped_avatar(&a.path) {
                Ok(p) => (AvatarEntry::Found(p.clone(), Instant::now()), Ok(p)),
                Err(e) => {
                    tracing::warn!(error = %e, "bridge avatar path rejected");
                    (
                        AvatarEntry::Missing(Instant::now() + AVATAR_NONE_TTL),
                        Err(AppError::not_found(NO_PICTURE)),
                    )
                }
            },
            Ok(Err(e)) if e.kind == ErrorKind::NotFound && e.message.contains(NO_PICTURE) => (
                AvatarEntry::Missing(Instant::now() + AVATAR_NONE_TTL),
                Err(AppError::not_found(NO_PICTURE)),
            ),
            other => {
                // Down, timed out, old bridge: serve a file the bridge cached earlier.
                if let Ok(Err(e)) = &other {
                    tracing::debug!(kind = ?e.kind, "avatar lookup failed");
                }
                match self.disk_avatar(jid, full) {
                    Some(p) => (AvatarEntry::Found(p.clone(), Instant::now()), Ok(p)),
                    None => (
                        AvatarEntry::Unavailable(Instant::now() + AVATAR_RETRY),
                        Err(AppError::not_found(PICTURE_UNAVAILABLE)),
                    ),
                }
            }
        };
        self.caches.avatars.lock().insert(key, entry);
        out
    }

    fn scoped_avatar(&self, p: &Path) -> Result<PathBuf> {
        let real =
            wa_core::paths::resolve_within(p, std::slice::from_ref(&self.paths.bridge_store))?;
        let ok_ext = real.extension().is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().to_str(),
                Some("jpg" | "jpeg" | "png" | "webp")
            )
        });
        if !ok_ext || !real.is_file() {
            return Err(AppError::forbidden("not an image file"));
        }
        Ok(real)
    }

    /// `store/avatars/<jid with ':'→'_'>[-full].jpg`, if present.
    fn disk_avatar(&self, jid: &str, full: bool) -> Option<PathBuf> {
        if jid.contains('/') || jid.contains("..") {
            return None;
        }
        let name = format!(
            "{}{}.jpg",
            jid.replace(':', "_"),
            if full { "-full" } else { "" }
        );
        self.scoped_avatar(&self.paths.bridge_store.join("avatars").join(name))
            .ok()
    }

    /// `refresh_chat_names`: ask the bridge to refresh group/community/channel
    /// metadata, reload names and tell the UI everything may have changed.
    pub async fn refresh_chat_names(&self) -> Result<u32> {
        let stats =
            self.bridge.refresh_chats().await.map_err(|e| {
                super::friendly_bridge(e, "chat names can't be refreshed right now")
            })?;
        self.db.names().invalidate();
        self.caches.profiles.lock().clear();
        let db = self.db.clone();
        let jids: Vec<String> = blocking(move || {
            let mut q = wa_core::types::ChatQuery {
                limit: Some(500),
                ..Default::default()
            };
            let mut out = Vec::new();
            loop {
                let p = db.list_chats(&q)?;
                out.extend(p.items.into_iter().map(|c| c.jid));
                match p.next_cursor {
                    Some(c) => q.cursor = Some(c),
                    None => return Ok(out),
                }
            }
        })
        .await?;
        emit_typed(
            &*self.emitter,
            events::DB_CHANGED,
            &DbChangedEvent { chat_jids: jids },
        );
        tracing::info!(
            groups = stats.groups,
            communities = stats.communities,
            channels = stats.channels,
            renamed = stats.renamed,
            cached = stats.cached,
            "chat names refreshed"
        );
        Ok(stats.renamed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phones_format() {
        assert_eq!(
            format_phone("919876543210@s.whatsapp.net").as_deref(),
            Some("+91 98765 43210")
        );
        assert_eq!(
            format_phone("14155550100@s.whatsapp.net").as_deref(),
            Some("+14155550100")
        );
        assert_eq!(format_phone("2222@lid"), None);
        assert_eq!(format_phone("1203@g.us"), None);
    }
}
