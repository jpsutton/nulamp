//! Test helpers.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A unique folder under the system temp directory, removed on drop.
pub struct TempDir {
    root: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "nulamp-playlist-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    /// Whether the folder's file system tells names apart by case (most Linux file systems do;
    /// macOS and Windows defaults do not).
    pub fn case_sensitive(&self) -> bool {
        std::fs::write(self.root.join("CaseProbe"), b"").unwrap();
        !self.root.join("caseprobe").exists()
    }

    /// Absolute path of `rel` (with `/` separators) inside the folder.
    pub fn path(&self, rel: &str) -> String {
        self.root
            .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
            .to_string_lossy()
            .into_owned()
    }

    /// Writes `bytes` to `rel`, creating parent folders. Returns the absolute path.
    pub fn write(&self, rel: &str, bytes: &[u8]) -> String {
        let p = self
            .root
            .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
