//! Display names and normalised JIDs.
//!
//! `messages.sender` is written inconsistently by the bridge: bare phone
//! (`9198…`), phone JID (`9198…@s.whatsapp.net`), bare LID, LID JID
//! (`2222…@lid`), sometimes with a `:device` suffix. `NameCache` normalises all
//! of them to `pn@s.whatsapp.net` when a phone number is known (via
//! `whatsmeow_lid_map`), else `lid@lid`.
//!
//! Name priority for a person: a real `chats.name` (not a placeholder), then
//! whatsmeow contacts `full_name > push_name > first_name > business_name`
//! (looked up under both the phone and the LID JID), then `+<phone>`, then the
//! raw value. Our own account (from `whatsmeow_device`) is always "Me".
//!
//! Everything is loaded once into memory (a few thousand rows, paged reads)
//! and cached until `invalidate()` or a 5 minute TTL. `whatsapp.db` may be
//! missing: names then degrade to `chats.name` and phone numbers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use wa_core::Result;
use wa_core::types::{ChatRef, ChatType};

use crate::bridge_ro::{PAGE_ROWS, RoPool};

const TTL: Duration = Duration::from_secs(300);
/// Server part of phone-number JIDs.
pub const PN_SERVER: &str = "s.whatsapp.net";
/// The status broadcast pseudo-chat.
pub const STATUS_JID: &str = "status@broadcast";

/// `(user, server)` of a JID with any `:device` / `.agent` suffix removed.
pub fn split_jid(raw: &str) -> (&str, &str) {
    let raw = raw.trim();
    let (user, server) = raw.split_once('@').unwrap_or((raw, ""));
    let user = if matches!(server, "" | PN_SERVER | "lid" | "c.us") {
        let u = user.split(':').next().unwrap_or(user);
        u.split('.').next().unwrap_or(u)
    } else {
        user
    };
    (user, server)
}

/// True for group chats.
pub fn is_group_jid(jid: &str) -> bool {
    jid.ends_with("@g.us")
}

/// True for status/broadcast/newsletter pseudo-chats.
pub fn is_status_jid(jid: &str) -> bool {
    jid.ends_with("@broadcast") || jid.ends_with("@newsletter")
}

/// True when a stored chat name is one of the bridge's fallbacks, not a real name.
pub fn is_placeholder_name(jid: &str, name: Option<&str>) -> bool {
    let Some(name) = name.map(str::trim) else {
        return true;
    };
    if name.is_empty() {
        return true;
    }
    let (user, _) = split_jid(jid);
    let raw_user = jid.split('@').next().unwrap_or(jid);
    if name == user || name == raw_user || name == jid {
        return true;
    }
    if name == format!("Group {raw_user}") {
        return true;
    }
    let digitish = name
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-'));
    digitish && name.chars().any(|c| c.is_ascii_digit())
}

/// In-memory snapshot of everything needed to resolve names.
#[derive(Debug, Default, Clone)]
pub struct NameCache {
    lid_to_pn: HashMap<String, String>,
    pn_to_lid: HashMap<String, String>,
    /// Normalised their_jid → best contact name.
    contacts: HashMap<String, String>,
    /// chats.jid → stored chats.name (may be a placeholder).
    chat_names: HashMap<String, Option<String>>,
    own_users: HashSet<String>,
    own_pn: Option<String>,
    /// Bridge `chat_meta` rows (empty when the table does not exist yet).
    meta: HashMap<String, ChatMeta>,
}

/// One bridge `chat_meta` row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatMeta {
    pub chat_type: Option<ChatType>,
    pub parent_jid: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Map a stored `chat_meta.chat_type`.
pub fn chat_type_from_meta(s: &str) -> Option<ChatType> {
    match s.trim() {
        "group" => Some(ChatType::Group),
        "community" => Some(ChatType::Community),
        "channel" | "newsletter" => Some(ChatType::Channel),
        "broadcast" => Some(ChatType::Broadcast),
        "status" => Some(ChatType::Status),
        "direct" | "user" => Some(ChatType::Direct),
        _ => None,
    }
}

