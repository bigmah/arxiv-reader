//! A dumb two-tier cache: a process-local map in front of files on disk.
//!
//! Every LLM call costs money, and PDFs are megabytes, so both are written to
//! `CACHE_DIR` and survive restarts. Delete the directory to force regeneration.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::sync::Mutex;

pub struct Cache {
    root: PathBuf,
    memory: Mutex<HashMap<PathBuf, String>>,
}

impl Cache {
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        tokio::fs::create_dir_all(&root)
            .await
            .with_context(|| format!("creating cache dir {}", root.display()))?;
        Ok(Self { root, memory: Mutex::new(HashMap::new()) })
    }

    fn path(&self, namespace: &str, key: &str, ext: &str) -> PathBuf {
        self.root.join(namespace).join(format!("{}.{ext}", sanitize(key)))
    }

    /// Return the cached text for `key`, or compute it with `build`, store it, and return that.
    ///
    /// Two concurrent misses on the same key may both call `build`; the loser's
    /// result is simply overwritten, which costs one extra API call at worst.
    pub async fn text_or_build<F, Fut>(&self, namespace: &str, key: &str, build: F) -> Result<String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String>>,
    {
        let path = self.path(namespace, key, "txt");

        if let Some(hit) = self.memory.lock().await.get(&path) {
            return Ok(hit.clone());
        }
        if let Ok(text) = tokio::fs::read_to_string(&path).await {
            self.memory.lock().await.insert(path, text.clone());
            return Ok(text);
        }

        let value = build().await?;
        write_atomic(&path, value.as_bytes()).await?;
        self.memory.lock().await.insert(path, value.clone());
        Ok(value)
    }

    /// Same contract as [`Cache::text_or_build`], for binary blobs. These are not
    /// held in memory — PDFs are far too big for that to be a good idea.
    pub async fn bytes_or_build<F, Fut>(
        &self,
        namespace: &str,
        key: &str,
        ext: &str,
        build: F,
    ) -> Result<Vec<u8>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>>>,
    {
        let path = self.path(namespace, key, ext);
        if let Ok(bytes) = tokio::fs::read(&path).await {
            return Ok(bytes);
        }

        let value = build().await?;
        write_atomic(&path, &value).await?;
        Ok(value)
    }
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Write beside the target then rename, so a crash mid-write can't leave a
    // truncated file that later reads would happily treat as a cache hit.
    let temp = path.with_extension("partial");
    tokio::fs::write(&temp, bytes)
        .await
        .with_context(|| format!("writing cache file {}", temp.display()))?;
    tokio::fs::rename(&temp, path)
        .await
        .with_context(|| format!("renaming cache file into {}", path.display()))?;
    Ok(())
}

/// arXiv ids can contain `/` (`math/0309136`), so keys never touch the filesystem raw.
fn sanitize(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_path_separators_and_traversal() {
        assert_eq!(sanitize("math/0309136"), "math_0309136");
        assert_eq!(sanitize("2608.14539v1"), "2608.14539v1");
        assert_eq!(sanitize("../../etc/passwd"), ".._.._etc_passwd");
    }

    #[tokio::test]
    async fn builds_once_then_serves_from_cache() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = std::env::temp_dir().join(format!("arxiv-cache-test-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let cache = Cache::new(&dir).await.unwrap();

        let builds = AtomicUsize::new(0);
        let build = || async {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok("hello".to_string())
        };

        assert_eq!(cache.text_or_build("brief", "cs/1", build).await.unwrap(), "hello");
        assert_eq!(cache.text_or_build("brief", "cs/1", build).await.unwrap(), "hello");
        assert_eq!(builds.load(Ordering::SeqCst), 1, "second call should hit the memory tier");

        // A fresh Cache (cold memory tier) still hits the file on disk.
        let reopened = Cache::new(&dir).await.unwrap();
        assert_eq!(reopened.text_or_build("brief", "cs/1", build).await.unwrap(), "hello");
        assert_eq!(builds.load(Ordering::SeqCst), 1, "third call should hit the disk tier");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
