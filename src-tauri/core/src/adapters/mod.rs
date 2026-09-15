//! Provider adapters: each turns whatever a tool leaves on disk (or exposes
//! locally) into a [`ProviderSnapshot`](crate::model::ProviderSnapshot).
//!
//! Two rules hold across all of them:
//!
//! 1. **Read-only.** Adapters never write to, lock, or modify another tool's
//!    state. Worst case they report [`Health::Unavailable`](crate::model::Health).
//! 2. **No invented numbers.** If a value can't be read, the snapshot degrades
//!    to a visible status instead of guessing, and anything derived rather than
//!    reported is flagged with [`UsageWindow::estimated`](crate::model::UsageWindow).

pub mod claude;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod ollama;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};

/// Exponential back-off with jitter, used for endpoints that rate limit us.
///
/// Claude's usage endpoint is the motivating case: it answers 429 with a
/// `Retry-After`, and hammering it is both rude and counterproductive.
#[derive(Debug, Clone)]
pub struct Backoff {
    consecutive_failures: u32,
    next_attempt: Option<DateTime<Utc>>,
    base: Duration,
    ceiling: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(Duration::from_secs(30), Duration::from_secs(30 * 60))
    }
}

impl Backoff {
    pub fn new(base: Duration, ceiling: Duration) -> Self {
        Self {
            consecutive_failures: 0,
            next_attempt: None,
            base,
            ceiling,
        }
    }

    /// Whether a request may be made now.
    pub fn ready_at(&self, now: DateTime<Utc>) -> bool {
        self.next_attempt.is_none_or(|t| now >= t)
    }

    /// When the next attempt is allowed, if we are currently holding off.
    pub fn next_attempt(&self) -> Option<DateTime<Utc>> {
        self.next_attempt
    }

    pub fn is_backing_off(&self) -> bool {
        self.consecutive_failures > 0
    }

    /// Clear the penalty after a good response.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.next_attempt = None;
    }

    /// Record a failure and schedule the next attempt.
    ///
    /// `retry_after` (from the response header) always wins when present;
    /// otherwise the delay doubles per consecutive failure up to the ceiling,
    /// with +/-20% jitter so several clients don't retry in lockstep.
    pub fn record_failure(
        &mut self,
        now: DateTime<Utc>,
        retry_after: Option<Duration>,
        jitter: f64,
    ) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);

        let delay = match retry_after {
            Some(d) => d.min(self.ceiling),
            None => {
                let shift = (self.consecutive_failures - 1).min(16);
                let scaled = self.base.saturating_mul(1u32 << shift);
                let capped = scaled.min(self.ceiling);
                // jitter in [0,1) maps to a +/-20% multiplier
                let factor = 0.8 + 0.4 * jitter.clamp(0.0, 1.0);
                Duration::from_secs_f64(capped.as_secs_f64() * factor)
            }
        };

        self.next_attempt = Some(now + chrono::Duration::from_std(delay).unwrap_or_default());
    }
}

/// Parse a JSON field that may hold an RFC 3339 string or a Unix timestamp.
///
/// Providers are inconsistent here (and change over time), so accept seconds,
/// milliseconds and strings alike rather than binding to one shape.
pub fn parse_timestamp(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    match value {
        serde_json::Value::String(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&Utc));
            }
            // Some writers omit the timezone; assume UTC rather than dropping it.
            if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
                return Some(Utc.from_utc_datetime(&naive));
            }
            // Or the value is a number in a string.
            s.parse::<i64>().ok().and_then(from_unix)
        }
        serde_json::Value::Number(n) => n.as_i64().and_then(from_unix),
        _ => None,
    }
}

/// Interpret an integer as seconds or milliseconds since the epoch.
fn from_unix(n: i64) -> Option<DateTime<Utc>> {
    // Anything past ~year 2286 in seconds is really milliseconds.
    const MS_THRESHOLD: i64 = 10_000_000_000;
    if n.abs() >= MS_THRESHOLD {
        Utc.timestamp_millis_opt(n).single()
    } else {
        Utc.timestamp_opt(n, 0).single()
    }
}

/// Read up to `max_bytes` from the end of a file and return whole lines.
///
/// Agent transcripts grow to tens of megabytes; only the tail says what the
/// session is doing right now, so reading the whole file would be wasteful.
/// The first (possibly partial) line of the window is dropped.
pub fn tail_lines(path: &Path, max_bytes: u64) -> std::io::Result<Vec<String>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;

    let mut buf = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut buf)?;

    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    // When we started mid-file the first line is a fragment.
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines.retain(|l| !l.trim().is_empty());
    Ok(lines)
}

/// Files under `dir` with the given extension, newest modification first.
///
/// Recurses (agent tools nest transcripts per project) and never follows
/// symlinks, so a stray link can't send us wandering the filesystem.
pub fn newest_files(dir: &Path, extension: &str, limit: usize) -> Vec<PathBuf> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = walkdir::WalkDir::new(dir)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case(extension))
        })
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.into_path()))
        })
        .collect();

    found.sort_by_key(|a| std::cmp::Reverse(a.0));
    found.into_iter().take(limit).map(|(_, p)| p).collect()
}

