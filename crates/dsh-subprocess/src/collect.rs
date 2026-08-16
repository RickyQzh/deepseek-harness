//! Bounded tail-keep collection for one subprocess output stream.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{CollectedOutput, SubprocessOutputRead};

static SPILL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Bounded in-memory tail of one stream, with an optional whole-stream spill file.
pub struct OutputCollector {
    max_bytes: usize,
    spill_max_bytes: Option<usize>,
    label: String,
    spill_dir: PathBuf,
    tail: Vec<u8>,
    total: u64,
    truncated: bool,
    spill_path: Option<PathBuf>,
    spill_file: Option<File>,
    spill_disabled: bool,
}

impl OutputCollector {
    /// Create a collector that retains at most `max_bytes` of the stream tail.
    ///
    /// `spill_max_bytes` `None` disables spilling. `label` is included in the spill file name. Spill files are created under `spill_dir` with mode `0o600` and exclusive create (`wx`).
    #[must_use]
    pub fn new(
        max_bytes: usize,
        spill_max_bytes: Option<usize>,
        label: &str,
        spill_dir: &Path,
    ) -> Self {
        Self {
            max_bytes,
            spill_max_bytes,
            label: label.to_string(),
            spill_dir: spill_dir.to_path_buf(),
            tail: Vec::new(),
            total: 0,
            truncated: false,
            spill_path: None,
            spill_file: None,
            spill_disabled: spill_max_bytes.is_none(),
        }
    }

    /// Ingest one chunk. Every byte counts toward the whole-stream total.
    ///
    /// When the in-memory tail would exceed `max_bytes`, bytes are dropped from the head so the retained window is byte-exact. A spill file, when enabled, receives the complete stream until `total` exceeds `spill_max_bytes`.
    pub fn push(&mut self, chunk: &[u8]) {
        self.total += chunk.len() as u64;
        let overflows = self.tail.len().saturating_add(chunk.len()) > self.max_bytes;
        if !self.spill_disabled && (overflows || self.spill_file.is_some()) {
            self.spill_all(chunk);
        }
        self.tail.extend_from_slice(chunk);
        if self.tail.len() > self.max_bytes {
            let excess = self.tail.len() - self.max_bytes;
            self.tail.drain(..excess);
            self.truncated = true;
        }
    }

    /// Read from a whole-stream byte offset.
    ///
    /// `lossy` is true when `from_byte` is before the retained tail window; the whole tail is then returned. `next_offset` is the whole-stream total.
    #[must_use]
    pub fn read_from(&self, from_byte: u64) -> SubprocessOutputRead {
        let window_start = self.total.saturating_sub(self.tail.len() as u64);
        let lossy = from_byte < window_start;
        let text = if lossy {
            String::from_utf8_lossy(&self.tail).into_owned()
        } else {
            let skip =
                usize::try_from(from_byte.saturating_sub(window_start)).unwrap_or(usize::MAX);
            let slice = self.tail.get(skip..).unwrap_or(&[]);
            String::from_utf8_lossy(slice).into_owned()
        };
        SubprocessOutputRead {
            text,
            next_offset: self.total,
            lossy,
            spill_path: self
                .spill_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }

    /// Close the spill file. Idempotent. In-memory reads keep working.
    ///
    /// A failed flush stops advertising the spill path: the file may be missing its tail.
    pub fn seal(&mut self) {
        if let Some(file) = self.spill_file.take() {
            if file.sync_all().is_err() {
                self.spill_path = None;
            }
        }
    }

    /// Seal the spill file and return the retained tail.
    ///
    /// `truncated` is true when bytes were dropped from `text`. `spill_path` is present when a spill file was created and remains intact.
    #[must_use]
    pub fn finalize(&mut self) -> CollectedOutput {
        self.seal();
        CollectedOutput {
            text: String::from_utf8_lossy(&self.tail).into_owned(),
            truncated: self.truncated,
            spill_path: self
                .spill_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }

    fn spill_all(&mut self, chunk: &[u8]) {
        if let Some(max) = self.spill_max_bytes {
            if self.total > max as u64 {
                self.discard_spill();
                return;
            }
        }
        if self.spill_file.is_none() {
            let Some((path, mut file)) = create_spill_file(&self.spill_dir, &self.label) else {
                self.spill_disabled = true;
                return;
            };
            if file.write_all(&self.tail).is_err() {
                drop(file);
                let _ = std::fs::remove_file(&path);
                self.spill_disabled = true;
                return;
            }
            self.spill_path = Some(path);
            self.spill_file = Some(file);
        }
        if let Some(file) = self.spill_file.as_mut() {
            if file.write_all(chunk).is_err() {
                self.discard_spill();
            }
        }
    }

    fn discard_spill(&mut self) {
        self.spill_disabled = true;
        self.spill_file = None;
        if let Some(path) = self.spill_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn create_spill_file(dir: &Path, label: &str) -> Option<(PathBuf, File)> {
    for _ in 0..8 {
        let n = SPILL_COUNTER.fetch_add(1, Ordering::Relaxed);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = dir.join(format!(
            "dsh-subprocess-{}-{n}-{unique:x}-{label}.log",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Some((path, file)),
            Err(_) => continue,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::OutputCollector;
    use std::path::PathBuf;

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dsh-collect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn under_cap_is_not_lossy() {
        let mut c = OutputCollector::new(8, None, "stdout", &dir());
        c.push(b"hello");
        let read = c.read_from(0);
        assert_eq!(read.text, "hello");
        assert!(!read.lossy);
        assert_eq!(read.next_offset, 5);
        let out = c.finalize();
        assert_eq!(out.text, "hello");
        assert!(!out.truncated);
        assert!(out.spill_path.is_none());
    }

    #[test]
    fn over_cap_keeps_the_tail() {
        let mut c = OutputCollector::new(4, None, "stdout", &dir());
        c.push(b"abcdef");
        let read = c.read_from(0);
        assert_eq!(read.text, "cdef");
        assert!(read.lossy);
        let out = c.finalize();
        assert_eq!(out.text, "cdef");
        assert!(out.truncated);
    }
}
