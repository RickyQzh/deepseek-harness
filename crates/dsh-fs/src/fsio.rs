//! Host-filesystem mechanics used by [`crate::LocalFileSystem`].

use std::io::{self, ErrorKind};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dsh_tools::AbortFlag;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{FsError, FsErrorCode, FsInfoType, FsTarget, FsTargetKey, FsVersion};

/// Bytes inspected for a leading NUL when classifying a read as binary.
pub(crate) const BINARY_SAMPLE_BYTES: usize = 8192;

/// Read size for [`stream_whole_text`]. Matches Node `createReadStream` default.
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// Default exclusive UTF-8 byte limit on each overwrite-diff side (10 MiB).
pub(crate) const DEFAULT_DIFF_BASIS_MAX_BYTES: usize = 10 * 1024 * 1024;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Fail with [`FsErrorCode::Aborted`] when `signal` has been aborted.
pub(crate) fn throw_if_aborted(signal: Option<&AbortFlag>, verb: &str) -> Result<(), FsError> {
    if signal.is_some_and(AbortFlag::is_aborted) {
        Err(FsError::new(
            format!("{verb} aborted"),
            FsErrorCode::Aborted,
        ))
    } else {
        Ok(())
    }
}

/// Kernel metadata plus the freshness token for one path.
pub(crate) struct PathInfo {
    pub version: FsVersion,
    pub mode: u32,
    pub kind: FsInfoType,
    pub size: u64,
}

/// `{dev}:{ino}:{size}:{mtime_nsec}:{ctime_nsec}` from [`MetadataExt`].
pub(crate) fn version_of(meta: &std::fs::Metadata) -> FsVersion {
    FsVersion::new(format!(
        "{}:{}:{}:{}:{}",
        meta.dev(),
        meta.ino(),
        meta.size(),
        meta.mtime_nsec(),
        meta.ctime_nsec()
    ))
}