/// The user profile directory (`%USERPROFILE%` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// A human label for a working directory: its last component.
pub fn project_label(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn backoff_starts_ready_and_clears_after_success() {
        let now = at("2026-01-01T00:00:00Z");
        let mut b = Backoff::default();
        assert!(b.ready_at(now));
        assert!(!b.is_backing_off());

        b.record_failure(now, None, 0.5);
        assert!(b.is_backing_off());
        assert!(!b.ready_at(now));

        b.record_success();
        assert!(b.ready_at(now));
        assert!(!b.is_backing_off());
    }

    #[test]
    fn backoff_delay_doubles_and_respects_the_ceiling() {
        let now = at("2026-01-01T00:00:00Z");
        let mut b = Backoff::new(Duration::from_secs(10), Duration::from_secs(60));

        // Jitter fixed at 0.5 => factor 1.0, so delays are exactly base*2^n.
        b.record_failure(now, None, 0.5);
        assert_eq!(
            b.next_attempt().unwrap(),
            now + chrono::Duration::seconds(10)
        );

        b.record_failure(now, None, 0.5);
        assert_eq!(
            b.next_attempt().unwrap(),
            now + chrono::Duration::seconds(20)
        );

        b.record_failure(now, None, 0.5);
        assert_eq!(
            b.next_attempt().unwrap(),
            now + chrono::Duration::seconds(40)
        );

        // Fourth would be 80s but the ceiling is 60s.
        b.record_failure(now, None, 0.5);
        assert_eq!(
            b.next_attempt().unwrap(),
            now + chrono::Duration::seconds(60)
        );
    }

    #[test]
    fn backoff_honours_retry_after_over_its_own_schedule() {
        let now = at("2026-01-01T00:00:00Z");
        let mut b = Backoff::new(Duration::from_secs(30), Duration::from_secs(1800));
        b.record_failure(now, Some(Duration::from_secs(5)), 0.5);
        assert_eq!(
            b.next_attempt().unwrap(),
            now + chrono::Duration::seconds(5)
        );

        // But a hostile Retry-After can't park us past the ceiling.
        b.record_failure(now, Some(Duration::from_secs(86_400)), 0.5);
        assert_eq!(
            b.next_attempt().unwrap(),
            now + chrono::Duration::seconds(1800)
        );
    }

    #[test]
    fn backoff_jitter_spreads_retries() {
        let now = at("2026-01-01T00:00:00Z");
        let delay_with = |j: f64| {
            let mut b = Backoff::new(Duration::from_secs(100), Duration::from_secs(1000));
            b.record_failure(now, None, j);
            (b.next_attempt().unwrap() - now).num_seconds()
        };
        assert_eq!(delay_with(0.0), 80, "-20%");
        assert_eq!(delay_with(1.0), 120, "+20%");
        // Out-of-range jitter is clamped, not propagated.
        assert_eq!(delay_with(99.0), 120);
    }

    #[test]
    fn timestamps_parse_from_every_shape_providers_use() {
        use serde_json::json;
        let expect = at("2026-01-01T00:00:00Z");

        assert_eq!(
            parse_timestamp(&json!("2026-01-01T00:00:00Z")),
            Some(expect)
        );
        assert_eq!(
            parse_timestamp(&json!("2026-01-01T01:00:00+01:00")),
            Some(expect)
        );
        assert_eq!(parse_timestamp(&json!("2026-01-01T00:00:00")), Some(expect));
        assert_eq!(parse_timestamp(&json!(1_767_225_600i64)), Some(expect));
        assert_eq!(parse_timestamp(&json!(1_767_225_600_000i64)), Some(expect));
        assert_eq!(parse_timestamp(&json!("1767225600")), Some(expect));

        assert_eq!(parse_timestamp(&json!("not a date")), None);
        assert_eq!(parse_timestamp(&json!(null)), None);
        assert_eq!(parse_timestamp(&json!({"a": 1})), None);
    }

    #[test]
    fn tail_lines_reads_only_the_end_and_drops_the_partial_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut f = File::create(&path).unwrap();
        for i in 0..500 {
            writeln!(f, "{{\"n\":{i},\"pad\":\"{}\"}}", "x".repeat(60)).unwrap();
        }
        drop(f);

        let lines = tail_lines(&path, 400).unwrap();
        assert!(!lines.is_empty());
        assert!(lines.len() < 20, "only the tail should be read");
        // Every retained line must be complete, parseable JSON.
        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|e| panic!("partial line survived: {line:?} ({e})"));
        }
        assert!(lines.last().unwrap().contains("\"n\":499"));
    }

    #[test]
    fn tail_lines_returns_whole_small_files_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\n").unwrap();
        let lines = tail_lines(&path, 1_000_000).unwrap();
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}"]);
    }

    #[test]
    fn newest_files_sorts_by_modification_and_filters_extension() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("project-a");
        std::fs::create_dir_all(&nested).unwrap();

        std::fs::write(dir.path().join("a.jsonl"), "1").unwrap();
        std::fs::write(nested.join("b.jsonl"), "2").unwrap();
        std::fs::write(dir.path().join("ignore.txt"), "3").unwrap();

        // Make b.jsonl clearly newer.
        let later = std::time::SystemTime::now() + Duration::from_secs(60);
        let f = File::options()
            .write(true)
            .open(nested.join("b.jsonl"))
            .unwrap();
        f.set_modified(later).unwrap();

        let found = newest_files(dir.path(), "jsonl", 10);
        assert_eq!(found.len(), 2, "the .txt must be filtered out");
        assert!(found[0].ends_with("b.jsonl"), "newest first: {found:?}");

        assert_eq!(
            newest_files(dir.path(), "jsonl", 1).len(),
            1,
            "limit applies"
        );
        assert!(newest_files(Path::new("/nope"), "jsonl", 10).is_empty());
    }

    #[test]
    fn project_label_takes_the_last_path_component() {
        assert_eq!(project_label(r"C:\Users\dev\code\myapp"), "myapp");
        assert_eq!(project_label(r"C:\Users\dev\code\myapp\"), "myapp");
        assert_eq!(project_label("/home/dev/myapp"), "myapp");
        assert_eq!(project_label("myapp"), "myapp");
        assert_eq!(project_label(""), "");
    }
}