impl NameCache {
    /// Normalised JID for any sender form; `None` for an empty value.
    pub fn normalize(&self, raw: &str) -> Option<String> {
        let (user, server) = split_jid(raw);
        if user.is_empty() {
            return None;
        }
        Some(match server {
            "lid" => match self.lid_to_pn.get(user) {
                Some(pn) => format!("{pn}@{PN_SERVER}"),
                None => format!("{user}@lid"),
            },
            "" => match self.lid_to_pn.get(user) {
                Some(pn) => format!("{pn}@{PN_SERVER}"),
                None if user.chars().all(|c| c.is_ascii_digit()) => format!("{user}@{PN_SERVER}"),
                None => user.to_string(),
            },
            PN_SERVER | "c.us" => format!("{user}@{PN_SERVER}"),
            other => format!("{user}@{other}"),
        })
    }

    /// Whether `raw` (any form) is our own account.
    pub fn is_own(&self, raw: &str) -> bool {
        let (user, server) = split_jid(raw);
        if user.is_empty() {
            return false;
        }
        if self.own_users.contains(user) {
            return true;
        }
        if matches!(server, "lid" | "")
            && let Some(pn) = self.lid_to_pn.get(user)
        {
            return self.own_users.contains(pn);
        }
        false
    }

    /// Our own phone JID, when known.
    pub fn own_jid(&self) -> Option<String> {
        self.own_pn.as_ref().map(|u| format!("{u}@{PN_SERVER}"))
    }

    fn real_chat_name(&self, jid: &str) -> Option<&str> {
        let stored = self.chat_names.get(jid)?.as_deref();
        (!is_placeholder_name(jid, stored)).then_some(stored?.trim())
    }

    fn contact(&self, jid: &str) -> Option<&str> {
        self.contacts.get(jid).map(String::as_str)
    }

    /// Best person name for a normalised JID (no fallback).
    fn person_name(&self, norm: &str) -> Option<String> {
        let (user, server) = split_jid(norm);
        let mut forms = vec![norm.to_string()];
        if server == PN_SERVER
            && let Some(lid) = self.pn_to_lid.get(user)
        {
            forms.push(format!("{lid}@lid"));
        }
        forms
            .iter()
            .find_map(|f| self.real_chat_name(f))
            .or_else(|| forms.iter().find_map(|f| self.contact(f)))
            .map(str::to_string)
    }

    fn fallback(&self, norm: Option<&str>, raw: &str) -> String {
        match norm {
            Some(n) if n.ends_with(&format!("@{PN_SERVER}")) => format!("+{}", split_jid(n).0),
            _ if raw.trim().is_empty() => "Unknown".to_string(),
            _ => raw.trim().to_string(),
        }
    }

    /// Display name of a message sender.
    pub fn sender_name(&self, raw: &str, is_from_me: bool) -> String {
        if is_from_me || self.is_own(raw) {
            return "Me".to_string();
        }
        let norm = self.normalize(raw);
        norm.as_deref()
            .and_then(|n| self.person_name(n))
            .unwrap_or_else(|| self.fallback(norm.as_deref(), raw))
    }

    /// Display name of a chat (never empty).
    pub fn chat_name(&self, jid: &str) -> String {
        if let Some(n) = self.real_chat_name(jid) {
            return n.to_string();
        }
        if jid == STATUS_JID {
            return "Status".to_string();
        }
        if let Some(n) = self.meta_name(jid) {
            return n.to_string();
        }
        match ChatType::from_jid(jid) {
            ChatType::Channel => return "WhatsApp channel".to_string(),
            ChatType::Broadcast => return "Broadcast list".to_string(),
            _ => {}
        }
        if is_group_jid(jid) || is_status_jid(jid) {
            let stored = self.chat_names.get(jid).cloned().flatten();
            return match stored {
                Some(s) if !s.trim().is_empty() => s.trim().to_string(),
                _ => jid.split('@').next().unwrap_or(jid).to_string(),
            };
        }
        let norm = self.normalize(jid);
        norm.as_deref()
            .and_then(|n| self.person_name(n))
            .unwrap_or_else(|| self.fallback(norm.as_deref(), jid))
    }

    /// Real name from bridge `chat_meta` (None when empty or a placeholder).
    fn meta_name(&self, jid: &str) -> Option<&str> {
        let n = self.meta.get(jid)?.name.as_deref()?.trim();
        (!is_placeholder_name(jid, Some(n))).then_some(n)
    }

    /// Bridge metadata of a chat, if any.
    pub fn meta(&self, jid: &str) -> Option<&ChatMeta> {
        self.meta.get(jid)
    }

