//! Parsing of `wa-media://localhost/<kind>/<a>/<b>` request paths (see docs/CONTRACT.md).
//! Pure and side-effect free so it can be exhaustively tested; the protocol
//! handler resolves the parsed request to a file and scope-checks it.

use percent_encoding::percent_decode_str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRequest {
    /// Original media file of a message.
    Original {
        chat_jid: String,
        message_id: String,
    },
    /// 256px thumbnail.
    Thumb {
        chat_jid: String,
        message_id: String,
    },
    /// Cached preview PDF of an office document.
    Preview {
        chat_jid: String,
        message_id: String,
    },
    /// An exported/converted file by absolute path (must be inside exports/cache).
    Export { path: String },
    /// Profile picture of a chat or person (`?full=1` for the large one).
    Avatar { jid: String, full: bool },
}

fn decode(seg: &str) -> Option<String> {
    let s = decode_any(seg)?;
    // A leftover '%' means an invalid escape (JIDs/message ids never contain '%').
    if s.contains('%') { None } else { Some(s) }
}

/// Decode allowing literal '%' (file names may contain it).
fn decode_any(seg: &str) -> Option<String> {
    let s = percent_decode_str(seg).decode_utf8().ok()?.into_owned();
    if s.is_empty() || s.contains('\0') {
        None
    } else {
        Some(s)
    }
}

/// Parse the URI path (with or without leading '/'). The query string is
/// only used for `a` (`full=1`).
pub fn parse(path: &str) -> Option<MediaRequest> {
    let (path, query) = {
        let no_frag = path.split('#').next().unwrap_or("");
        match no_frag.split_once('?') {
            Some((p, q)) => (p, q),
            None => (no_frag, ""),
        }
    };
    let mut it = path.trim_start_matches('/').splitn(3, '/');
    let kind = it.next()?;
    let a = it.next()?;
    match kind {
        "x" => {
            // Single segment holding the fully percent-encoded absolute path.
            if it.next().is_some() {
                return None;
            }
            let p = decode_any(a)?;
            p.starts_with('/')
                .then_some(MediaRequest::Export { path: p })
        }
        "a" => {
            if it.next().is_some() {
                return None;
            }
            let jid = decode(a)?;
            if jid.contains('/') || !jid.contains('@') {
                return None;
            }
            let full = query
                .split('&')
                .any(|kv| kv == "full=1" || kv == "full=true");
            Some(MediaRequest::Avatar { jid, full })
        }
        "m" | "t" | "p" => {
            let b = it.next()?;
            if b.contains('/') {
                return None;
            }
            let (chat_jid, message_id) = (decode(a)?, decode(b)?);
            if chat_jid.contains('/') || message_id.contains('/') {
                return None;
            }
            Some(match kind {
                "m" => MediaRequest::Original {
                    chat_jid,
                    message_id,
                },
                "t" => MediaRequest::Thumb {
                    chat_jid,
                    message_id,
                },
                _ => MediaRequest::Preview {
                    chat_jid,
                    message_id,
                },
            })
        }
        _ => None,
    }
}

/// Build a URL for an export/cache file (mirror of `parse` for `x`).
pub fn export_url(path: &std::path::Path) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    format!(
        "wa-media://localhost/x/{}",
        utf8_percent_encode(&path.to_string_lossy(), NON_ALPHANUMERIC)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_message_media() {
        assert_eq!(
            parse("/m/120363%40g.us/3EB0ABC"),
            Some(MediaRequest::Original {
                chat_jid: "120363@g.us".into(),
                message_id: "3EB0ABC".into()
            })
        );
        assert_eq!(
            parse("t/91999%3A12%40s.whatsapp.net/X?v=2"),
            Some(MediaRequest::Thumb {
                chat_jid: "91999:12@s.whatsapp.net".into(),
                message_id: "X".into()
            })
        );
        assert!(matches!(
            parse("/p/a%40b/Y"),
            Some(MediaRequest::Preview { .. })
        ));
    }

    #[test]
    fn parses_avatars() {
        assert_eq!(
            parse("/a/120363%40g.us"),
            Some(MediaRequest::Avatar {
                jid: "120363@g.us".into(),
                full: false
            })
        );
        assert_eq!(
            parse("/a/9198%40s.whatsapp.net?x=1&full=1"),
            Some(MediaRequest::Avatar {
                jid: "9198@s.whatsapp.net".into(),
                full: true
            })
        );
        for bad in ["/a", "/a/", "/a/nojid", "/a/x%40g.us/extra", "/a/%2F%40x"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn rejects_bad_paths() {
        for p in [
            "",
            "/",
            "/m",
            "/m/a",
            "/m/a/b/c",
            "/z/a/b",
            "/m/%2F/b",
            "/m/a/%2Fetc",
            "/m/a/%00",
            "/m//b",
            "/m/%ZZ/b",
            "/x/relative",
            "/x/%2Fa/extra",
        ] {
            assert_eq!(parse(p), None, "{p}");
        }
    }

    #[test]
    fn export_roundtrip() {
        let p = std::path::Path::new("/home/u/Downloads/wa-desk/My 50% File (1).pdf");
        let url = export_url(p);
        let path = url.strip_prefix("wa-media://localhost").unwrap();
        assert_eq!(
            parse(path),
            Some(MediaRequest::Export {
                path: p.to_string_lossy().into_owned()
            })
        );
    }
}
