use std::fs;
use std::process;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{CACHE_SCHEMA_VERSION, cache_file};
use crate::model::Session;

#[derive(Debug, Serialize, Deserialize)]
struct SessionCache {
    schema_version: u32,
    sessions: Vec<Session>,
}

pub fn load_sessions() -> Result<Vec<Session>> {
    let path = cache_file();
    let data = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut cache: SessionCache = serde_json::from_slice(&data)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if cache.schema_version != CACHE_SCHEMA_VERSION {
        return Ok(Vec::new());
    }
    cache.sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(cache.sessions)
}

pub fn save_sessions(sessions: &[Session]) -> Result<()> {
    let path = cache_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let cache = SessionCache {
        schema_version: CACHE_SCHEMA_VERSION,
        sessions: sessions.to_vec(),
    };
    let data = serde_json::to_vec(&cache).context("failed to serialize session cache")?;
    let tmp_path = path.with_extension(format!("json.tmp.{}", process::id()));
    fs::write(&tmp_path, data)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path).with_context(|| format!("failed to replace {}", path.display()))
}