    /// What a chat is: bridge metadata first, else derived from the JID.
    pub fn chat_type(&self, jid: &str) -> ChatType {
        if jid == STATUS_JID {
            return ChatType::Status;
        }
        self.meta
            .get(jid)
            .and_then(|m| m.chat_type)
            .unwrap_or_else(|| ChatType::from_jid(jid))
    }

    /// Lightweight reference to a chat.
    pub fn chat_ref(&self, jid: &str) -> ChatRef {
        ChatRef {
            jid: jid.to_string(),
            name: self.chat_name(jid),
            chat_type: self.chat_type(jid),
        }
    }

    /// The community a sub-group belongs to.
    pub fn parent(&self, jid: &str) -> Option<ChatRef> {
        let p = self.meta.get(jid)?.parent_jid.as_deref()?.trim();
        (!p.is_empty() && p != jid).then(|| self.chat_ref(p))
    }

    /// Groups that list `parent` as their community.
    pub fn children(&self, parent: &str) -> Vec<ChatRef> {
        let mut v: Vec<ChatRef> = self
            .meta
            .iter()
            .filter(|(j, m)| m.parent_jid.as_deref() == Some(parent) && j.as_str() != parent)
            .map(|(j, _)| self.chat_ref(j))
            .collect();
        v.sort_by_key(|c| c.name.to_lowercase());
        v
    }

    /// Our own user parts (phone and LID), for @mention detection.
    pub fn own_user_parts(&self) -> Vec<String> {
        let mut v: Vec<String> = self.own_users.iter().cloned().collect();
        if let Some(l) = self.own_pn.as_ref().and_then(|pn| self.pn_to_lid.get(pn)) {
            v.push(l.clone());
        }
        v.sort();
        v.dedup();
        v
    }

    /// Every stored form a sender may appear as (both phone and LID forms).
    pub fn sender_aliases(&self, value: &str) -> Vec<String> {
        let (bare, _) = split_jid(value);
        let bare = bare.trim_start_matches('+');
        let (pn, lid) = if let Some(l) = self.pn_to_lid.get(bare) {
            (Some(bare.to_string()), Some(l.clone()))
        } else if let Some(p) = self.lid_to_pn.get(bare) {
            (Some(p.clone()), Some(bare.to_string()))
        } else {
            (None, None)
        };
        let mut out = Vec::new();
        if let Some(p) = pn {
            out.push(p.clone());
            out.push(format!("{p}@{PN_SERVER}"));
        }
        if let Some(l) = lid {
            out.push(l.clone());
            out.push(format!("{l}@lid"));
        }
        if out.is_empty() && !bare.is_empty() {
            out = vec![
                bare.to_string(),
                format!("{bare}@{PN_SERVER}"),
                format!("{bare}@lid"),
            ];
        }
        out
    }

    /// Known chat JIDs (used by the watcher to detect renames).
    pub fn chat_name_raw(&self, jid: &str) -> Option<&str> {
        self.chat_names.get(jid).and_then(|n| n.as_deref())
    }
}

/// Lazily loaded, cached `NameCache`.
pub struct NameResolver {
    bridge: Arc<RoPool>,
    wa: Arc<RoPool>,
    cache: RwLock<Option<(Instant, Arc<NameCache>)>>,
    loading: parking_lot::Mutex<()>,
}

impl NameResolver {
    /// Resolver over the bridge pool and the (possibly missing) whatsapp.db pool.
    pub fn new(bridge: Arc<RoPool>, wa: Arc<RoPool>) -> Self {
        Self {
            bridge,
            wa,
            cache: RwLock::new(None),
            loading: parking_lot::Mutex::new(()),
        }
    }

    /// Drop the cache; the next `snapshot()` reloads.
    pub fn invalidate(&self) {
        *self.cache.write() = None;
    }

    /// Current snapshot (loads on first use or after TTL / invalidate).
    ///
    /// Single-flight: only one caller reloads; while it does, others keep
    /// using the expired snapshot (if there is one) instead of piling up
    /// identical ~200 ms reloads.
    pub fn snapshot(&self) -> Result<Arc<NameCache>> {
        let stale = match self.cache.read().as_ref() {
            Some((at, c)) if at.elapsed() < TTL => return Ok(Arc::clone(c)),
            Some((_, c)) => Some(Arc::clone(c)),
            None => None,
        };
        let _loading = match (self.loading.try_lock(), stale) {
            (Some(g), _) => g,
            (None, Some(s)) => return Ok(s),
            (None, None) => self.loading.lock(),
        };
        // Someone else may have finished loading while we waited.
        if let Some((at, c)) = self.cache.read().as_ref()
            && at.elapsed() < TTL
        {
            return Ok(Arc::clone(c));
        }
        let fresh = Arc::new(self.load()?);
        *self.cache.write() = Some((Instant::now(), Arc::clone(&fresh)));
        Ok(fresh)
    }

