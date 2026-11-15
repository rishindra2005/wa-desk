//! Conversions, previews and thumbnails with the real tools (ffmpeg, gs, pdftoppm, soffice).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;
use tokio_util::sync::CancellationToken;
use wa_core::ErrorKind;
use wa_core::types::*;
use wa_jobs::{Tools, office_preview_with, thumbnail_with};

fn png(path: &Path, w: u32, h: u32, alpha: bool) {
    if alpha {
        image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 128])
        })
        .save(path)
        .unwrap();
    } else {
        image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        })
        .save(path)
        .unwrap();
    }
}

fn run(cmd: &mut Command) {
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn video(path: &Path) {
    run(Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=10:duration=1.5",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1.5",
            "-shortest",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path));
}

fn pdf(path: &Path) {
    run(Command::new("gs").args(["-q", "-dSAFER", "-dBATCH", "-dNOPAUSE", "-sDEVICE=pdfwrite"])
        .arg(format!("-sOutputFile={}", path.display()))
        .args(["-c", "/Helvetica findfont 24 scalefont setfont 72 700 moveto (Page one) show showpage 72 700 moveto (Page two) show showpage"]));
}

fn probe(path: &Path, entry: &str) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", entry, "-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn spec(input: &Path, kind: ConvertKind, out: Option<&Path>) -> JobSpec {
    JobSpec::Convert(ConvertSpec {
        input: ConvertInput::File {
            path: input.to_string_lossy().into(),
        },
        kind,
        output_dir: out.map(|p| p.to_string_lossy().into()),
    })
}

async fn convert(env: &Env, s: JobSpec) -> JobRecord {
    let (m, mut ev) = env.manager();
    let r = m.start(s).unwrap();
    ev.finished(r.id).await
}

fn outputs(r: &JobRecord) -> Vec<PathBuf> {
    r.outputs.iter().map(|u| url_path(u)).collect()
}

#[tokio::test]
async fn image_jpeg_resize_and_unique_names() {
    let env = Env::new();
    let src = env.dir.path().join("photo.png");
    png(&src, 800, 600, false);
    let out = env.dir.path().join("out");
    fs::create_dir(&out).unwrap();
    let kind = ConvertKind::ImageCompress {
        quality: 70,
        max_dim: 200,
        format: ImageFormat::Jpeg,
    };
    let a = convert(&env, spec(&src, kind.clone(), Some(&out))).await;
    assert_eq!(a.status, JobStatus::Succeeded, "{a:?}");
    let pa = &outputs(&a)[0];
    assert_eq!(pa.file_name().unwrap(), "photo-compressed.jpg");
    let img = image::open(pa).unwrap();
    assert_eq!((img.width(), img.height()), (200, 150));
    let b = convert(&env, spec(&src, kind, Some(&out))).await;
    assert_eq!(
        outputs(&b)[0].file_name().unwrap(),
        "photo-compressed (1).jpg"
    );
    assert_eq!(b.message.as_deref(), Some("Saved photo-compressed (1).jpg"));
}

#[tokio::test]
async fn image_png_and_webp_default_to_exports() {
    let env = Env::new();
    let src = env.dir.path().join("logo.png");
    png(&src, 100, 50, true);
    let r = convert(
        &env,
        spec(
            &src,
            ConvertKind::ImageCompress {
                quality: 80,
                max_dim: 1600,
                format: ImageFormat::Png,
            },
            None,
        ),
    )
    .await;
    let p = &outputs(&r)[0];
    assert!(p.starts_with(&env.paths.exports));
    assert_eq!(image::open(p).unwrap().width(), 100, "never upscales");
    let r = convert(
        &env,
        spec(
            &src,
            ConvertKind::ImageCompress {
                quality: 60,
                max_dim: 40,
                format: ImageFormat::Webp,
            },
            None,
        ),
    )
    .await;
    assert_eq!(r.status, JobStatus::Succeeded, "{r:?}");
    let p = &outputs(&r)[0];
    assert_eq!(p.extension().unwrap(), "webp");
    assert_eq!(probe(p, "stream=width"), "40");
}

#[tokio::test]
async fn image_garbage_is_invalid() {
    let env = Env::new();
    let src = env.dir.path().join("fake.jpg");
    fs::write(&src, b"not an image").unwrap();
    let r = convert(
        &env,
        spec(
            &src,
            ConvertKind::ImageCompress {
                quality: 80,
                max_dim: 100,
                format: ImageFormat::Jpeg,
            },
            None,
        ),
    )
    .await;
    assert_eq!(r.status, JobStatus::Failed);
    assert_eq!(r.error.as_deref(), Some("unsupported or damaged image"));
}

#[tokio::test]
async fn video_compress_caps_height() {
    let env = Env::new();
    let src = env.dir.path().join("clip.mp4");
    video(&src);
    let r = convert(
        &env,
        spec(
            &src,
            ConvertKind::VideoCompress {
                crf: 32,
                max_height: 120,
            },
            None,
        ),
    )
    .await;
    assert_eq!(r.status, JobStatus::Succeeded, "{r:?}");
    let p = &outputs(&r)[0];
    assert_eq!(p.file_name().unwrap(), "clip-compressed.mp4");
    assert_eq!(probe(p, "stream=height"), "120");
    assert_eq!(probe(p, "stream=codec_name"), "h264");
}

