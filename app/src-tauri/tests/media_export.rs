//! `export_media_files`: zip contents, name de-duplication, refusals and partial failures.

mod common;

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use common::*;
use wa_core::ErrorKind;
use wa_core::types::*;
use wa_testkit::ids::*;
use wa_testkit::{Msg, TOKEN};

const TS: &str = "2026-11-15 09:00:00+05:30";

fn target(chat: &str, id: &str) -> MediaTarget {
    MediaTarget {
        chat_jid: chat.into(),
        message_id: id.into(),
    }
}

fn alice(id: &str) -> MediaTarget {
    target(DM_ALICE, id)
}

fn req(items: Vec<MediaTarget>, mode: MediaExportMode, dest: &Path) -> MediaExportReq {
    MediaExportReq {
        items,
        mode,
        dest: dest.to_string_lossy().into_owned(),
        request_id: "req-1".into(),
    }
}

/// name → bytes of every entry in the zip.
fn read_zip(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        out.insert(f.name().to_string(), data);
    }
    out
}

fn add_image(h: &H, id: &str, data: &[u8]) {
    h.f.add_message(&Msg::text(id, DM_ALICE, "919811111111", TS, "").media("image", None, data));
    h.f.put_store_media(DM_ALICE, "image", id, TS, data);
}

