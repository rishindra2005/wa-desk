//! Test fixtures for wa-desk.
//!
//! `Fixture::new()` creates a temp tree laid out exactly like the live install
//! (see `wa_core::config::Paths::under_root`) with:
//! - `messages.db` using the bridge's exact schema (whatsapp-bridge/main.go:194
//!   plus `ensureMessageStoreSchema` columns) and the external `phone_media_links` table;
//! - `whatsapp.db` with the whatsmeow tables the readers use;
//! - `.bridge-token` (with a trailing newline, like the real one).
//!
//! `Fixture::standard()` additionally seeds the edge-case scenario described
//! in `seed` (constants in `ids`), which every crate's tests can rely on.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use wa_core::config::{DEFAULT_BRIDGE_URL, Paths};

pub const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// Our own account (whatsmeow_device / our_jid).
pub const ME: &str = "919900000000@s.whatsapp.net";

pub const BRIDGE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS chats (
    jid TEXT PRIMARY KEY,
    name TEXT,
    last_message_time TIMESTAMP,
    ephemeral_expiration INTEGER NOT NULL DEFAULT 0,
    ephemeral_setting_timestamp INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS messages (
    id TEXT,
    chat_jid TEXT,
    sender TEXT,
    content TEXT,
    timestamp TIMESTAMP,
    is_from_me BOOLEAN,
    media_type TEXT,
    filename TEXT,
    url TEXT,
    media_key BLOB,
    file_sha256 BLOB,
    file_enc_sha256 BLOB,
    file_length INTEGER,
    deleted_at TIMESTAMP,
    PRIMARY KEY (id, chat_jid),
    FOREIGN KEY (chat_jid) REFERENCES chats(jid)
);
CREATE TABLE IF NOT EXISTS calls (
    call_id TEXT, chat_jid TEXT, from_jid TEXT, timestamp TIMESTAMP, is_from_me BOOLEAN,
    call_type TEXT, is_group BOOLEAN, result TEXT, duration_sec INTEGER, ended_at TIMESTAMP, reason TEXT,
    PRIMARY KEY (call_id, chat_jid)
);
CREATE INDEX IF NOT EXISTS idx_calls_chat ON calls(chat_jid);
CREATE INDEX IF NOT EXISTS idx_calls_timestamp ON calls(timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_chat_jid ON messages(chat_jid);
ALTER TABLE chats ADD COLUMN last_read_time TIMESTAMP;
ALTER TABLE messages ADD COLUMN quoted_message_id TEXT;
CREATE TABLE IF NOT EXISTS phone_media_links (
    message_id TEXT NOT NULL, chat_jid TEXT NOT NULL, media_type TEXT,
    phone_path TEXT NOT NULL, store_path TEXT, matched_by TEXT NOT NULL,
    linked_at TEXT NOT NULL, PRIMARY KEY (message_id, chat_jid));
"#;

pub const WHATSMEOW_SCHEMA: &str = r#"
CREATE TABLE whatsmeow_device (jid TEXT PRIMARY KEY, lid TEXT);
CREATE TABLE whatsmeow_contacts (
    our_jid TEXT, their_jid TEXT, first_name TEXT, full_name TEXT, push_name TEXT, business_name TEXT,
    PRIMARY KEY (our_jid, their_jid));
CREATE TABLE whatsmeow_lid_map (lid TEXT PRIMARY KEY, pn TEXT UNIQUE NOT NULL);
"#;

/// One message row. Use the builder-ish constructors then `Fixture::add_message`.
#[derive(Debug, Clone, Default)]
pub struct Msg {
    pub id: String,
    pub chat_jid: String,
    pub sender: String,
    pub content: String,
    pub ts: String,
    pub from_me: bool,
    pub media_type: String,
    pub filename: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<Vec<u8>>,
    pub length: Option<i64>,
    pub deleted_at: Option<String>,
    pub quoted: Option<String>,
}

impl Msg {
    pub fn text(id: &str, chat: &str, sender: &str, ts: &str, content: &str) -> Self {
        Self {
            id: id.into(),
            chat_jid: chat.into(),
            sender: sender.into(),
            ts: ts.into(),
            content: content.into(),
            ..Default::default()
        }
    }
    pub fn mine(mut self) -> Self {
        self.from_me = true;
        self
    }
    pub fn media(mut self, kind: &str, filename: Option<&str>, data: &[u8]) -> Self {
        use sha2_lite::sha256;
        self.media_type = kind.into();
        self.filename = filename.map(Into::into);
        self.url = Some(format!("https://mmg.whatsapp.net/v/t62/{}?ccb=1", self.id));
        self.sha256 = Some(sha256(data).to_vec());
        self.length = Some(data.len() as i64);
        self
    }
    pub fn reaction(
        id: &str,
        chat: &str,
        sender: &str,
        ts: &str,
        target: &str,
        emoji: &str,
    ) -> Self {
        Self {
            media_type: "reaction".into(),
            filename: Some(target.into()),
            ..Self::text(id, chat, sender, ts, emoji)
        }
    }
    pub fn deleted(mut self, at: &str) -> Self {
        self.deleted_at = Some(at.into());
        self
    }
    pub fn quoting(mut self, id: &str) -> Self {
        self.quoted = Some(id.into());
        self
    }
}

pub struct Fixture {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub paths: Paths,
}

impl Fixture {
    /// Empty but schema-complete install.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let paths = Paths::under_root(&root, DEFAULT_BRIDGE_URL.to_string());
        std::fs::create_dir_all(&paths.bridge_store).unwrap();
        std::fs::create_dir_all(&paths.phone_media).unwrap();
        std::fs::create_dir_all(&paths.outbox).unwrap();
        paths.ensure_own_dirs().unwrap();
        std::fs::write(&paths.token_file, format!("{TOKEN}\n")).unwrap();
        Connection::open(&paths.messages_db)
            .unwrap()
            .execute_batch(BRIDGE_SCHEMA)
            .unwrap();
        let wa = Connection::open(&paths.whatsapp_db).unwrap();
        wa.execute_batch(WHATSMEOW_SCHEMA).unwrap();
        wa.execute(
            "INSERT INTO whatsmeow_device (jid, lid) VALUES (?1, ?2)",
            params![
                format!(
                    "{}:5@s.whatsapp.net",
                    ME.trim_end_matches("@s.whatsapp.net")
                ),
                "111111111111111:5@lid"
            ],
        )
        .unwrap();
        Self {
            _dir: dir,
            root,
            paths,
        }
    }

    /// Install seeded with the standard edge-case scenario (see `seed`).
    pub fn standard() -> Self {
        let f = Self::new();
        seed::standard(&f);
        f
    }

    /// Writable connection to the fixture's messages.db (tests act as "the bridge").
    pub fn bridge_conn(&self) -> Connection {
        Connection::open(&self.paths.messages_db).unwrap()
    }

    pub fn wa_conn(&self) -> Connection {
        Connection::open(&self.paths.whatsapp_db).unwrap()
    }

    pub fn add_chat(&self, jid: &str, name: Option<&str>, last_ts: Option<&str>) {
        self.bridge_conn()
            .execute(
                "INSERT OR REPLACE INTO chats (jid, name, last_message_time) VALUES (?1, ?2, ?3)",
                params![jid, name, last_ts],
            )
            .unwrap();
    }

    pub fn set_last_read(&self, jid: &str, ts: Option<&str>) {
        self.bridge_conn()
            .execute(
                "UPDATE chats SET last_read_time = ?2 WHERE jid = ?1",
                params![jid, ts],
            )
            .unwrap();
    }

    /// Insert like the bridge does (upsert on (id, chat_jid) keeps the rowid).
    pub fn add_message(&self, m: &Msg) -> i64 {
        let c = self.bridge_conn();
        c.execute(
            "INSERT INTO messages (id, chat_jid, sender, content, timestamp, is_from_me, media_type, filename, url, media_key, file_sha256, file_enc_sha256, file_length, deleted_at, quoted_message_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
             ON CONFLICT(id, chat_jid) DO UPDATE SET content=excluded.content, sender=excluded.sender, deleted_at=excluded.deleted_at, url=excluded.url",
            params![
                m.id, m.chat_jid, m.sender, m.content, m.ts, m.from_me, m.media_type, m.filename, m.url,
                m.sha256.as_ref().map(|_| vec![7u8; 32]), m.sha256, m.sha256.as_ref().map(|_| vec![9u8; 32]), m.length, m.deleted_at, m.quoted
            ],
        )
        .unwrap();
        c.query_row(
            "SELECT rowid FROM messages WHERE id=?1 AND chat_jid=?2",
            params![m.id, m.chat_jid],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// Insert or replace a `calls` row (the bridge upserts by `(call_id, chat_jid)`).
    /// `result` is e.g. `in_progress`/`answered`/`ended`/`missed`/`rejected`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_call(
        &self,
        call_id: &str,
        chat_jid: &str,
        from_jid: &str,
        ts: &str,
        from_me: bool,
        call_type: &str,
        result: &str,
        duration: Option<i64>,
    ) -> i64 {
        let c = self.bridge_conn();
        c.execute(
            "INSERT INTO calls (call_id, chat_jid, from_jid, timestamp, is_from_me, call_type, is_group, result, duration_sec)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(call_id, chat_jid) DO UPDATE SET result=excluded.result, duration_sec=excluded.duration_sec,
               ended_at=CASE WHEN excluded.result IN ('ended','missed','rejected') THEN excluded.timestamp ELSE ended_at END",
            params![call_id, chat_jid, from_jid, ts, from_me, call_type, chat_jid.ends_with("@g.us"), result, duration],
        )
        .unwrap();
        c.query_row(
            "SELECT rowid FROM calls WHERE call_id=?1 AND chat_jid=?2",
            params![call_id, chat_jid],
            |r| r.get(0),
        )
        .unwrap()
    }

    pub fn add_contact(
        &self,
        their_jid: &str,
        full: Option<&str>,
        push: Option<&str>,
        first: Option<&str>,
        business: Option<&str>,
    ) {
        self.wa_conn()
            .execute(
                "INSERT OR REPLACE INTO whatsmeow_contacts (our_jid, their_jid, first_name, full_name, push_name, business_name) VALUES (?1,?2,?3,?4,?5,?6)",
                params![ME, their_jid, first, full, push, business],
            )
            .unwrap();
    }

    pub fn add_lid(&self, lid: &str, pn: &str) {
        self.wa_conn()
            .execute(
                "INSERT OR REPLACE INTO whatsmeow_lid_map (lid, pn) VALUES (?1, ?2)",
                params![lid, pn],
            )
            .unwrap();
    }

    /// Put a file where the bridge would have downloaded it.
    pub fn put_store_media(
        &self,
        chat: &str,
        kind: &str,
        id: &str,
        raw_ts: &str,
        data: &[u8],
    ) -> PathBuf {
        let p = wa_core::paths::bridge_media_path(&self.paths.bridge_store, chat, kind, id, raw_ts)
            .unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, data).unwrap();
        p
    }

    /// Put a file in the phone-media copy (e.g. "WhatsApp Documents/x.pdf").
    pub fn put_phone_file(&self, rel: &str, data: &[u8]) -> PathBuf {
        let p = self.paths.phone_media.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, data).unwrap();
        p
    }

    pub fn path(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.root.join(rel)
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

/// Seeded scenario ids.
pub mod ids {
    pub const DM_ALICE: &str = "919811111111@s.whatsapp.net"; // named chat
    pub const DM_BOB: &str = "919822222222@s.whatsapp.net"; // chats.name is digits → contact full_name
    pub const DM_LID: &str = "222222222222222@lid"; // LID chat, mapped to CAROL_PN
    pub const CAROL_PN: &str = "919833333333@s.whatsapp.net";
    pub const DM_UNKNOWN: &str = "919844444444@s.whatsapp.net"; // no name anywhere → phone fallback
    pub const GROUP_CLASS: &str = "120363000000000001@g.us"; // named group
    pub const GROUP_PLACEHOLDER: &str = "120363000000000002@g.us"; // name "Group 120363000000000002"
    pub const STATUS: &str = "status@broadcast";

    pub const M_ALICE_1: &str = "A1";
    pub const M_ALICE_2_ME: &str = "A2";
    pub const M_ALICE_PDF: &str = "A3PDF";
    pub const M_ALICE_REACT: &str = "A4R";
    pub const M_ALICE_REVOKED: &str = "A5DEL";
    pub const M_ALICE_REPLY: &str = "A6Q";
    pub const M_BOB_1: &str = "B1";
    pub const M_LID_1: &str = "L1";
    pub const M_GROUP_IMG: &str = "G1IMG";
    pub const M_GROUP_TXT_LID: &str = "G2";
    pub const M_GROUP_DEVANAGARI: &str = "G3";
    pub const M_STATUS: &str = "S1";

    pub const PDF_BYTES: &[u8] = b"%PDF-1.4 fixture pdf\n%%EOF\n";
    pub const IMG_BYTES: &[u8] = b"\xFF\xD8\xFF\xE0fixture-jpeg";
}

pub mod seed {
    use super::ids::*;
    use super::*;

    /// Scenario (timestamps deliberately mix offsets/forms):
    /// - Alice (named): text, my reply, a PDF (file in store), a reaction (from me), a revoked msg, a quoted reply.
    /// - Bob: chats.name is just digits → name must come from whatsmeow full_name "Bob Builder".
    /// - LID chat mapped to Carol's phone with push_name "Carol".
    /// - Unknown DM: no names anywhere → falls back to phone.
    /// - Group "ML CSE - 3C": image (only on phone copy, not in store), LID sender text, Devanagari text.
    /// - Placeholder-named group with no messages.
    /// - status@broadcast message.
    ///
    /// Unread: Alice read up to before her last message → unread; Bob read after → not unread.
    pub fn standard(f: &Fixture) {
        f.add_contact(
            DM_BOB,
            Some("Bob Builder"),
            Some("bobby"),
            Some("Bob"),
            None,
        );
        f.add_contact(CAROL_PN, None, Some("Carol"), None, None);
        f.add_contact(
            "919855555555@s.whatsapp.net",
            None,
            None,
            None,
            Some("Sharma Traders"),
        );
        f.add_lid(
            "222222222222222",
            CAROL_PN.trim_end_matches("@s.whatsapp.net"),
        );
        f.add_lid("333333333333333", "919855555555");

        f.add_chat(DM_ALICE, Some("Alice"), Some("2026-11-15 10:06:00+05:30"));
        f.add_chat(
            DM_BOB,
            Some("919822222222"),
            Some("2026-11-14 09:00:00+05:30"),
        );
        f.add_chat(DM_LID, None, Some("2026-11-13 08:00:00Z"));
        f.add_chat(DM_UNKNOWN, None, Some("2026-11-12 08:00:00"));
        f.add_chat(
            GROUP_CLASS,
            Some("ML CSE - 3C"),
            Some("2026-11-15T12:00:00+05:30"),
        );
        f.add_chat(GROUP_PLACEHOLDER, Some("Group 120363000000000002"), None);
        f.add_chat(STATUS, None, Some("2026-11-15 11:00:00+05:30"));

        f.add_message(&Msg::text(
            M_ALICE_1,
            DM_ALICE,
            "919811111111",
            "2026-11-15 10:00:00+05:30",
            "Hi! Did you get the ML notes?",
        ));
        f.add_message(
            &Msg::text(
                M_ALICE_2_ME,
                DM_ALICE,
                "919900000000",
                "2026-11-15 10:01:00+05:30",
                "Yes, module 4 too",
            )
            .mine(),
        );
        f.add_message(
            &Msg::text(
                M_ALICE_PDF,
                DM_ALICE,
                "919811111111",
                "2026-11-15 10:02:00+05:30",
                "here",
            )
            .media("document", Some("ML-MODULE 1 Notes.pdf"), PDF_BYTES),
        );
        f.put_store_media(
            DM_ALICE,
            "document",
            M_ALICE_PDF,
            "2026-11-15 10:02:00+05:30",
            PDF_BYTES,
        );
        f.add_message(
            &Msg::reaction(
                M_ALICE_REACT,
                DM_ALICE,
                "919900000000",
                "2026-11-15 10:03:00+05:30",
                M_ALICE_PDF,
                "👍",
            )
            .mine(),
        );
        f.add_message(
            &Msg::text(
                M_ALICE_REVOKED,
                DM_ALICE,
                "919811111111",
                "2026-11-15 10:04:00+05:30",
                "oops",
            )
            .deleted("2026-11-15 10:04:30+05:30"),
        );
        f.add_message(
            &Msg::text(
                M_ALICE_REPLY,
                DM_ALICE,
                "919811111111",
                "2026-11-15 10:06:00+05:30",
                "Thanks for module four",
            )
            .quoting(M_ALICE_2_ME),
        );
        f.set_last_read(DM_ALICE, Some("2026-11-15 10:05:00+05:30"));

        f.add_message(&Msg::text(
            M_BOB_1,
            DM_BOB,
            "919822222222@s.whatsapp.net",
            "2026-11-14 09:00:00+05:30",
            "Can we fix the build?",
        ));
        f.set_last_read(DM_BOB, Some("2026-11-14 09:30:00+05:30"));

        f.add_message(&Msg::text(
            M_LID_1,
            DM_LID,
            "222222222222222@lid",
            "2026-11-13 08:00:00Z",
            "hello from a lid chat",
        ));
        f.add_message(&Msg::text(
            "U1",
            DM_UNKNOWN,
            "919844444444",
            "2026-11-12 08:00:00",
            "who is this",
        ));

        f.add_message(
            &Msg::text(
                M_GROUP_IMG,
                GROUP_CLASS,
                "919811111111",
                "2026-11-15 11:58:00+05:30",
                "Module 4 diagram",
            )
            .media("image", None, IMG_BYTES),
        );
        f.put_phone_file("WhatsApp Images/IMG-20261115-WA0001.jpg", IMG_BYTES);
        f.add_message(&Msg::text(
            M_GROUP_TXT_LID,
            GROUP_CLASS,
            "333333333333333",
            "2026-11-15T11:59:00+05:30",
            "Group assignment link: https://example.org/course",
        ));
        f.add_message(&Msg::text(
            M_GROUP_DEVANAGARI,
            GROUP_CLASS,
            "919822222222",
            "2026-11-15 12:00:00+05:30",
            "नमस्ते सभी 🙏 exam कल है",
        ));

        f.add_message(&Msg::text(
            M_STATUS,
            STATUS,
            "919811111111",
            "2026-11-15 11:00:00+05:30",
            "my status update",
        ));
    }
}

/// Tiny dependency-free SHA-256 so fixtures match `messages.file_sha256` without pulling sha2 here.
mod sha2_lite {
    pub fn sha256(data: &[u8]) -> [u8; 32] {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];
        let mut msg = data.to_vec();
        let bit_len = (data.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[4 * i],
                    chunk[4 * i + 1],
                    chunk[4 * i + 2],
                    chunk[4 * i + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let mut v = h;
            for i in 0..64 {
                let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
                let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
                let t1 = v[7]
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
                let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
                let t2 = s0.wrapping_add(maj);
                v = [
                    t1.wrapping_add(t2),
                    v[0],
                    v[1],
                    v[2],
                    v[3].wrapping_add(t1),
                    v[4],
                    v[5],
                    v[6],
                ];
            }
            for i in 0..8 {
                h[i] = h[i].wrapping_add(v[i]);
            }
        }
        let mut out = [0u8; 32];
        for (i, x) in h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
        }
        out
    }

    #[cfg(test)]
    #[test]
    fn known_vector() {
        let d = sha256(b"abc");
        assert_eq!(d[..4], [0xba, 0x78, 0x16, 0xbf]);
        assert_eq!(d[28..], [0xf2, 0x00, 0x15, 0xad]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_fixture_is_consistent() {
        let f = Fixture::standard();
        let c = f.bridge_conn();
        let n: i64 = c
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 13);
        let (len, sha): (i64, Vec<u8>) = c
            .query_row(
                "SELECT file_length, file_sha256 FROM messages WHERE id=?1",
                [ids::M_ALICE_PDF],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(len as usize, ids::PDF_BYTES.len());
        assert_eq!(sha.len(), 32);
        assert!(
            f.paths
                .bridge_store
                .join(ids::DM_ALICE)
                .join("document_20261115_100200_A3PDF")
                .exists()
        );
        assert!(
            std::fs::read_to_string(&f.paths.token_file)
                .unwrap()
                .ends_with('\n')
        );
    }

    #[test]
    fn upsert_keeps_rowid() {
        let f = Fixture::standard();
        let m = Msg::text(
            ids::M_BOB_1,
            ids::DM_BOB,
            "919822222222",
            "2026-11-14 09:00:00+05:30",
            "edited",
        );
        let before: i64 = f
            .bridge_conn()
            .query_row("SELECT rowid FROM messages WHERE id='B1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(f.add_message(&m), before);
    }
}
