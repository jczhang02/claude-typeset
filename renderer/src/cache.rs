//! TeX pictures kept on disk, in $XDG_CACHE_HOME/typeset/tex (else
//! ~/.cache/typeset/tex): a TeX run takes a few hundred milliseconds, so a
//! formula that falls back to TeX costs that once. RaTeX's pictures take a
//! few milliseconds and are not kept.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::protocol::ItemResult;

const MAX_ENTRIES: usize = 2000;
const MAX_BYTES: u64 = 200 * 1024 * 1024;
const STALE_TMP: Duration = Duration::from_secs(3600);

/// The folder, or None without a home (then nothing is kept).
pub fn folder() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("typeset").join("tex"))
}

/// The key of everything a picture depends on, as one JSON value.
pub fn key(parts: &serde_json::Value) -> String {
    Sha256::digest(parts.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn get(key: &str) -> Option<ItemResult> {
    let path = folder()?.join(format!("{key}.json"));
    let text = std::fs::read_to_string(&path).ok()?;
    let found: ItemResult = serde_json::from_str(&text).ok()?;
    // Recently used: pruned last.
    let _ = std::fs::File::options()
        .append(true)
        .open(&path)
        .and_then(|f| f.set_modified(SystemTime::now()));
    Some(found)
}

/// Writes atomically (a temporary file, then a rename); false when it could not.
pub fn put(key: &str, value: &ItemResult) -> bool {
    let Some(dir) = folder() else {
        return false;
    };
    let Ok(text) = serde_json::to_string(value) else {
        return false;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let tmp = dir.join(format!(
        ".{key}.{}.{:?}.tmp",
        std::process::id(),
        std::thread::current().id()
    ));
    if std::fs::write(&tmp, text).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    std::fs::rename(&tmp, dir.join(format!("{key}.json"))).is_ok()
}

/// Deletes the least recently used entries beyond the bounds, down to 80 % of them.
pub fn prune() {
    let Some(dir) = folder() else {
        return;
    };
    let Ok(listing) = std::fs::read_dir(&dir) else {
        return;
    };
    let now = SystemTime::now();
    let mut entries = Vec::new();
    for entry in listing.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if name.ends_with(".json") {
            entries.push((modified, meta.len(), entry.path()));
        } else if name.ends_with(".tmp") && now.duration_since(modified).is_ok_and(|age| age > STALE_TMP) {
            let _ = std::fs::remove_file(entry.path()); // left by a killed run
        }
    }
    let mut total: u64 = entries.iter().map(|(_, size, _)| size).sum();
    let mut count = entries.len();
    if count <= MAX_ENTRIES && total <= MAX_BYTES {
        return;
    }
    entries.sort();
    let (keep_entries, keep_bytes) = (MAX_ENTRIES * 4 / 5, MAX_BYTES * 4 / 5);
    for (_, size, path) in entries {
        if count <= keep_entries && total <= keep_bytes {
            break;
        }
        let _ = std::fs::remove_file(path);
        count -= 1;
        total -= size;
    }
}
