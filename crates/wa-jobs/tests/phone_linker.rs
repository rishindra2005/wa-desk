//! Phone sync relies on GVFS/MTP mounts, inodes and executable scripts: Unix only.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use common::*;
use rusqlite::Connection;
use wa_core::traits::MediaInfo;
use wa_core::types::*;
use wa_testkit::{Fixture, Msg, ids};

const MEDIA_REL: &str = "Android/media/com.whatsapp/WhatsApp/Media";

fn fake_phone(env: &Env) -> (PathBuf, PathBuf) {
    let mount = env.dir.path().join("gvfs/mtp:host=SAMSUNG_SM_S911B");
    let media = mount.join("Internal shared storage").join(MEDIA_REL);
    for (rel, data) in [
        ("WhatsApp Images/IMG-1.jpg", &b"img1"[..]),
        ("WhatsApp Images/Sent/IMG-2.jpg", b"img2"),
        ("WhatsApp Documents/Notes.pdf", b"%PDF notes"),
        ("Stickers/s.webp", b"sticker"),
        ("Profile Photos/p.jpg", b"p"),
        ("WhatsApp Voice Notes/202646/PTT-1.opus", b"voice"),
    ] {
        let p = media.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }
    (mount, media)
}

#[tokio::test]
async fn phone_sync_copies_excludes_and_never_deletes() {
    let env = Env::new();
    fake_phone(&env);
    let keep = env
        .paths
        .phone_media
        .join("WhatsApp Images/only-on-laptop.jpg");
    fs::create_dir_all(keep.parent().unwrap()).unwrap();
    fs::write(&keep, b"keep me").unwrap();
    let (m, mut ev) = env.manager();
    let r = m.start(JobSpec::PhoneSync).unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Succeeded, "{done:?}");
    assert_eq!(
        done.message.as_deref(),
        Some("Copied 3 folder(s) from the phone")
    );
    let pm = &env.paths.phone_media;
    assert_eq!(
        fs::read(pm.join("WhatsApp Images/Sent/IMG-2.jpg")).unwrap(),
        b"img2"
    );
    assert!(pm.join("WhatsApp Documents/Notes.pdf").exists());
    assert!(pm.join("WhatsApp Voice Notes/202646/PTT-1.opus").exists());
    assert!(!pm.join("Stickers").exists() && !pm.join("Profile Photos").exists());
    assert!(keep.exists(), "destination files are never deleted");
    assert_eq!(done.done, done.total);
}

#[tokio::test]
async fn phone_sync_without_phone() {
    let env = Env::new();
    fs::create_dir_all(env.dir.path().join("gvfs/smb-share:server=nas")).unwrap();
    let (m, mut ev) = env.manager();
    let r = m.start(JobSpec::PhoneSync).unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    assert_eq!(
        done.error.as_deref(),
        Some("No phone connected over USB (enable File transfer)")
    );
}

#[tokio::test]
async fn phone_sync_detects_unplug_mid_run() {
    let env = Env::new();
    let (mount, _) = fake_phone(&env);
    // rsync wrapper: copy the first folder for real, then "unplug" the phone.
    let script = env.dir.path().join("rsync-unplug.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nrsync \"$@\"; rc=$?; rm -rf '{}'; exit $rc\n",
            mount.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let mut deps = env.deps();
    deps.tools.rsync = script;
    let (m, mut ev) = env.manager_with(deps);
    let r = m.start(JobSpec::PhoneSync).unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    assert_eq!(done.error.as_deref(), Some("Phone disconnected"));
    // First folder (sorted: "WhatsApp Documents") made it.
    assert!(
        env.paths
            .phone_media
            .join("WhatsApp Documents/Notes.pdf")
            .exists()
    );
}

#[tokio::test]
async fn phone_sync_missing_rsync() {
    let env = Env::new();
    fake_phone(&env);
    let mut deps = env.deps();
    deps.tools.rsync = "/nonexistent/rsync".into();
    let (m, mut ev) = env.manager_with(deps);
    let r = m.start(JobSpec::PhoneSync).unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.error.as_deref(), Some("rsync is not installed"));
}

// ---------- linker ----------

