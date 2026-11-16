//! Read-only smoke run against the real bridge store.
//!
//! Reads messages.db / whatsapp.db from `Paths::from_env()` (read-only) and
//! puts app.db in a temporary directory, so nothing is written anywhere else.
//! Chat names are masked in the output.
//!
//! `cargo run -p wa-db --example smoke --release`

use std::time::Instant;

use wa_core::config::Paths;
use wa_core::types::{ChatQuery, MessagesQuery, SearchQuery};
use wa_db::Db;

fn mask(s: &str) -> String {
    let first: String = s.chars().take(1).collect();
    format!("{first}*** ({} chars)", s.chars().count())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let mut paths = Paths::from_env();
    paths.app_data = tmp.path().to_path_buf();
    paths.app_db = tmp.path().join("app.db");
    let db = Db::open(&paths)?;

    let t = Instant::now();
    let chats = db.list_chats(&ChatQuery::default())?;
    println!(
        "list_chats (unindexed): {} items in {:?}",
        chats.items.len(),
        t.elapsed()
    );
    for c in chats.items.iter().take(3) {
        println!(
            "  chat group={} unread={} name={} preview_len={}",
            c.is_group,
            c.unread,
            mask(&c.name),
            c.last_preview.as_deref().map_or(0, |p| p.chars().count())
        );
    }
    let first = chats.items.first().map(|c| c.jid.clone());
    if let Some(jid) = &first {
        let t = Instant::now();
        let p = db.get_messages(&MessagesQuery {
            chat_jid: jid.clone(),
            before: None,
            after: None,
            limit: Some(80),
        })?;
        println!(
            "get_messages (unindexed): {} in {:?}",
            p.items.len(),
            t.elapsed()
        );
    }

    let t = Instant::now();
    let n = db.indexer().build_full(&|_, _| {})?;
    println!("index build: {n} rows in {:?}", t.elapsed());

    let t = Instant::now();
    let o = db.overview()?;
    println!(
        "overview in {:?}: chats={} messages={} media_total={} media_local={} indexed={}",
        t.elapsed(),
        o.chats,
        o.messages,
        o.media_total,
        o.media_local,
        o.indexed
    );
    for text in ["meeting", "pdf", "ok"] {
        let r = db.search(&SearchQuery {
            text: text.into(),
            ..Default::default()
        })?;
        println!("search {text:?}: total={} took_ms={}", r.total, r.took_ms);
    }
    if let Some(jid) = &first {
        let t = Instant::now();
        let p = db.get_messages(&MessagesQuery {
            chat_jid: jid.clone(),
            before: None,
            after: None,
            limit: Some(80),
        })?;
        println!(
            "get_messages (indexed): {} in {:?}",
            p.items.len(),
            t.elapsed()
        );
    }
    let t = Instant::now();
    let chats = db.list_chats(&ChatQuery::default())?;
    println!(
        "list_chats (indexed): {} in {:?}",
        chats.items.len(),
        t.elapsed()
    );
    db.poll()?;
    let t = Instant::now();
    db.poll()?;
    println!("idle poll: {:?}", t.elapsed());
    Ok(())
}
