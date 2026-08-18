//! Host-filesystem UTF-8 backend, optionally fenced by [`SandboxFence`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dsh_sandbox::{SandboxExecutionPolicy, SandboxMode};
use dsh_tools::AbortFlag;
use tokio::sync::Mutex as AsyncMutex;

use crate::fsio::{
    DEFAULT_DIFF_BASIS_MAX_BYTES, apply_literal_edit, normalize_line_endings, posix_file_url,
    probe, read_for_edit, read_text_for_diff, read_whole_text, resolve_local_target,
    restore_line_endings, stream_whole_text, throw_if_aborted, write_file_atomic,
};
use crate::sandbox::{SandboxFence, checked_target};
use crate::{
    FsEditOutcome, FsEditRequest, FsError, FsErrorCode, FsInfo, FsInfoType, FsTarget, FsVersion,
    FsWriteIntent, FsWriteOutcome,
};

/// Host-filesystem UTF-8 backend.
///
/// `cwd` is a resolution default, not a containment root. Relative paths may
/// leave it; absolute paths ignore it. [`Self::sandbox_mode`] is [`None`] for
/// [`Self::new`] and `Some(default_policy.mode)` for [`Self::sandboxed`].
pub struct LocalFileSystem {
    /// Base directory for relative paths.
    pub cwd: PathBuf,
    /// Exclusive UTF-8 byte limit on each overwrite-diff side. Defaults to 10 MiB.
    pub diff_basis_max_bytes: usize,
    fence: Option<SandboxFence>,
    /// Per-`target_key` lock serializing the probe→guard→publish window.
    locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl LocalFileSystem {
    /// Build an unfenced backend that resolves relative paths against `cwd`.
    ///
    /// Mutations ignore `sandbox_policy`.
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            diff_basis_max_bytes: DEFAULT_DIFF_BASIS_MAX_BYTES,
            fence: None,
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Build a backend that fences `write_text` and `edit_text` with `default_policy`.
    ///
    /// Reads are never fenced. A per-call `sandbox_policy` replaces the default
    /// for that mutation.
    #[must_use]
    pub fn sandboxed(cwd: impl Into<PathBuf>, default_policy: SandboxExecutionPolicy) -> Self {
        Self {
            cwd: cwd.into(),
            diff_basis_max_bytes: DEFAULT_DIFF_BASIS_MAX_BYTES,
            fence: Some(SandboxFence { default_policy }),
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// File-effect mode of the installed fence, or [`None`] when unfenced.
    #[must_use]
    pub fn sandbox_mode(&self) -> Option<SandboxMode> {
        self.fence.as_ref().map(|fence| fence.default_policy.mode)
    }

    /// When a fence is installed, enforce `sandbox_policy` or the fence default.
    async fn enforce_fence(
        &self,
        target: &FsTarget,
        sandbox_policy: Option<&SandboxExecutionPolicy>,
    ) -> Result<FsTarget, FsError> {
        let Some(policy) = self.fence.as_ref().map(|fence| {
            sandbox_policy
                .cloned()
                .unwrap_or_else(|| fence.default_policy.clone())
        }) else {
            return Ok(target.clone());
        };
        checked_target(self, target, Some(&policy)).await
    }

    fn mutation_lock(&self, key: &str) -> Arc<AsyncMutex<()>> {
        let mut map = self
            .locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    /// Resolve `path` to a canonical target.
    ///
    /// Relative paths join against `cwd` when given, otherwise [`Self::cwd`].
    /// The deepest existing ancestor is realpathed and the missing suffix is
    /// appended. `target_key` and `display_path` are that canonical string.
    ///
    /// # Errors
    ///
    /// [`FsErrorCode::NotFound`] when `path` is empty or whitespace-only.
    /// [`FsErrorCode::Aborted`] when `signal` is aborted.
    pub async fn resolve(
        &self,
        path: &str,
        cwd: Option<&Path>,
        signal: Option<&AbortFlag>,
    ) -> Result<FsTarget, FsError> {
        throw_if_aborted(signal, "resolve")?;
        let target = resolve_local_target(cwd.unwrap_or(&self.cwd), path).await?;
        throw_if_aborted(signal, "resolve")?;
        Ok(target)
    }

    /// Canonical host path stored on `target.target_key`.
    #[must_use]
    pub fn process_path(&self, target: &FsTarget) -> String {
        target.target_key.as_str().to_string()
    }

    /// `file:` URI of [`Self::process_path`].
    ///
    /// POSIX form is `file:///abs/path` with percent-encoding matching Node
    /// `pathToFileURL` on unix. Does not inspect the filesystem.
    ///
    /// # Parameters
    ///
    /// * `target` - Resolved filesystem target.
    ///
    /// # Returns
    ///
    /// A `file:` URI for the canonical process path.
    #[must_use]
    pub fn file_url(&self, target: &FsTarget) -> String {
        posix_file_url(&self.process_path(target))
    }

    /// Whether `child`'s canonical key is `parent` or a lexical path under it.
    #[must_use]
    pub fn contains(&self, parent: &FsTarget, child: &FsTarget) -> bool {
        Path::new(child.target_key.as_str()).starts_with(parent.target_key.as_str())
    }

    /// Metadata for `target`, or [`None`] when it is absent.
    ///
    /// # Errors
    ///
    /// [`FsErrorCode::Aborted`] when `signal` is aborted. Other I/O failures
    /// besides absence map to [`FsErrorCode::PermissionDenied`] or
    /// [`FsErrorCode::IoError`].
    pub async fn stat(
        &self,
        target: &FsTarget,
        signal: Option<&AbortFlag>,
    ) -> Result<Option<FsInfo>, FsError> {
        throw_if_aborted(signal, "stat")?;
        let info = probe(target.target_key.as_str()).await?;
        throw_if_aborted(signal, "stat")?;
        Ok(info.map(|path_info| FsInfo {
            version: path_info.version,
            kind: path_info.kind,
            size: match path_info.kind {
                FsInfoType::File => Some(path_info.size),
                FsInfoType::Directory | FsInfoType::Other => None,
            },
        }))
    }

    /// Read a regular UTF-8 file, normalizing `\r\n` to `\n`.
    ///
    /// # Errors
    ///
    /// [`FsErrorCode::NotFound`] when missing. [`FsErrorCode::NotRegularFile`]
    /// when the target is not a regular file. [`FsErrorCode::NotText`] when the
    /// first 8192 bytes contain NUL or the contents are not UTF-8.
    /// [`FsErrorCode::Aborted`] when cancelled.
    pub async fn read_text(
        &self,
        target: &FsTarget,
        signal: Option<&AbortFlag>,
    ) -> Result<String, FsError> {
        throw_if_aborted(signal, "read")?;
        let text = read_whole_text(target.target_key.as_str(), &target.display_path).await?;
        throw_if_aborted(signal, "read")?;
        Ok(text)
    }

    /// Stream decoded UTF-8 chunks with no CRLF rewrite.
    ///
    /// Same regular-file, first-8192 NUL, and UTF-8 rules as TypeScript
    /// `streamWholeText`. Chunk boundaries carry no meaning. Does not emit
    /// `fs/observed`.
    ///
    /// # Parameters
    ///
    /// * `target` - Canonical file to stream.
    /// * `signal` - When aborted, fail with [`FsErrorCode::Aborted`].
    /// * `on_chunk` - Receives each decoded chunk in file order. The `&str` is
    ///   valid only for the duration of the call.
    ///
    /// # Returns
    ///
    /// `Ok(())` after every chunk has been delivered.
    ///
    /// # Errors
    ///
    /// [`FsErrorCode::NotFound`] when missing. [`FsErrorCode::NotRegularFile`]
    /// when the target is not a regular file. [`FsErrorCode::NotText`] when the
    /// first 8192 bytes contain NUL or the contents are not UTF-8.
    /// [`FsErrorCode::Aborted`] when cancelled. Errors returned from `on_chunk`
    /// propagate unchanged.
    pub async fn stream_text(
        &self,
        target: &FsTarget,
        signal: Option<&AbortFlag>,
        on_chunk: &mut dyn FnMut(&str) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        stream_whole_text(
            target.target_key.as_str(),
            &target.display_path,
            signal,
            on_chunk,
        )
        .await
    }

    /// Atomically create or replace a UTF-8 file.
    ///
    /// Omitting `expected` is unconditional create-or-overwrite. Publication
    /// writes a `0o600` sibling temp file and `rename`s it into place.
    /// When a fence is installed, `sandbox_policy` (or the fence default) is
    /// enforced before the mutation; [`Self::new`] ignores it.
    ///
    /// # Errors
    ///
    /// [`FsErrorCode::SandboxDenied`] when the fence refuses the path.
    /// [`FsErrorCode::StaleVersion`] when `ReplaceIfVersion` misses or
    /// mismatches. [`FsErrorCode::NotObserved`] when `CreateIfAbsent` finds an
    /// existing file. [`FsErrorCode::NotRegularFile`] when the path exists and
    /// is not a file. [`FsErrorCode::Aborted`] when cancelled.
    pub async fn write_text(
        &self,
        target: &FsTarget,
        content: &str,
        expected: Option<FsWriteIntent>,
        signal: Option<&AbortFlag>,
        sandbox_policy: Option<&SandboxExecutionPolicy>,
    ) -> Result<FsWriteOutcome, FsError> {
        let target = self.enforce_fence(target, sandbox_policy).await?;
        throw_if_aborted(signal, "write")?;
        let lock = self.mutation_lock(target.target_key.as_str());
        let _guard = lock.lock().await;
        throw_if_aborted(signal, "write")?;

        let existing = probe(target.target_key.as_str()).await?;
        if existing
            .as_ref()
            .is_some_and(|info| info.kind != FsInfoType::File)
        {
            return Err(FsError::new(
                format!(
                    "cannot write \"{}\": not a regular file",
                    target.display_path
                ),
                FsErrorCode::NotRegularFile,
            ));
        }

        match &expected {
            Some(FsWriteIntent::ReplaceIfVersion { version }) => match &existing {
                None => {
                    return Err(FsError::new(
                        format!(
                            "cannot write \"{}\": file no longer exists",
                            target.display_path
                        ),
                        FsErrorCode::StaleVersion,
                    ));
                }
                Some(info) if &info.version != version => {
                    return Err(FsError::new(
                        format!(
                            "cannot write \"{}\": file changed since it was read",
                            target.display_path
                        ),
                        FsErrorCode::StaleVersion,
                    ));
                }
                Some(_) => {}
            },
            Some(FsWriteIntent::CreateIfAbsent) => {
                if existing.is_some() {
                    return Err(FsError::new(
                        format!(
                            "cannot overwrite existing \"{}\" without reading it first",
                            target.display_path
                        ),
                        FsErrorCode::NotObserved,
                    ));
                }
            }
            None => {}
        }

        let diffable = existing.is_some() && content.len() < self.diff_basis_max_bytes;
        let before = if diffable {
            throw_if_aborted(signal, "read")?;
            read_text_for_diff(target.target_key.as_str(), self.diff_basis_max_bytes).await?
        } else {
            None
        };

        throw_if_aborted(signal, "write")?;
        write_file_atomic(
            target.target_key.as_str(),
            &target.display_path,
            content,
            existing.as_ref().map(|info| info.mode),
        )
        .await?;

        let version = match probe(target.target_key.as_str()).await? {
            Some(info) => info.version,
            None => FsVersion::new(format!("missing:{}", target.target_key.as_str())),
        };
        Ok(FsWriteOutcome {
            operation: if existing.is_some() {
                "update"
            } else {
                "create"
            },
            version,
            before,
            after: normalize_line_endings(content),
        })
    }

    /// Literal replacement of `edit.old_string` in a regular UTF-8 file.
    ///
    /// Matching runs on LF-normalized text. A CRLF original is restored on
    /// publish. When a fence is installed, `sandbox_policy` (or the fence
    /// default) is enforced before the mutation; [`Self::new`] ignores it.
    ///
    /// # Errors
    ///
    /// [`FsErrorCode::SandboxDenied`] when the fence refuses the path.
    /// [`FsErrorCode::NotFound`] when missing. [`FsErrorCode::StaleVersion`]
    /// when `expected` does not match. [`FsErrorCode::EditNotFound`] when
    /// `old_string` is absent. [`FsErrorCode::AmbiguousEdit`] when several
    /// matches exist and `replace_all` is false. [`FsErrorCode::NotRegularFile`]
    /// when not a file. [`FsErrorCode::Aborted`] when cancelled.
    pub async fn edit_text(
        &self,
        target: &FsTarget,
        edit: &FsEditRequest,
        expected: Option<&FsVersion>,
        signal: Option<&AbortFlag>,
        sandbox_policy: Option<&SandboxExecutionPolicy>,
    ) -> Result<FsEditOutcome, FsError> {
        let target = self.enforce_fence(target, sandbox_policy).await?;
        throw_if_aborted(signal, "edit")?;
        let lock = self.mutation_lock(target.target_key.as_str());
        let _guard = lock.lock().await;
        throw_if_aborted(signal, "edit")?;

        let existing = probe(target.target_key.as_str()).await?;
        let Some(existing) = existing else {
            return Err(FsError::new(
                format!("cannot edit \"{}\": not found", target.display_path),
                FsErrorCode::NotFound,
            ));
        };
        if existing.kind != FsInfoType::File {
            return Err(FsError::new(
                format!(
                    "cannot edit \"{}\": not a regular file",
                    target.display_path
                ),
                FsErrorCode::NotRegularFile,
            ));
        }
        if expected.is_some_and(|version| existing.version != *version) {
            return Err(FsError::new(
                format!(
                    "cannot edit \"{}\": file changed since it was read",
                    target.display_path
                ),
                FsErrorCode::StaleVersion,
            ));
        }

        throw_if_aborted(signal, "edit")?;
        let (before, line_endings) =
            read_for_edit(target.target_key.as_str(), &target.display_path).await?;
        let after = apply_literal_edit(
            &before,
            &edit.old_string,
            &edit.new_string,
            edit.replace_all,
            &target.display_path,
        )?;
        let published = restore_line_endings(&after, line_endings);
        throw_if_aborted(signal, "edit")?;
        write_file_atomic(
            target.target_key.as_str(),
            &target.display_path,
            &published,
            Some(existing.mode),
        )
        .await?;
        let version = match probe(target.target_key.as_str()).await? {
            Some(info) => info.version,
            None => FsVersion::new(format!("missing:{}", target.target_key.as_str())),
        };
        Ok(FsEditOutcome {
            version,
            before,
            after,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::LocalFileSystem;
    use crate::{FsErrorCode, FsWriteIntent};
    use std::fs;

    fn setup() -> (LocalFileSystem, std::path::PathBuf) {
        let dir = {
            let d = std::env::temp_dir().join(format!(
                "dsh-fs-local-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&d).unwrap();
            d
        };
        (LocalFileSystem::new(&dir), dir)
    }

    #[tokio::test]
    async fn write_then_read_round_trip() {
        let (fs, dir) = setup();
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        fs.write_text(&target, "hello\n", None, None, None)
            .await
            .unwrap();
        let text = fs.read_text(&target, None).await.unwrap();
        assert_eq!(text, "hello\n");
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "hello\n");
        assert!(fs.sandbox_mode().is_none());
    }

    #[tokio::test]
    async fn create_if_absent_on_existing_file_is_not_observed() {
        let (fs, _) = setup();
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        fs.write_text(&target, "x", None, None, None).await.unwrap();
        let err = fs
            .write_text(
                &target,
                "y",
                Some(FsWriteIntent::CreateIfAbsent),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotObserved);
        assert!(err.message.contains("without reading it first"));
    }

    #[tokio::test]
    async fn replace_if_version_detects_stale() {
        let (fs, _) = setup();
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        let first = fs
            .write_text(&target, "one", None, None, None)
            .await
            .unwrap();
        fs.write_text(&target, "two", None, None, None)
            .await
            .unwrap();
        let err = fs
            .write_text(
                &target,
                "three",
                Some(FsWriteIntent::ReplaceIfVersion {
                    version: first.version,
                }),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::StaleVersion);
    }

    #[tokio::test]
    async fn edit_requires_unique_match_unless_replace_all() {
        let (fs, _) = setup();
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        fs.write_text(&target, "aa", None, None, None)
            .await
            .unwrap();
        let err = fs
            .edit_text(
                &target,
                &crate::FsEditRequest {
                    old_string: "a".into(),
                    new_string: "b".into(),
                    replace_all: false,
                },
                None,
                None,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::AmbiguousEdit);
        let out = fs
            .edit_text(
                &target,
                &crate::FsEditRequest {
                    old_string: "a".into(),
                    new_string: "b".into(),
                    replace_all: true,
                },
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(out.after, "bb");
    }

    #[tokio::test]
    async fn empty_path_is_not_found() {
        let (fs, _) = setup();
        let err = fs.resolve("", None, None).await.unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotFound);
        assert_eq!(err.message, "file_path must be a non-empty string");
        let err = fs.resolve("   ", None, None).await.unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotFound);
    }

    #[tokio::test]
    async fn aborted_resolve_reports_aborted() {
        let (fs, _) = setup();
        let flag = dsh_tools::AbortFlag::new();
        flag.abort();
        let err = fs.resolve("a.txt", None, Some(&flag)).await.unwrap_err();
        assert_eq!(err.code, FsErrorCode::Aborted);
        assert!(err.message.contains("aborted"));
    }

    #[tokio::test]
    async fn read_directory_is_not_regular_file() {
        let (fs, _) = setup();
        let target = fs.resolve(".", None, None).await.unwrap();
        let err = fs.read_text(&target, None).await.unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotRegularFile);
    }

    #[tokio::test]
    async fn edit_missing_file_is_not_found() {
        let (fs, _) = setup();
        let target = fs.resolve("missing.txt", None, None).await.unwrap();
        let err = fs
            .edit_text(
                &target,
                &crate::FsEditRequest {
                    old_string: "a".into(),
                    new_string: "b".into(),
                    replace_all: false,
                },
                None,
                None,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotFound);
    }

    #[tokio::test]
    async fn contains_is_lexical_prefix_of_canonical_keys() {
        let (fs, _) = setup();
        let parent = fs.resolve("dir", None, None).await.unwrap();
        let child = fs.resolve("dir/a.txt", None, None).await.unwrap();
        let sibling = fs.resolve("dir2/a.txt", None, None).await.unwrap();
        assert!(fs.contains(&parent, &child));
        assert!(fs.contains(&parent, &parent));
        assert!(!fs.contains(&parent, &sibling));
        assert_eq!(fs.process_path(&child), child.target_key.as_str());
    }

    #[tokio::test]
    async fn edit_restores_crlf_and_read_normalizes() {
        let (fs, dir) = setup();
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        fs.write_text(&target, "one\r\ntwo\r\n", None, None, None)
            .await
            .unwrap();
        assert_eq!(fs.read_text(&target, None).await.unwrap(), "one\ntwo\n");
        let out = fs
            .edit_text(
                &target,
                &crate::FsEditRequest {
                    old_string: "one".into(),
                    new_string: "uno".into(),
                    replace_all: false,
                },
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(out.after, "uno\ntwo\n");
        assert_eq!(
            fs::read_to_string(dir.join("a.txt")).unwrap(),
            "uno\r\ntwo\r\n"
        );
    }

    #[tokio::test]
    async fn replace_if_version_missing_is_stale() {
        let (fs, _) = setup();
        let target = fs.resolve("gone.txt", None, None).await.unwrap();
        let err = fs
            .write_text(
                &target,
                "x",
                Some(FsWriteIntent::ReplaceIfVersion {
                    version: crate::FsVersion::new("0:0:0:0:0"),
                }),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::StaleVersion);
        assert!(err.message.contains("file no longer exists"));
    }

    #[tokio::test]
    async fn nul_in_first_8192_bytes_is_not_text() {
        let (fs, dir) = setup();
        std::fs::write(dir.join("bin.txt"), b"ok\0no").unwrap();
        let target = fs.resolve("bin.txt", None, None).await.unwrap();
        let err = fs.read_text(&target, None).await.unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotText);
    }

    #[tokio::test]
    async fn stream_text_preserves_crlf_when_read_text_normalizes() {
        let (fs, dir) = setup();
        std::fs::write(dir.join("a.txt"), b"a\r\nb").unwrap();
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        assert_eq!(fs.read_text(&target, None).await.unwrap(), "a\nb");
        let mut collected = String::new();
        fs.stream_text(&target, None, &mut |chunk| {
            collected.push_str(chunk);
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(collected, "a\r\nb");
    }

    #[tokio::test]
    async fn stream_text_rejects_nul() {
        let (fs, dir) = setup();
        std::fs::write(dir.join("bin.txt"), b"ok\0no").unwrap();
        let target = fs.resolve("bin.txt", None, None).await.unwrap();
        let err = fs
            .stream_text(&target, None, &mut |_| Ok(()))
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotText);
    }

    #[tokio::test]
    async fn file_url_is_file_scheme_of_process_path() {
        let (fs, _) = setup();
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        let process = fs.process_path(&target);
        let url = fs.file_url(&target);
        assert!(process.starts_with('/'), "{process}");
        assert_eq!(url, format!("file://{process}"));

        let special = crate::FsTarget {
            target_key: crate::FsTargetKey::new("/tmp/foo bar#[].txt~"),
            display_path: "/tmp/foo bar#[].txt~".into(),
        };
        assert_eq!(
            fs.file_url(&special),
            "file:///tmp/foo%20bar%23%5B%5D.txt%7E"
        );
        let cafe = crate::FsTarget {
            target_key: crate::FsTargetKey::new("/tmp/café"),
            display_path: "/tmp/café".into(),
        };
        assert_eq!(fs.file_url(&cafe), "file:///tmp/caf%C3%A9");
    }
}