    fn load(&self) -> Result<NameCache> {
        let mut c = NameCache {
            chat_names: load_chat_names(&self.bridge)?,
            ..Default::default()
        };
        match load_chat_meta(&self.bridge) {
            Ok(m) => c.meta = m,
            Err(e) => tracing::warn!(kind = ?e.kind, "chat_meta unreadable; chat types from JIDs"),
        }
        if self.wa.available() {
            if let Err(e) = load_whatsmeow(&self.wa, &mut c) {
                tracing::warn!(kind = ?e.kind, "whatsapp.db unreadable; names degrade");
            }
        } else {
            tracing::debug!("whatsapp.db missing; names degrade to chats.name/phone");
        }
        Ok(c)
    }
}

fn load_chat_names(pool: &RoPool) -> Result<HashMap<String, Option<String>>> {
    let conn = pool.get()?;
    let mut out = HashMap::new();
    let mut last = String::new();
    loop {
        let page: Vec<(String, Option<String>)> = conn.rows(
            "SELECT jid, name FROM chats WHERE jid > ?1 ORDER BY jid LIMIT 500",
            [&last],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let n = page.len();
        if let Some((j, _)) = page.last() {
            last = j.clone();
        }
        out.extend(page);
        if n < PAGE_ROWS {
            return Ok(out);
        }
    }
}

type MetaRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn load_chat_meta(pool: &RoPool) -> Result<HashMap<String, ChatMeta>> {
    let conn = pool.get()?;
    let mut out = HashMap::new();
    if !conn.has_table("chat_meta")? {
        return Ok(out);
    }
    let mut last = String::new();
    loop {
        let page: Vec<MetaRow> = conn.rows(
            "SELECT jid, chat_type, parent_jid, name, description FROM chat_meta \
                 WHERE jid > ?1 ORDER BY jid LIMIT 500",
            [&last],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let n = page.len();
        for (jid, ty, parent, name, desc) in page {
            last = jid.clone();
            let clean =
                |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            out.insert(
                jid,
                ChatMeta {
                    chat_type: ty.as_deref().and_then(chat_type_from_meta),
                    parent_jid: clean(parent),
                    name: clean(name),
                    description: clean(desc),
                },
            );
        }
        if n < PAGE_ROWS {
            return Ok(out);
        }
    }
}

fn load_whatsmeow(pool: &RoPool, c: &mut NameCache) -> Result<()> {
    let conn = pool.get()?;
    if conn.has_table("whatsmeow_lid_map")? {
        let mut last = String::new();
        loop {
            let page: Vec<(String, String)> = conn.rows(
                "SELECT lid, pn FROM whatsmeow_lid_map WHERE lid > ?1 ORDER BY lid LIMIT 500",
                [&last],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let n = page.len();
            for (lid, pn) in page {
                last = lid.clone();
                let (l, p) = (split_jid(&lid).0.to_string(), split_jid(&pn).0.to_string());
                c.pn_to_lid.insert(p.clone(), l.clone());
                c.lid_to_pn.insert(l, p);
            }
            if n < PAGE_ROWS {
                break;
            }
        }
    }
    let mut own: Vec<String> = Vec::new();
    if conn.has_table("whatsmeow_device")? {
        let cols = conn.columns("whatsmeow_device")?;
        let sql = if cols.iter().any(|x| x == "lid") {
            "SELECT jid, lid FROM whatsmeow_device LIMIT 50"
        } else {
            "SELECT jid, NULL FROM whatsmeow_device LIMIT 50"
        };
        for (j, l) in conn.rows(sql, [], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })? {
            own.extend(j);
            own.extend(l);
        }
    }
    if conn.has_table("whatsmeow_contacts")? {
        load_contacts(&conn, c, &mut own)?;
    }
    for j in &own {
        let (user, server) = split_jid(j);
        if user.is_empty() {
            continue;
        }
        c.own_users.insert(user.to_string());
        if server == PN_SERVER && c.own_pn.is_none() {
            c.own_pn = Some(user.to_string());
        }
        if server == "lid"
            && let Some(pn) = c.lid_to_pn.get(user).cloned()
        {
            c.own_users.insert(pn);
        }
    }
    Ok(())
}

type ContactRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn load_contacts(
    conn: &crate::bridge_ro::RoConn,
    c: &mut NameCache,
    own: &mut Vec<String>,
) -> Result<()> {
    let (mut a, mut b) = (String::new(), String::new());
    let mut ours = HashSet::new();
    loop {
        let page: Vec<ContactRow> = conn.rows(
            "SELECT COALESCE(our_jid,''), COALESCE(their_jid,''), full_name, push_name, first_name, business_name \
             FROM whatsmeow_contacts WHERE (COALESCE(our_jid,''), COALESCE(their_jid,'')) > (?1, ?2) \
             ORDER BY COALESCE(our_jid,''), COALESCE(their_jid,'') LIMIT 500",
            [&a, &b],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        let n = page.len();
        for (our, their, full, push, first, biz) in page {
            a = our.clone();
            b = their.clone();
            ours.insert(our);
            let name = [full, push, first, biz]
                .into_iter()
                .flatten()
                .map(|s| s.trim().to_string())
                .find(|s| !s.is_empty());
            let (user, server) = split_jid(&their);
            if let Some(name) = name
                && !user.is_empty()
            {
                let server = if server.is_empty() { PN_SERVER } else { server };
                c.contacts.entry(format!("{user}@{server}")).or_insert(name);
            }
        }
        if n < PAGE_ROWS {
            break;
        }
    }
    if own.is_empty() {
        own.extend(ours.into_iter().filter(|s| !s.is_empty()));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::bridge_ro::DEFAULT_DEADLINE;
    use wa_testkit::{Fixture, ids::*};

    pub(crate) fn resolver(f: &Fixture) -> NameResolver {
        NameResolver::new(
            Arc::new(RoPool::new(&f.paths.messages_db, DEFAULT_DEADLINE)),
            Arc::new(RoPool::new(&f.paths.whatsapp_db, DEFAULT_DEADLINE)),
        )
    }

    #[test]
    fn split_strips_device_and_agent() {
        assert_eq!(
            split_jid("919811111111:12@s.whatsapp.net"),
            ("919811111111", "s.whatsapp.net")
        );
        assert_eq!(split_jid("2222.0:3@lid"), ("2222", "lid"));
        assert_eq!(split_jid("919811111111"), ("919811111111", ""));
        assert_eq!(split_jid("120363-99@g.us"), ("120363-99", "g.us"));
    }

    #[test]
    fn placeholder_rules() {
        assert!(is_placeholder_name("1@g.us", None));
        assert!(is_placeholder_name("1@g.us", Some("  ")));
        assert!(is_placeholder_name(
            "919822222222@s.whatsapp.net",
            Some("919822222222")
        ));
        assert!(is_placeholder_name(
            "x@s.whatsapp.net",
            Some("+91 98222-22222")
        ));
        assert!(is_placeholder_name(
            GROUP_PLACEHOLDER,
            Some("Group 120363000000000002")
        ));
        assert!(!is_placeholder_name(GROUP_CLASS, Some("ML CSE - 3C")));
        assert!(!is_placeholder_name(DM_ALICE, Some("Alice 2")));
    }

    #[test]
    fn sender_forms_all_resolve() {
        let f = Fixture::standard();
        let c = resolver(&f).snapshot().unwrap();
        // Alice: real chats.name
        for form in [
            "919811111111",
            "919811111111@s.whatsapp.net",
            "919811111111:7@s.whatsapp.net",
        ] {
            assert_eq!(c.sender_name(form, false), "Alice", "{form}");
            assert_eq!(c.normalize(form).as_deref(), Some(DM_ALICE));
        }
        // Bob: chats.name is digits → contact full_name
        assert_eq!(c.sender_name("919822222222", false), "Bob Builder");
        assert_eq!(
            c.sender_name("919822222222@s.whatsapp.net", false),
            "Bob Builder"
        );
        // Carol via LID in every form
        for form in [
            "222222222222222",
            "222222222222222@lid",
            "222222222222222:4@lid",
        ] {
            assert_eq!(c.sender_name(form, false), "Carol", "{form}");
            assert_eq!(c.normalize(form).as_deref(), Some(CAROL_PN));
        }
        // business_name fallback via LID
        assert_eq!(c.sender_name("333333333333333", false), "Sharma Traders");
        // Unknown phone → +phone
        assert_eq!(c.sender_name("919844444444", false), "+919844444444");
        // Unmapped LID → raw
        assert_eq!(
            c.sender_name("999999999999999@lid", false),
            "999999999999999@lid"
        );
        assert_eq!(
            c.normalize("999999999999999@lid").as_deref(),
            Some("999999999999999@lid")
        );
        assert_eq!(c.sender_name("", false), "Unknown");
        assert_eq!(c.normalize(""), None);
    }

    #[test]
    fn own_number_is_me() {
        let f = Fixture::standard();
        let c = resolver(&f).snapshot().unwrap();
        assert_eq!(c.sender_name("919900000000", false), "Me");
        assert_eq!(c.sender_name("919900000000:5@s.whatsapp.net", false), "Me");
        assert_eq!(c.sender_name("111111111111111@lid", false), "Me");
        assert_eq!(c.sender_name("919811111111", true), "Me");
        assert_eq!(c.own_jid().as_deref(), Some(wa_testkit::ME));
    }

    #[test]
    fn chat_names() {
        let f = Fixture::standard();
        let c = resolver(&f).snapshot().unwrap();
        assert_eq!(c.chat_name(DM_ALICE), "Alice");
        assert_eq!(c.chat_name(DM_BOB), "Bob Builder");
        assert_eq!(c.chat_name(DM_LID), "Carol");
        assert_eq!(c.chat_name(DM_UNKNOWN), "+919844444444");
        assert_eq!(c.chat_name(GROUP_CLASS), "ML CSE - 3C");
        assert_eq!(c.chat_name(GROUP_PLACEHOLDER), "Group 120363000000000002");
        assert_eq!(c.chat_name(STATUS), "Status");
        assert_eq!(c.chat_name("777@lid"), "777@lid");
    }

    #[test]
    fn aliases_both_directions() {
        let f = Fixture::standard();
        let c = resolver(&f).snapshot().unwrap();
        let want = vec![
            "919833333333".to_string(),
            CAROL_PN.to_string(),
            "222222222222222".to_string(),
            "222222222222222@lid".to_string(),
        ];
        assert_eq!(c.sender_aliases("919833333333"), want);
        assert_eq!(c.sender_aliases(CAROL_PN), want);
        assert_eq!(c.sender_aliases("222222222222222@lid"), want);
        assert_eq!(c.sender_aliases("+919833333333"), want);
        assert_eq!(
            c.sender_aliases("12345"),
            vec!["12345", "12345@s.whatsapp.net", "12345@lid"]
        );
    }

    #[test]
    fn missing_whatsapp_db_degrades() {
        let f = Fixture::standard();
        std::fs::remove_file(&f.paths.whatsapp_db).unwrap();
        let c = resolver(&f).snapshot().unwrap();
        assert_eq!(c.chat_name(DM_ALICE), "Alice");
        assert_eq!(c.chat_name(DM_BOB), "+919822222222");
        assert_eq!(
            c.sender_name("222222222222222@lid", false),
            "222222222222222@lid"
        );
        assert_eq!(c.sender_name("919900000000", false), "+919900000000");
        assert!(!f.paths.whatsapp_db.exists());
    }

    #[test]
    fn invalidate_reloads() {
        let f = Fixture::standard();
        let r = resolver(&f);
        assert_eq!(r.snapshot().unwrap().chat_name(DM_UNKNOWN), "+919844444444");
        f.add_contact(DM_UNKNOWN, Some("Dave"), None, None, None);
        assert_eq!(r.snapshot().unwrap().chat_name(DM_UNKNOWN), "+919844444444");
        r.invalidate();
        assert_eq!(r.snapshot().unwrap().chat_name(DM_UNKNOWN), "Dave");
    }

    #[test]
    fn contact_priority() {
        let f = Fixture::new();
        f.add_contact(
            "1@s.whatsapp.net",
            None,
            Some("push"),
            Some("first"),
            Some("biz"),
        );
        f.add_contact("2@s.whatsapp.net", None, None, Some("first"), Some("biz"));
        f.add_contact("3@s.whatsapp.net", Some(" "), None, None, Some("biz"));
        let c = resolver(&f).snapshot().unwrap();
        assert_eq!(c.sender_name("1", false), "push");
        assert_eq!(c.sender_name("2", false), "first");
        assert_eq!(c.sender_name("3", false), "biz");
    }
}