fn kind_of(meta: &std::fs::Metadata) -> FsInfoType {
    if meta.is_file() {
        FsInfoType::File
    } else if meta.is_dir() {
        FsInfoType::Directory
    } else {
        FsInfoType::Other
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn is_absent(err: &io::Error) -> bool {
    matches!(err.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

fn io_fs_error(verb: &str, path: &str, err: io::Error) -> FsError {
    match err.kind() {
        ErrorKind::NotFound => FsError::new(
            format!("cannot {verb} \"{path}\": not found"),
            FsErrorCode::NotFound,
        ),
        ErrorKind::NotADirectory => FsError::new(
            format!("cannot {verb} \"{path}\": a parent path segment is not a directory"),
            FsErrorCode::NotFound,
        ),
        ErrorKind::PermissionDenied => FsError::new(
            format!("cannot {verb} \"{path}\": permission denied"),
            FsErrorCode::PermissionDenied,
        ),
        _ => FsError::new(
            format!("cannot {verb} \"{path}\": {err}"),
            FsErrorCode::IoError,
        ),
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn absolute_join(cwd: &Path, path: &str) -> PathBuf {
    let joined = {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };
    let absolute = if joined.is_absolute() {
        joined
    } else {
        match std::env::current_dir() {
            Ok(base) => base.join(joined),
            Err(_) => joined,
        }
    };
    lexical_normalize(&absolute)
}

/// Join `path` against `cwd`, realpath the deepest existing ancestor, and append the missing suffix.
pub(crate) async fn resolve_local_target(cwd: &Path, path: &str) -> Result<FsTarget, FsError> {
    if path.trim().is_empty() {
        return Err(FsError::new(
            "file_path must be a non-empty string",
            FsErrorCode::NotFound,
        ));
    }
    let absolute = absolute_join(cwd, path);
    let canonical = realpath_existing_or_suffix(&absolute).await?;
    let key = display_path(&canonical);
    Ok(FsTarget {
        target_key: FsTargetKey::new(key.clone()),
        display_path: key,
    })
}

async fn realpath_existing_or_suffix(absolute: &Path) -> Result<PathBuf, FsError> {
    let display = display_path(absolute);
    match fs::canonicalize(absolute).await {
        Ok(resolved) => return Ok(resolved),
        Err(err) if is_absent(&err) => {}
        Err(err) => return Err(io_fs_error("resolve", &display, err)),
    }

    let mut suffix = Vec::new();
    let mut ancestor = absolute.to_path_buf();
    loop {
        let name = ancestor.file_name().map(std::ffi::OsStr::to_os_string);
        let Some(parent) = ancestor.parent() else {
            return Ok(absolute.to_path_buf());
        };
        if parent == ancestor {
            return Ok(absolute.to_path_buf());
        }
        if let Some(name) = name {
            suffix.push(name);
        }
        ancestor = parent.to_path_buf();
        match fs::canonicalize(&ancestor).await {
            Ok(real) => {
                let meta = fs::metadata(&real)
                    .await
                    .map_err(|err| io_fs_error("resolve", &display, err))?;
                if !meta.is_dir() {
                    return Err(FsError::new(
                        format!(
                            "cannot resolve \"{display}\": a parent path segment is not a directory"
                        ),
                        FsErrorCode::NotFound,
                    ));
                }
                let mut out = real;
                for component in suffix.iter().rev() {
                    out.push(component);
                }
                return Ok(out);
            }
            Err(err) if is_absent(&err) => {}
            Err(err) => return Err(io_fs_error("resolve", &display, err)),
        }
    }
}

/// Stat `absolute_path`, following the final symlink. `Ok(None)` when the path is absent.
pub(crate) async fn probe(absolute_path: &str) -> Result<Option<PathInfo>, FsError> {
    match fs::metadata(absolute_path).await {
        Ok(meta) => Ok(Some(PathInfo {
            version: version_of(&meta),
            mode: meta.mode() & 0o777,
            kind: kind_of(&meta),
            size: meta.size(),
        })),
        Err(err) if is_absent(&err) => Ok(None),
        Err(err) => Err(io_fs_error("stat", absolute_path, err)),
    }
}

fn not_text(verb: &str, display: &str, binary: bool) -> FsError {
    let detail = if binary {
        "binary file"
    } else {
        "invalid UTF-8 text"
    };
    FsError::new(
        format!("cannot {verb} \"{display}\": {detail}"),
        FsErrorCode::NotText,
    )
}

async fn require_regular_file(
    io_path: &str,
    display_path: &str,
    verb: &str,
) -> Result<(), FsError> {
    let meta = match fs::metadata(io_path).await {
        Ok(meta) => meta,
        Err(err) if is_absent(&err) => {
            return Err(FsError::new(
                format!("cannot {verb} \"{display_path}\": not found"),
                FsErrorCode::NotFound,
            ));
        }
        Err(err) => return Err(io_fs_error(verb, display_path, err)),
    };
    if !meta.is_file() {
        return Err(FsError::new(
            format!("cannot {verb} \"{display_path}\": not a regular file"),
            FsErrorCode::NotRegularFile,
        ));
    }
    Ok(())
}

fn utf8_stream_valid_up_to(pending: &[u8]) -> Option<usize> {
    match std::str::from_utf8(pending) {
        Ok(_) => Some(pending.len()),
        Err(err) => {
            if err.error_len().is_some() {
                None
            } else {
                Some(err.valid_up_to())
            }
        }
    }
}

fn posix_file_url_keep_ascii(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!$&'()*+,-./:;=@_".contains(&byte)
}

fn push_percent_encoded_byte(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0x0F) as usize] as char);
}

/// POSIX `file:` URI of an absolute host path, matching Node `pathToFileURL` on unix.
///
/// Unencoded ASCII bytes are RFC 3986 `pchar` plus `/`, except `~`, which Node
/// encodes. Other bytes, including non-ASCII UTF-8, are percent-encoded.
pub(crate) fn posix_file_url(process_path: &str) -> String {
    let mut out = String::from("file://");
    for &byte in process_path.as_bytes() {
        if posix_file_url_keep_ascii(byte) {
            out.push(byte as char);
        } else {
            push_percent_encoded_byte(&mut out, byte);
        }
    }
    out
}

/// Read a regular UTF-8 file and normalize `\r\n` to `\n`.
pub(crate) async fn read_whole_text(io_path: &str, display_path: &str) -> Result<String, FsError> {
    require_regular_file(io_path, display_path, "read").await?;
    let bytes = fs::read(io_path)
        .await
        .map_err(|err| io_fs_error("read", display_path, err))?;
    if bytes.iter().take(BINARY_SAMPLE_BYTES).any(|&b| b == 0) {
        return Err(not_text("read", display_path, true));
    }
    let raw = std::str::from_utf8(&bytes).map_err(|_| not_text("read", display_path, false))?;
    Ok(normalize_line_endings(raw))
}

/// Stream a regular UTF-8 file as decoded chunks with no CRLF rewrite.
pub(crate) async fn stream_whole_text(
    io_path: &str,
    display_path: &str,
    signal: Option<&AbortFlag>,
    on_chunk: &mut dyn FnMut(&str) -> Result<(), FsError>,
) -> Result<(), FsError> {
    throw_if_aborted(signal, "read")?;
    require_regular_file(io_path, display_path, "read").await?;
    throw_if_aborted(signal, "read")?;

    let mut file = fs::File::open(io_path)
        .await
        .map_err(|err| io_fs_error("read", display_path, err))?;
    let mut buf = vec![0u8; STREAM_CHUNK_BYTES];
    let mut pending = Vec::new();
    let mut sampled = 0usize;

    loop {
        throw_if_aborted(signal, "read")?;
        let n = file
            .read(&mut buf)
            .await
            .map_err(|err| io_fs_error("read", display_path, err))?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        if sampled < BINARY_SAMPLE_BYTES {
            let take = std::cmp::min(chunk.len(), BINARY_SAMPLE_BYTES - sampled);
            if chunk[..take].contains(&0) {
                return Err(not_text("read", display_path, true));
            }
            sampled += take;
        }
        pending.extend_from_slice(chunk);
        let Some(valid_up_to) = utf8_stream_valid_up_to(&pending) else {
            return Err(not_text("read", display_path, false));
        };
        if valid_up_to == 0 {
            continue;
        }
        let text =
            std::str::from_utf8(&pending[..valid_up_to]).expect("valid_up_to is a UTF-8 prefix");
        if !text.is_empty() {
            on_chunk(text)?;
        }
        pending.drain(..valid_up_to);
    }

    throw_if_aborted(signal, "read")?;
    if !pending.is_empty() {
        return Err(not_text("read", display_path, false));
    }
    Ok(())
}

/// Dominant newline style in a file before LF normalization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LineEndings {
    Lf,
    Crlf,
}

/// Collapse `\r\n` to `\n`. Lone `\r` bytes are left unchanged.
pub(crate) fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n")
}

fn prefix_chars(raw: &str, max_bytes: usize) -> &str {
    if raw.len() <= max_bytes {
        return raw;
    }
    let mut end = max_bytes;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    &raw[..end]
}

fn detect_line_endings(raw: &str) -> LineEndings {
    let sample = prefix_chars(raw, 4096);
    let crlf_count = sample.matches("\r\n").count();
    let lf_count = sample.matches('\n').count().saturating_sub(crlf_count);
    if crlf_count > lf_count {
        LineEndings::Crlf
    } else {
        LineEndings::Lf
    }
}

/// Restore `content` to the newline style detected at read time.
pub(crate) fn restore_line_endings(content: &str, line_endings: LineEndings) -> String {
    match line_endings {
        LineEndings::Lf => content.to_string(),
        LineEndings::Crlf => normalize_line_endings(content).replace('\n', "\r\n"),
    }
}

/// Decode a file for editing: reject binary/non-UTF-8, return LF-normalized text plus original style.
pub(crate) async fn read_for_edit(
    io_path: &str,
    display_path: &str,
) -> Result<(String, LineEndings), FsError> {
    let bytes = match fs::read(io_path).await {
        Ok(bytes) => bytes,
        Err(err) if is_absent(&err) => {
            return Err(FsError::new(
                format!("cannot edit \"{display_path}\": not found"),
                FsErrorCode::NotFound,
            ));
        }
        Err(err) => return Err(io_fs_error("edit", display_path, err)),
    };
    if bytes.contains(&0) {
        return Err(not_text("edit", display_path, true));
    }
    let raw = std::str::from_utf8(&bytes).map_err(|_| not_text("edit", display_path, false))?;
    Ok((normalize_line_endings(raw), detect_line_endings(raw)))
}

/// Best-effort overwrite diff basis. I/O, binary, and UTF-8 failures yield `Ok(None)`.
pub(crate) async fn read_text_for_diff(
    io_path: &str,
    max_bytes: usize,
) -> Result<Option<String>, FsError> {
    let meta = match fs::metadata(io_path).await {
        Ok(meta) => meta,
        Err(_) => return Ok(None),
    };
    if !meta.is_file() || meta.size() >= max_bytes as u64 {
        return Ok(None);
    }
    let bytes = match fs::read(io_path).await {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    if bytes.len() >= max_bytes || bytes.contains(&0) {
        return Ok(None);
    }
    match std::str::from_utf8(&bytes) {
        Ok(raw) => Ok(Some(normalize_line_endings(raw))),
        Err(_) => Ok(None),
    }
}

fn count_occurrences(content: &str, needle: &str) -> usize {
    let mut count = 0;
    let mut index = 0;
    while let Some(found) = content[index..].find(needle) {
        count += 1;
        index += found + needle.len();
    }
    count
}

/// Literal replace on already LF-normalized `content`.
pub(crate) fn apply_literal_edit(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
    display_path: &str,
) -> Result<String, FsError> {
    let old_norm = normalize_line_endings(old_string);
    if old_norm.is_empty() {
        return Err(FsError::new(
            "old_string must be a non-empty string",
            FsErrorCode::EditNotFound,
        ));
    }
    let new_norm = normalize_line_endings(new_string);
    let replacements = count_occurrences(content, &old_norm);
    if replacements == 0 {
        return Err(FsError::new(
            format!("old_string was not found in \"{display_path}\""),
            FsErrorCode::EditNotFound,
        ));
    }
    if !replace_all && replacements > 1 {
        return Err(FsError::new(
            format!(
                "old_string matched {replacements} times in \"{display_path}\"; provide a more specific old_string or set replace_all to true"
            ),
            FsErrorCode::AmbiguousEdit,
        ));
    }
    Ok(content.replace(&old_norm, &new_norm))
}

/// Write `content` to a `0o600` sibling temp file and `rename` it onto `io_path`.
pub(crate) async fn write_file_atomic(
    io_path: &str,
    display_path: &str,
    content: &str,
    existing_mode: Option<u32>,
) -> Result<(), FsError> {
    let dest = Path::new(io_path);
    let parent = dest
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .await
        .map_err(|err| io_fs_error("write", display_path, err))?;

    let file_name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "file".into());

    let mut last_exists: Option<io::Error> = None;
    for _ in 0..8 {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(".{file_name}.tmp.{}.{seq}", std::process::id()));
        match publish_temp(&temp, dest, content, existing_mode).await {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                last_exists = Some(err);
            }
            Err(err) => {
                let _temp_residue = fs::remove_file(&temp).await;
                return Err(io_fs_error("write", display_path, err));
            }
        }
    }
    Err(io_fs_error(
        "write",
        display_path,
        last_exists.unwrap_or_else(|| io::Error::other("could not allocate a sibling temp file")),
    ))
}

async fn publish_temp(
    temp: &Path,
    dest: &Path,
    content: &str,
    existing_mode: Option<u32>,
) -> io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true).mode(0o600);
    let mut file = opts.open(temp).await?;
    if let Err(err) = file.write_all(content.as_bytes()).await {
        drop(file);
        let _temp_residue = fs::remove_file(temp).await;
        return Err(err);
    }
    if let Err(err) = file.sync_all().await {
        drop(file);
        let _temp_residue = fs::remove_file(temp).await;
        return Err(err);
    }
    drop(file);

    let mut perms = fs::metadata(temp).await?.permissions();
    perms.set_mode(existing_mode.unwrap_or(0o600));
    if let Err(err) = fs::set_permissions(temp, perms).await {
        let _temp_residue = fs::remove_file(temp).await;
        return Err(err);
    }

    match fs::rename(temp, dest).await {
        Ok(()) => Ok(()),
        Err(err) => {
            let _temp_residue = fs::remove_file(temp).await;
            Err(err)
        }
    }
}
