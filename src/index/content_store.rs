use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::document::session_key;

/// Sidecar store for conversation transcripts. The Tantivy documents keep
/// only metadata (plus the indexed content tokens), so search hits no longer
/// decompress full conversations; the transcript for a session is read from
/// here only when it is actually previewed.
pub(super) fn store_dir(index_path: &Path) -> PathBuf {
    let name = index_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("index");
    index_path.with_file_name(format!("{name}.content"))
}

fn content_path(index_path: &Path, agent: &str, id: &str) -> PathBuf {
    let key = session_key(agent, id);
    let mut hex = String::with_capacity(key.len() * 2);
    for byte in key.bytes() {
        let _ = write!(hex, "{byte:02x}");
    }
    store_dir(index_path).join(hex)
}

pub(super) fn write_content(index_path: &Path, agent: &str, id: &str, content: &str) -> Result<()> {
    let path = content_path(index_path, agent, id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("failed to store {}", path.display()))?;
    Ok(())
}

pub(super) fn read_content(index_path: &Path, agent: &str, id: &str) -> Result<String> {
    let path = content_path(index_path, agent, id);
    fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))
}

/// Best-effort cleanup after a committed delete; an orphaned content file is
/// harmless and is overwritten or ignored on later refreshes.
pub(super) fn delete_content(index_path: &Path, agent: &str, id: &str) {
    let _ = fs::remove_file(content_path(index_path, agent, id));
}

pub(super) fn remove_store(index_path: &Path) -> Result<()> {
    let dir = store_dir(index_path);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to clear {}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn write_read_delete_roundtrip() {
        let temp = tempdir().unwrap();
        let index_path = temp.path().join("tantivy_index");
        write_content(&index_path, "claude", "id/with:chars", "hello").unwrap();
        assert_eq!(
            read_content(&index_path, "claude", "id/with:chars").unwrap(),
            "hello"
        );
        delete_content(&index_path, "claude", "id/with:chars");
        assert!(read_content(&index_path, "claude", "id/with:chars").is_err());
        delete_content(&index_path, "claude", "id/with:chars");
        remove_store(&index_path).unwrap();
        remove_store(&index_path).unwrap();
    }
}
