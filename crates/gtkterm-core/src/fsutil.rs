//! Durable file writes.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// Replace `path` with `contents` atomically and durably: write a unique temporary file next
/// to it, `fsync` it, rename it over `path` and `fsync` the directory. A crash leaves either
/// the old or the new file, never a truncated one, and two processes saving at once cannot
/// mix their temporary files. `mode` sets the permissions of the new file (default 0644).
pub fn write_atomic(path: &Path, contents: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode.unwrap_or(0o644))
            .open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        // Make the rename itself durable.
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn replaces_atomically_with_mode() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/config.toml");
        write_atomic(&p, b"one", None).unwrap();
        write_atomic(&p, b"two", Some(0o600)).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        let leftovers: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no temporary files left behind");
    }
}