#[tokio::test]
async fn audio_to_opus() {
    let env = Env::new();
    let src = env.dir.path().join("voice.wav");
    run(Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=1",
        ])
        .arg(&src));
    let r = convert(
        &env,
        spec(&src, ConvertKind::AudioToOpus { bitrate_kbps: 32 }, None),
    )
    .await;
    let p = &outputs(&r)[0];
    assert_eq!(p.file_name().unwrap(), "voice-opus.ogg");
    assert_eq!(probe(p, "stream=codec_name"), "opus");
}

#[tokio::test]
async fn pdf_compress_and_pages() {
    let env = Env::new();
    let src = env.dir.path().join("doc.pdf");
    pdf(&src);
    let r = convert(
        &env,
        spec(
            &src,
            ConvertKind::PdfCompress {
                level: "screen".into(),
            },
            None,
        ),
    )
    .await;
    assert_eq!(r.status, JobStatus::Succeeded, "{r:?}");
    let p = &outputs(&r)[0];
    assert_eq!(p.file_name().unwrap(), "doc-compressed.pdf");
    assert!(fs::read(p).unwrap().starts_with(b"%PDF"));
    let r = convert(&env, spec(&src, ConvertKind::PdfToImages { dpi: 40 }, None)).await;
    let names: Vec<_> = outputs(&r)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["doc-page-1.png", "doc-page-2.png"]);
    assert!(r.message.unwrap().starts_with("Saved 2 files"));
}

#[tokio::test]
async fn office_to_pdf_and_cached_preview() {
    let env = Env::new();
    let txt = env.dir.path().join("notes.txt");
    fs::write(&txt, "Hello from wa-desk\nSecond line\n").unwrap();
    let docx_dir = env.dir.path().join("docx");
    run(Command::new("soffice")
        .arg(format!(
            "-env:UserInstallation=file://{}/lo-test",
            env.dir.path().display()
        ))
        .args(["--headless", "--convert-to", "docx", "--outdir"])
        .arg(&docx_dir)
        .arg(&txt));
    let docx = docx_dir.join("notes.docx");
    assert!(docx.exists());
    let r = convert(&env, spec(&docx, ConvertKind::OfficeToPdf, None)).await;
    assert_eq!(r.status, JobStatus::Succeeded, "{r:?}");
    let p = &outputs(&r)[0];
    assert_eq!(p.file_name().unwrap(), "notes.pdf");
    assert!(fs::read(p).unwrap().starts_with(b"%PDF"));

    let tools = Tools::default();
    let c = CancellationToken::new();
    let a = office_preview_with(&env.paths, &tools, &docx, &c)
        .await
        .unwrap();
    assert!(a.starts_with(env.paths.cache.join("previews")));
    let t0 = std::time::Instant::now();
    let b = office_preview_with(&env.paths, &tools, &docx, &c)
        .await
        .unwrap();
    assert_eq!(a, b);
    assert!(t0.elapsed().as_millis() < 200, "served from cache");
}

#[tokio::test]
async fn message_input_downloads_first() {
    let env = Env::new();
    let src = env.dir.path().join("p.png");
    png(&src, 64, 64, false);
    env.bridge
        .content
        .lock()
        .insert("M1".into(), fs::read(&src).unwrap());
    let mut row = media("c@s.whatsapp.net", "M1", MediaKind::Image, 0);
    row.filename = Some("holiday.png".into());
    env.store.rows.lock().push(row);
    let r = convert(
        &env,
        JobSpec::Convert(ConvertSpec {
            input: ConvertInput::Message {
                chat_jid: "c@s.whatsapp.net".into(),
                message_id: "M1".into(),
            },
            kind: ConvertKind::ImageCompress {
                quality: 80,
                max_dim: 32,
                format: ImageFormat::Jpeg,
            },
            output_dir: None,
        }),
    )
    .await;
    assert_eq!(r.status, JobStatus::Succeeded, "{r:?}");
    assert_eq!(
        outputs(&r)[0].file_name().unwrap(),
        "holiday-compressed.jpg"
    );
    assert!(!env.bridge.calls.lock()[0].2, "may ask the phone");
    let r = convert(
        &env,
        JobSpec::Convert(ConvertSpec {
            input: ConvertInput::Message {
                chat_jid: "c".into(),
                message_id: "nope".into(),
            },
            kind: ConvertKind::OfficeToPdf,
            output_dir: None,
        }),
    )
    .await;
    assert_eq!(r.error.as_deref(), Some("message has no media"));
}

