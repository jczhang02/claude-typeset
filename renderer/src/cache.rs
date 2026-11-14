//! TeX pictures kept on disk, in $XDG_CACHE_HOME/typeset/tex (else
//! ~/.cache/typeset/tex): a TeX run takes a few hundred milliseconds, so a
//! formula that falls back to TeX costs that once. RaTeX's pictures take a
//! few milliseconds and are not kept.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::protocol::ItemResult;

const MAX_ENTRIES: usize = 2000;
const MAX_BYTES: u64 = 200 * 1024 * 1024;
const STALE_TMP: Duration = Duration::from_secs(3600);

/// $XDG_CACHE_HOME/typeset, else ~/.cache/typeset; None without a home.
pub fn base() -> Option<PathBuf> {
    base_from(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}

/// An empty or relative XDG_CACHE_HOME is ignored, as the XDG spec says
/// (else the cache would land in the session's working folder).
fn base_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let absolute = |dir: &OsString| Path::new(dir).is_absolute();
    let cache = xdg
        .filter(absolute)
        .map(PathBuf::from)
        .or_else(|| home.filter(absolute).map(|home| PathBuf::from(home).join(".cache")))?;
    Some(cache.join("typeset"))
}

/// The folder, or None without a home (then nothing is kept).
pub fn folder() -> Option<PathBuf> {
    Some(base()?.join("tex"))
}

/// Makes `dir` (under base()) readable by the user alone, with every
/// folder from it up to base(): the cache holds pictures of the user's
/// conversations. A folder made by an earlier version (0755) is tightened
/// too. False when `dir` cannot be made.
pub fn private_dir(dir: &Path) -> bool {
    match base() {
        Some(top) => private_dir_under(dir, &top),
        None => false,
    }
}

fn private_dir_under(dir: &Path, top: &Path) -> bool {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .is_err()
    {
        return false;
    }
    for folder in dir.ancestors().take_while(|folder| folder.starts_with(top)) {
        if let Ok(meta) = std::fs::metadata(folder)
            && meta.permissions().mode() & 0o077 != 0
        {
            let _ = std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700));
        }
    }
    true
}

/// Writes a file only the user may read.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)
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
    if !private_dir(&dir) {
        return false;
    }
    let tmp = dir.join(format!(
        ".{key}.{}.{:?}.tmp",
        std::process::id(),
        std::thread::current().id()
    ));
    if write_private(&tmp, text.as_bytes()).is_err() {
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

#[cfg(test)]
mod tests {
    use super::base_from;
    use std::path::Path;

    #[test]
    fn the_cache_is_the_users_alone() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let outer = std::env::temp_dir().join(format!("typeset-cache-test-{}", std::process::id()));
        let top = outer.join("typeset");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(&top)
            .unwrap();
        let dir = top.join("tex");
        assert!(super::private_dir_under(&dir, &top));
        super::write_private(&dir.join("f.json"), b"{}").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&top), 0o700, "a folder an earlier version made is tightened");
        assert_eq!(mode(&outer), 0o755, "nothing above the cache is touched");
        assert_eq!(mode(&dir.join("f.json")), 0o600);
        std::fs::remove_dir_all(&outer).unwrap();
    }

    #[test]
    fn an_empty_or_relative_cache_home_is_ignored() {
        let home = Some("/home/someone".into());
        let at_home = Path::new("/home/someone/.cache/typeset");
        assert_eq!(base_from(Some("".into()), home.clone()).unwrap(), at_home);
        assert_eq!(base_from(Some("cache".into()), home.clone()).unwrap(), at_home);
        assert_eq!(base_from(None, home.clone()).unwrap(), at_home);
        assert_eq!(
            base_from(Some("/tmp/xdg".into()), home).unwrap(),
            Path::new("/tmp/xdg/typeset")
        );
        assert_eq!(base_from(Some("".into()), Some("".into())), None);
    }
}