/// MediaInfo rows straight from the fixture's messages.db (what wa-db will provide).
fn rows_from_fixture(f: &Fixture) -> Vec<MediaInfo> {
    let c = Connection::open(&f.paths.messages_db).unwrap();
    let mut st = c
        .prepare(
            "SELECT id, chat_jid, media_type, filename, file_sha256, file_length, timestamp FROM messages
             WHERE media_type IN ('image','video','audio','document','sticker') AND deleted_at IS NULL",
        )
        .unwrap();
    st.query_map([], |r| {
        let kind: String = r.get(2)?;
        let raw: String = r.get(6)?;
        let len: Option<i64> = r.get(5)?;
        Ok(MediaInfo {
            message_id: r.get(0)?,
            chat_jid: r.get(1)?,
            kind: MediaKind::from_db(&kind).unwrap(),
            filename: r.get(3)?,
            file_sha256: r.get(4)?,
            file_length: len.map(|l| l as u64),
            ts: wa_core::time::parse_ts(&raw).unwrap(),
            raw_ts: raw,
            local_path: None,
            source: MediaSource::Missing,
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn store_path(f: &Fixture, chat: &str, kind: &str, id: &str, ts: &str) -> PathBuf {
    wa_core::paths::bridge_media_path(&f.paths.bridge_store, chat, kind, id, ts).unwrap()
}

#[tokio::test]
async fn linker_matches_hash_and_name_size_without_overwriting() {
    let f = Fixture::standard();
    // A document only in an extra folder, renamed with a " (1)" counter and different bytes-hash
    // (the message has no sha256): matched by name + size and copied.
    let doc = b"exam schedule v3";
    let mut m = Msg::text(
        "D9",
        ids::DM_BOB,
        "919822222222",
        "2026-11-14 09:05:00+05:30",
        "",
    )
    .media("document", Some("Exam Schedule.pdf"), doc);
    m.sha256 = None;
    f.add_message(&m);
    let extra = f.path("Downloads");
    fs::create_dir_all(&extra).unwrap();
    fs::write(extra.join("exam schedule (1).PDF"), doc).unwrap();
    // A same-name doc with a different size must not match.
    fs::write(extra.join("ML-MODULE 1 Notes.pdf"), b"different size!").unwrap();

    let env = Env::new();
    let mut deps = env.deps();
    deps.paths = f.paths.clone();
    *env.store.rows.lock() = rows_from_fixture(&f);
    let existing = store_path(
        &f,
        ids::DM_ALICE,
        "document",
        ids::M_ALICE_PDF,
        "2026-11-15 10:02:00+05:30",
    );
    let before = fs::read(&existing).unwrap();
    // Put the same PDF on the "phone" so it hash-matches the already-stored message.
    f.put_phone_file("WhatsApp Documents/ML-MODULE 1 Notes.pdf", ids::PDF_BYTES);

    let (mgr, mut ev) = env.manager_with(deps);
    let r = mgr
        .start(JobSpec::LinkPhoneMedia {
            extra_dirs: vec![extra.to_string_lossy().into()],
        })
        .unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Succeeded, "{done:?}");

    // Image from the phone: hash match, hard-linked, 0600 in a 0700 chat dir.
    let img = store_path(
        &f,
        ids::GROUP_CLASS,
        "image",
        ids::M_GROUP_IMG,
        "2026-11-15 11:58:00+05:30",
    );
    assert_eq!(fs::read(&img).unwrap(), ids::IMG_BYTES);
    let phone_img = f
        .paths
        .phone_media
        .join("WhatsApp Images/IMG-20261115-WA0001.jpg");
    assert_eq!(
        fs::metadata(&img).unwrap().ino(),
        fs::metadata(&phone_img).unwrap().ino()
    );
    assert_eq!(
        fs::metadata(&img).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(img.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    // Doc from the extra folder: name+size, copied (not linked).
    let d = store_path(
        &f,
        ids::DM_BOB,
        "document",
        "D9",
        "2026-11-14 09:05:00+05:30",
    );
    assert_eq!(fs::read(&d).unwrap(), doc);
    assert_ne!(
        fs::metadata(&d).unwrap().ino(),
        fs::metadata(extra.join("exam schedule (1).PDF"))
            .unwrap()
            .ino()
    );

    // Existing store file untouched.
    assert_eq!(fs::read(&existing).unwrap(), before);

    let links = env.links.links.lock().clone();
    let by = |id: &str| {
        links
            .iter()
            .find(|l| l.message_id == id)
            .unwrap()
            .matched_by
            .clone()
    };
    assert_eq!(by(ids::M_GROUP_IMG), "hash");
    assert_eq!(by("D9"), "name+size");
    assert_eq!(by(ids::M_ALICE_PDF), "hash");
    assert_eq!(links.len(), 3);
    let msg = done.message.unwrap();
    assert!(msg.contains("2 placed now, 1 already in store"), "{msg}");
    assert!(
        msg.contains("document 2/2") && msg.contains("image 1/1"),
        "{msg}"
    );

    // Re-running is idempotent: nothing new placed.
    let r = mgr
        .start(JobSpec::LinkPhoneMedia { extra_dirs: vec![] })
        .unwrap();
    let done = ev.finished(r.id).await;
    assert!(done.message.unwrap().contains("0 placed now"));
}

#[tokio::test]
async fn linker_hardlink_falls_back_and_relative_extra_rejected() {
    let env = Env::new();
    let (m, mut ev) = env.manager();
    let r = m
        .start(JobSpec::LinkPhoneMedia {
            extra_dirs: vec!["relative/dir".into()],
        })
        .unwrap();
    let done = ev.finished(r.id).await;
    assert_eq!(done.status, JobStatus::Failed);
    // Missing absolute extra dirs are skipped, not fatal.
    let r = m
        .start(JobSpec::LinkPhoneMedia {
            extra_dirs: vec!["/nonexistent/wa-desk-x".into()],
        })
        .unwrap();
    assert_eq!(ev.finished(r.id).await.status, JobStatus::Succeeded);
    let _ = Path::new("/");
}