#[tokio::test]
async fn bad_inputs_and_missing_tools() {
    let env = Env::new();
    let r = convert(
        &env,
        spec(
            Path::new("relative.mp4"),
            ConvertKind::VideoCompress {
                crf: 28,
                max_height: 720,
            },
            None,
        ),
    )
    .await;
    assert_eq!(r.error.as_deref(), Some("input path must be absolute"));
    let r = convert(
        &env,
        spec(
            &env.dir.path().join("missing.mp4"),
            ConvertKind::VideoCompress {
                crf: 28,
                max_height: 720,
            },
            None,
        ),
    )
    .await;
    assert_eq!(r.error.as_deref(), Some("input file does not exist"));
    let r = convert(&env, spec(env.dir.path(), ConvertKind::OfficeToPdf, None)).await;
    assert_eq!(r.error.as_deref(), Some("input must be a regular file"));
    let src = env.dir.path().join("clip.mp4");
    fs::write(&src, b"x").unwrap();
    let r = convert(
        &env,
        spec(
            &src,
            ConvertKind::VideoCompress {
                crf: 28,
                max_height: 720,
            },
            Some(Path::new("/nonexistent/out")),
        ),
    )
    .await;
    assert_eq!(r.error.as_deref(), Some("output folder does not exist"));
    let mut deps = env.deps();
    deps.tools.ffmpeg = "/nonexistent/ffmpeg".into();
    deps.tools.ffprobe = "/nonexistent/ffprobe".into();
    let (m, mut ev) = env.manager_with(deps);
    let rec = m
        .start(spec(
            &src,
            ConvertKind::VideoCompress {
                crf: 28,
                max_height: 720,
            },
            None,
        ))
        .unwrap();
    let r = ev.finished(rec.id).await;
    assert_eq!(r.error.as_deref(), Some("ffmpeg is not installed"));
    // Failed conversions leave no temp files behind.
    let tmp = env.paths.cache.join("tmp");
    assert_eq!(fs::read_dir(&tmp).map(|d| d.count()).unwrap_or(0), 0);
}

#[tokio::test]
async fn thumbnails_for_each_kind_are_cached() {
    let env = Env::new();
    let tools = Tools::default();
    let c = CancellationToken::new();
    let d = env.dir.path();
    let (jpg_src, alpha_src, vid, doc, bad, wav) = (
        d.join("a.png"),
        d.join("b.png"),
        d.join("v.mp4"),
        d.join("d.pdf"),
        d.join("bad.jpg"),
        d.join("s.wav"),
    );
    png(&jpg_src, 1000, 500, false);
    png(&alpha_src, 300, 600, true);
    video(&vid);
    pdf(&doc);
    fs::write(&bad, b"garbage").unwrap();
    fs::write(&wav, b"RIFF").unwrap();
    let t = |p: PathBuf, k| {
        let (paths, tools, c) = (env.paths.clone(), tools.clone(), c.clone());
        async move { thumbnail_with(&paths, &tools, &p, k, &c).await }
    };
    let a = t(jpg_src.clone(), MediaKind::Image).await.unwrap().unwrap();
    assert_eq!(a.extension().unwrap(), "jpg");
    let img = image::open(&a).unwrap();
    assert_eq!((img.width(), img.height()), (256, 128));
    assert_eq!(
        t(jpg_src, MediaKind::Image).await.unwrap().unwrap(),
        a,
        "cached"
    );
    let b = t(alpha_src, MediaKind::Sticker).await.unwrap().unwrap();
    assert_eq!(b.extension().unwrap(), "webp");
    assert_eq!(image::open(&b).unwrap().height(), 256);
    let v = t(vid, MediaKind::Video).await.unwrap().unwrap();
    assert_eq!(image::open(&v).unwrap().width(), 256);
    let p = t(doc.clone(), MediaKind::Document).await.unwrap().unwrap();
    assert!(image::open(&p).unwrap().height() <= 256);
    assert!(t(bad, MediaKind::Image).await.unwrap().is_none());
    assert!(t(wav, MediaKind::Audio).await.unwrap().is_none());
    let txt = d.join("x.docx");
    fs::write(&txt, b"PK").unwrap();
    assert!(t(txt, MediaKind::Document).await.unwrap().is_none());
    for f in fs::read_dir(env.paths.cache.join("thumbs"))
        .unwrap()
        .flatten()
    {
        assert!(!f.file_name().to_string_lossy().ends_with(".tmp"));
    }
    let mut broken = tools.clone();
    broken.pdftoppm = "/nonexistent/pdftoppm".into();
    fs::write(d.join("d2.pdf"), b"%PDF-1.4 other").unwrap();
    let e = thumbnail_with(
        &env.paths,
        &broken,
        &d.join("d2.pdf"),
        MediaKind::Document,
        &c,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::External);
}

#[tokio::test]
async fn thumbnails_job_over_local_media() {
    let env = Env::new();
    let img = env.dir.path().join("i.png");
    png(&img, 50, 50, false);
    let mut a = media("c", "I1", MediaKind::Image, 0);
    a.local_path = Some(img);
    let mut b = media("c", "A1", MediaKind::Audio, 0);
    b.local_path = Some(env.dir.path().join("i.png"));
    env.store
        .rows
        .lock()
        .extend([a, b, media("c", "MISSING", MediaKind::Image, 0)]);
    let r = convert(&env, JobSpec::Thumbnails).await;
    assert_eq!(r.status, JobStatus::Succeeded);
    assert_eq!(
        r.message.as_deref(),
        Some("1 thumbnails ready; 0 files have no preview; 0 failed")
    );
}