#[tokio::test(flavor = "multi_thread")]
async fn zip_has_every_file_and_dedupes_names() {
    let h = harness().await;
    add_doc(&h.f, "D1", "report.pdf", b"one");
    add_doc(&h.f, "D2", "report.pdf", b"two");
    add_doc(&h.f, "D3", "Report.PDF", b"three");
    add_image(&h, "I1", b"\xFF\xD8\xFFimage");
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("media.zip");
    let items = vec![
        alice("D1"),
        alice("D2"),
        alice("D3"),
        alice("I1"),
        alice("D1"),
    ];
    let s = h
        .core
        .export_media_files(req(items, MediaExportMode::Zip, &dest))
        .await
        .unwrap();
    assert_eq!((s.saved, s.failed), (4, 0), "{s:?}");
    assert_eq!(s.dest.as_deref(), Some(dest.to_str().unwrap()));
    let zip = read_zip(&dest);
    assert_eq!(zip.len(), 4);
    assert_eq!(zip["report.pdf"], b"one");
    assert_eq!(zip["report (1).pdf"], b"two");
    assert_eq!(zip["Report (2).PDF"], b"three");
    let img = zip.keys().find(|n| n.ends_with(".jpg")).unwrap();
    assert_eq!(zip[img], b"\xFF\xD8\xFFimage");
    assert!(!out.path().join("media.zip.partial").exists());
    // Progress ends at total and carries the request id.
    let events = h.rec.named("media-export:progress");
    assert!(!events.is_empty());
    let last = events.last().unwrap();
    assert_eq!(last["request_id"], "req-1");
    assert_eq!(last["done"], last["total"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn zip_adds_extension_and_streams_larger_files() {
    let h = harness().await;
    let big = vec![7u8; 3 * 1024 * 1024];
    add_doc(&h.f, "BIG", "big.bin", &big);
    let out = tempfile::tempdir().unwrap();
    let s = h
        .core
        .export_media_files(req(
            vec![alice("BIG")],
            MediaExportMode::Zip,
            &out.path().join("bundle"),
        ))
        .await
        .unwrap();
    assert_eq!(s.saved, 1);
    let zip = read_zip(&out.path().join("bundle.zip"));
    assert_eq!(zip["big.bin"].len(), big.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_keeps_names_and_never_overwrites() {
    let h = harness().await;
    add_doc(&h.f, "D1", "report.pdf", b"one");
    add_doc(&h.f, "D2", "report.pdf", b"two");
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("report.pdf"), b"precious").unwrap();
    let s = h
        .core
        .export_media_files(req(
            vec![alice("D1"), alice("D2")],
            MediaExportMode::Folder,
            out.path(),
        ))
        .await
        .unwrap();
    assert_eq!((s.saved, s.failed), (2, 0), "{s:?}");
    assert_eq!(
        std::fs::read(out.path().join("report.pdf")).unwrap(),
        b"precious"
    );
    assert_eq!(
        std::fs::read(out.path().join("report (1).pdf")).unwrap(),
        b"one"
    );
    assert_eq!(
        std::fs::read(out.path().join("report (2).pdf")).unwrap(),
        b"two"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_requests_and_destinations_are_refused() {
    let h = harness().await;
    let out = tempfile::tempdir().unwrap();
    let ok = vec![alice(M_ALICE_PDF)];
    let store = h.f.paths.bridge_store.clone();
    let cases: Vec<(MediaExportReq, ErrorKind)> = vec![
        (
            req(vec![], MediaExportMode::Zip, &out.path().join("a.zip")),
            ErrorKind::Invalid,
        ),
        (
            req(
                (0..501).map(|i| alice(&format!("X{i}"))).collect(),
                MediaExportMode::Zip,
                &out.path().join("a.zip"),
            ),
            ErrorKind::Invalid,
        ),
        (
            req(ok.clone(), MediaExportMode::Zip, Path::new("relative.zip")),
            ErrorKind::Invalid,
        ),
        (
            req(
                ok.clone(),
                MediaExportMode::Zip,
                &out.path().join("missing/a.zip"),
            ),
            ErrorKind::Invalid,
        ),
        (
            req(ok.clone(), MediaExportMode::Zip, &store.join("a.zip")),
            ErrorKind::Forbidden,
        ),
        (
            req(
                ok.clone(),
                MediaExportMode::Zip,
                &out.path().join(".bridge-token"),
            ),
            ErrorKind::Forbidden,
        ),
        (
            req(
                ok.clone(),
                MediaExportMode::Zip,
                &out.path().join("evil.db"),
            ),
            ErrorKind::Forbidden,
        ),
        (
            req(ok.clone(), MediaExportMode::Folder, Path::new("rel")),
            ErrorKind::Invalid,
        ),
        (
            req(
                ok.clone(),
                MediaExportMode::Folder,
                &out.path().join("nope"),
            ),
            ErrorKind::Invalid,
        ),
        (
            req(ok.clone(), MediaExportMode::Folder, &store),
            ErrorKind::Forbidden,
        ),
        (
            req(
                ok.clone(),
                MediaExportMode::Folder,
                &store.join(DM_ALICE.replace(':', "_")),
            ),
            ErrorKind::Forbidden,
        ),
    ];
    for (r, kind) in cases {
        let dest = r.dest.clone();
        let e = h.core.export_media_files(r).await.unwrap_err();
        assert_eq!(e.kind, kind, "{dest}");
    }
    let mut blank = req(ok, MediaExportMode::Zip, &out.path().join("a.zip"));
    blank.request_id = " ".into();
    assert_eq!(
        h.core.export_media_files(blank).await.unwrap_err().kind,
        ErrorKind::Invalid
    );
    assert!(std::fs::read_dir(out.path()).unwrap().next().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_databases_and_outside_files_are_never_exported() {
    let h = harness().await;
    let store_file = |id: &str| {
        wa_core::paths::bridge_media_path(&h.f.paths.bridge_store, DM_ALICE, "document", id, TS)
            .unwrap()
    };
    let doc = |id: &str, name: &str| {
        h.f.add_message(&Msg::text(id, DM_ALICE, "919811111111", TS, "").media(
            "document",
            Some(name),
            b"x",
        ));
    };
    // Media whose stored file is a symlink to the token, to a database, and to a file outside every root.
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), b"outside-secret").unwrap();
    for (id, target) in [
        ("S1", h.f.paths.token_file.clone()),
        ("S2", h.f.paths.messages_db.clone()),
        ("S3", outside.path().to_path_buf()),
    ] {
        doc(id, "innocent.pdf");
        let p = store_file(id);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, p).unwrap();
    }
    // A legitimate file whose original name looks like a database.
    add_doc(&h.f, "S4", "messages.db", b"not really a db");
    // And one good file, so the export is partial.
    add_doc(&h.f, "GOOD", "good.pdf", b"fine");
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("x.zip");
    let items = ["S1", "S2", "S3", "S4", "GOOD"].map(alice).to_vec();
    let s = h
        .core
        .export_media_files(req(items, MediaExportMode::Zip, &dest))
        .await
        .unwrap();
    assert_eq!((s.saved, s.failed), (1, 4), "{s:?}");
    let zip = read_zip(&dest);
    assert_eq!(zip.keys().collect::<Vec<_>>(), vec!["good.pdf"]);
    let all: Vec<u8> = zip.values().flatten().copied().collect();
    assert!(!String::from_utf8_lossy(&all).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&all).contains("outside-secret"));
    for f in &s.failures {
        assert!(!f.reason.is_empty());
    }

    // Folder mode refuses the same items and writes only the good one.
    let dir = tempfile::tempdir().unwrap();
    let items = ["S1", "S2", "S3", "S4", "GOOD"].map(alice).to_vec();
    let s = h
        .core
        .export_media_files(req(items, MediaExportMode::Folder, dir.path()))
        .await
        .unwrap();
    assert_eq!((s.saved, s.failed), (1, 4));
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["good.pdf"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_exports_local_files_and_reports_missing_ones() {
    let h = harness().await;
    h.core.set_read_only(true);
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("ro.zip");
    let items = vec![alice(M_ALICE_PDF), target(GROUP_CLASS, M_GROUP_IMG)];
    let s = h
        .core
        .export_media_files(req(items, MediaExportMode::Zip, &dest))
        .await
        .unwrap();
    assert_eq!((s.saved, s.failed), (1, 1), "{s:?}");
    assert_eq!(s.failures[0].message_id, M_GROUP_IMG);
    assert!(
        s.failures[0].reason.contains("read-only"),
        "{:?}",
        s.failures
    );
    assert_eq!(read_zip(&dest).values().next().unwrap(), PDF_BYTES);
    assert!(h.mock.calls_to("/api/download").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_media_is_downloaded_first() {
    let h = harness().await;
    let data = b"\xFF\xD8\xFFdownloaded";
    let p = wa_core::paths::bridge_media_path(
        &h.f.paths.bridge_store,
        GROUP_CLASS,
        "image",
        M_GROUP_IMG,
        "2026-11-15 11:58:00+05:30",
    )
    .unwrap();
    h.mock
        .downloads
        .lock()
        .insert(M_GROUP_IMG.into(), (p, data.to_vec()));
    let out = tempfile::tempdir().unwrap();
    let s = h
        .core
        .export_media_files(req(
            vec![target(GROUP_CLASS, M_GROUP_IMG)],
            MediaExportMode::Folder,
            out.path(),
        ))
        .await
        .unwrap();
    assert_eq!((s.saved, s.failed), (1, 0), "{s:?}");
    assert_eq!(h.mock.calls_to("/api/download").len(), 1);
    let file = std::fs::read_dir(out.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(file.path()).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_saved_leaves_no_file() {
    let h = harness().await;
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("none.zip");
    // Not downloaded and the bridge cannot fetch it; plus an unknown message.
    let items = vec![target(GROUP_CLASS, M_GROUP_IMG), alice("NOPE")];
    let s = h
        .core
        .export_media_files(req(items, MediaExportMode::Zip, &dest))
        .await
        .unwrap();
    assert_eq!((s.saved, s.failed), (0, 2), "{s:?}");
    assert!(s.dest.is_none());
    assert!(std::fs::read_dir(out.path()).unwrap().next().is_none());
}
