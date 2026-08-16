//! Path containment for the in-process filesystem fence.

use std::path::{MAIN_SEPARATOR, Path};

/// Whether `path` is `root` or a descendant of it.
///
/// Linux uses a case-sensitive lexical prefix with [`MAIN_SEPARATOR`]. `/ws`
/// contains `/ws` and `/ws/a`; `/ws-other` is not under `/ws`. When spellings
/// differ, existing ancestors are compared by `(dev, ino)`. A missing `root`
/// returns `false`. Other metadata failures also return `false`.
#[must_use]
pub async fn is_path_under(path: &str, root: &str) -> bool {
    if is_lexically_under(path, root) {
        return true;
    }
    ancestor_identity_matches(path, root).await
}

fn is_lexically_under(path: &str, root: &str) -> bool {
    if path == root {
        return true;
    }
    path.starts_with(root) && {
        let rest = &path[root.len()..];
        rest.starts_with(MAIN_SEPARATOR) || root.ends_with(MAIN_SEPARATOR)
    }
}

async fn ancestor_identity_matches(path: &str, root: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(root_meta) = tokio::fs::metadata(root).await else {
        return false;
    };
    let mut ancestor = Path::new(path).to_path_buf();
    loop {
        if let Ok(meta) = tokio::fs::metadata(&ancestor).await {
            if meta.dev() == root_meta.dev() && meta.ino() == root_meta.ino() {
                return true;
            }
        }
        match ancestor.parent() {
            Some(parent) if parent != ancestor.as_path() && !parent.as_os_str().is_empty() => {
                ancestor = parent.to_path_buf();
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_path_under;

    #[tokio::test]
    async fn lexical_prefix_uses_separator() {
        assert!(is_path_under("/ws/a", "/ws").await);
        assert!(!is_path_under("/ws-other", "/ws").await);
        assert!(is_path_under("/ws", "/ws").await);
    }

    #[tokio::test]
    async fn missing_root_is_false() {
        assert!(!is_path_under("/tmp/file.txt", "/definitely-missing-dsh-fs-root").await);
    }

    #[tokio::test]
    async fn alias_root_matches_by_dev_ino() {
        let base = {
            let d = std::env::temp_dir().join(format!(
                "dsh-fs-containment-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&d).unwrap();
            d
        };
        let real = base.join("real");
        let alias = base.join("alias");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let path = format!(
            "{}/missing/file.txt",
            std::fs::canonicalize(&real).unwrap().display()
        );
        assert!(is_path_under(&path, alias.to_str().unwrap()).await);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn unrelated_existing_root_is_false() {
        let base = std::env::temp_dir().join(format!(
            "dsh-fs-containment-unrelated-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let allowed = base.join("allowed");
        let outside = base.join("outside");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let path = outside.join("file.txt");
        assert!(!is_path_under(path.to_str().unwrap(), allowed.to_str().unwrap()).await);
        let _ = std::fs::remove_dir_all(&base);
    }
}
