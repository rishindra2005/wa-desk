//! Minimal, strict HTTP `Range` handling for the `wa-media://` protocol.
//! WebKitGTK needs 206 responses to seek in <video>/<audio>.

/// A satisfiable byte range `[start, end]` (inclusive) within a file of `len` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    pub fn byte_len(&self) -> u64 {
        self.end - self.start + 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOutcome {
    /// No (or ignorable) Range header: send the whole file with 200.
    Full,
    /// Send 206 with this range.
    Partial(ByteRange),
    /// 416 Range Not Satisfiable.
    Unsatisfiable,
}

/// Largest chunk returned for one open-ended range request (keeps memory bounded
/// for multi-GB videos; the player simply asks for the next chunk).
pub const MAX_CHUNK: u64 = 8 * 1024 * 1024;

/// Parse a single-range `bytes=` header. Multi-range requests are answered with
/// the first range (allowed by RFC 9110). Malformed headers are ignored (Full).
pub fn parse_range(header: Option<&str>, len: u64) -> RangeOutcome {
    let Some(h) = header.map(str::trim) else {
        return RangeOutcome::Full;
    };
    let Some(spec) = h.strip_prefix("bytes=") else {
        return RangeOutcome::Full;
    };
    let first = spec.split(',').next().unwrap_or("").trim();
    let Some((a, b)) = first.split_once('-') else {
        return RangeOutcome::Full;
    };
    let (a, b) = (a.trim(), b.trim());
    if len == 0 {
        return RangeOutcome::Unsatisfiable;
    }
    let range = match (a.is_empty(), b.is_empty()) {
        // "-N": last N bytes
        (true, false) => {
            let Ok(n) = b.parse::<u64>() else {
                return RangeOutcome::Full;
            };
            if n == 0 {
                return RangeOutcome::Unsatisfiable;
            }
            let n = n.min(len);
            ByteRange {
                start: len - n,
                end: len - 1,
            }
        }
        // "N-": from N to end (capped)
        (false, true) => {
            let Ok(s) = a.parse::<u64>() else {
                return RangeOutcome::Full;
            };
            if s >= len {
                return RangeOutcome::Unsatisfiable;
            }
            ByteRange {
                start: s,
                end: (s + MAX_CHUNK - 1).min(len - 1),
            }
        }
        // "N-M"
        (false, false) => {
            let (Ok(s), Ok(e)) = (a.parse::<u64>(), b.parse::<u64>()) else {
                return RangeOutcome::Full;
            };
            if s > e {
                return RangeOutcome::Full;
            }
            if s >= len {
                return RangeOutcome::Unsatisfiable;
            }
            ByteRange {
                start: s,
                end: e.min(len - 1),
            }
        }
        (true, true) => return RangeOutcome::Full,
    };
    RangeOutcome::Partial(range)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_when_absent_or_malformed() {
        assert_eq!(parse_range(None, 100), RangeOutcome::Full);
        assert_eq!(parse_range(Some("items=0-1"), 100), RangeOutcome::Full);
        assert_eq!(parse_range(Some("bytes=abc-def"), 100), RangeOutcome::Full);
        assert_eq!(parse_range(Some("bytes=-"), 100), RangeOutcome::Full);
        assert_eq!(parse_range(Some("bytes=9-3"), 100), RangeOutcome::Full);
    }

    #[test]
    fn explicit_ranges() {
        assert_eq!(
            parse_range(Some("bytes=0-0"), 100),
            RangeOutcome::Partial(ByteRange { start: 0, end: 0 })
        );
        assert_eq!(
            parse_range(Some("bytes=10-19"), 100),
            RangeOutcome::Partial(ByteRange { start: 10, end: 19 })
        );
        assert_eq!(
            parse_range(Some("bytes=90-500"), 100),
            RangeOutcome::Partial(ByteRange { start: 90, end: 99 })
        );
        assert_eq!(
            parse_range(Some(" bytes=0-1, 5-6"), 100),
            RangeOutcome::Partial(ByteRange { start: 0, end: 1 })
        );
    }

    #[test]
    fn open_and_suffix_ranges() {
        assert_eq!(
            parse_range(Some("bytes=-10"), 100),
            RangeOutcome::Partial(ByteRange { start: 90, end: 99 })
        );
        assert_eq!(
            parse_range(Some("bytes=-1000"), 100),
            RangeOutcome::Partial(ByteRange { start: 0, end: 99 })
        );
        assert_eq!(
            parse_range(Some("bytes=50-"), 100),
            RangeOutcome::Partial(ByteRange { start: 50, end: 99 })
        );
        let big = 10 * MAX_CHUNK;
        let r = parse_range(Some("bytes=0-"), big);
        assert_eq!(
            r,
            RangeOutcome::Partial(ByteRange {
                start: 0,
                end: MAX_CHUNK - 1
            })
        );
    }

    #[test]
    fn unsatisfiable() {
        assert_eq!(
            parse_range(Some("bytes=100-"), 100),
            RangeOutcome::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=200-300"), 100),
            RangeOutcome::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=-0"), 100),
            RangeOutcome::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=0-1"), 0),
            RangeOutcome::Unsatisfiable
        );
    }

    #[test]
    fn len_is_inclusive() {
        assert_eq!(ByteRange { start: 5, end: 5 }.byte_len(), 1);
        assert_eq!(ByteRange { start: 0, end: 99 }.byte_len(), 100);
    }
}
