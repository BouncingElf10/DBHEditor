//! Locating, enumerating and backing up Detroit: Become Human save files.

use std::path::{Path, PathBuf};

/// A checkpoint save file the user can edit.
#[derive(Clone, Debug)]
pub struct SaveEntry {
    /// Full path to the `.qdsav` file.
    pub path: PathBuf,
    /// Chapter folder name, e.g. "02A Crossroads".
    pub chapter: String,
    /// Checkpoint file name without extension, e.g. "02A SECU START CONNOR".
    pub checkpoint: String,
}

impl SaveEntry {
    /// A one-line label for UI lists.
    pub fn label(&self) -> String {
        if self.chapter.is_empty() {
            self.checkpoint.clone()
        } else {
            format!("{} — {}", self.chapter, self.checkpoint)
        }
    }
}

/// The base save directory: `%USERPROFILE%\Saved Games\Quantic Dream\Detroit Become Human`.
pub fn default_save_root() -> Option<PathBuf> {
    let profile = std::env::var_os("USERPROFILE")?;
    let root = Path::new(&profile)
        .join("Saved Games")
        .join("Quantic Dream")
        .join("Detroit Become Human");
    root.is_dir().then_some(root)
}

/// Recursively collect every checkpoint `.qdsav` under `root`.
pub fn find_checkpoints(root: &Path) -> Vec<SaveEntry> {
    let mut out = Vec::new();
    collect(root, &mut out);
    out.sort_by_key(|e| e.label().to_lowercase());
    out
}

fn collect(dir: &Path, out: &mut Vec<SaveEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Never descend into our own backups folder.
            if path.file_name().map(|n| n == BACKUPS_DIRNAME).unwrap_or(false) {
                continue;
            }
            collect(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("qdsav") {
            let in_checkpoints = path
                .components()
                .any(|c| c.as_os_str().to_string_lossy() == "Checkpoints");
            if !in_checkpoints {
                continue;
            }
            let checkpoint = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let chapter = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|c| c != "Checkpoints")
                .unwrap_or_default();
            out.push(SaveEntry { path, chapter, checkpoint });
        }
    }
}

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Per-file backups (written automatically before each save)
// ---------------------------------------------------------------------------

/// Create a timestamped backup next to the file: `<name>.qdsav.bak-<epoch>`.
pub fn backup(path: &Path) -> std::io::Result<PathBuf> {
    let mut bak = path.as_os_str().to_owned();
    bak.push(format!(".bak-{}", epoch_secs()));
    let bak = PathBuf::from(bak);
    std::fs::copy(path, &bak)?;
    Ok(bak)
}

/// Newest `<file>.bak-*` sibling backup, if any.
pub fn latest_file_backup(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    let stem = path.file_name()?.to_string_lossy().into_owned();
    let prefix = format!("{stem}.bak-");
    let mut best: Option<(u64, PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(ts) = name.strip_prefix(&prefix).and_then(|s| s.parse::<u64>().ok()) {
            if best.as_ref().map(|(b, _)| ts > *b).unwrap_or(true) {
                best = Some((ts, e.path()));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Restore the most recent per-file backup over the original ("undo last save").
pub fn restore_latest_file_backup(path: &Path) -> std::io::Result<()> {
    let bak = latest_file_backup(path)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no backup for this file"))?;
    std::fs::copy(&bak, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Full backups of the entire save tree
// ---------------------------------------------------------------------------

const BACKUPS_DIRNAME: &str = "DBHEditor Backups";

/// Directory that holds full backups. Kept as a SIBLING of the save root so it
/// is never scanned as game saves. Falls back to inside the root if there is no
/// parent.
pub fn backups_dir(root: &Path) -> PathBuf {
    root.parent().unwrap_or(root).join(BACKUPS_DIRNAME)
}

/// A named full backup on disk.
#[derive(Clone, Debug)]
pub struct FullBackup {
    pub path: PathBuf,
    pub name: String,
    pub when_epoch: u64,
    pub file_count: usize,
}

impl FullBackup {
    /// e.g. "2026-07-13 10:54 · 30 files" (date is best-effort UTC-ish).
    pub fn label(&self) -> String {
        format!("{}  ·  {} files", human_time(self.when_epoch), self.file_count)
    }
}

/// Copy every file under `root` into `backups_dir/<timestamp>[ note]/`,
/// preserving the relative layout. Returns the created backup folder.
pub fn create_full_backup(root: &Path, note: Option<&str>) -> std::io::Result<PathBuf> {
    let ts = epoch_secs();
    let mut folder_name = ts.to_string();
    if let Some(n) = note.map(str::trim).filter(|n| !n.is_empty()) {
        folder_name.push(' ');
        folder_name.push_str(&sanitize(n));
    }
    let dest = backups_dir(root).join(&folder_name);
    std::fs::create_dir_all(&dest)?;
    copy_tree(root, root, &dest)?;
    Ok(dest)
}

/// List existing full backups, newest first.
pub fn list_full_backups(root: &Path) -> Vec<FullBackup> {
    let dir = backups_dir(root);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else { return out };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let when_epoch = name
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let file_count = count_files(&p);
        out.push(FullBackup { path: p, name, when_epoch, file_count });
    }
    out.sort_by(|a, b| b.when_epoch.cmp(&a.when_epoch));
    out
}

/// Restore a full backup back over the save root (overwrites matching files).
/// A safety full backup of the current state is taken first and returned.
pub fn restore_full_backup(root: &Path, backup: &Path) -> std::io::Result<PathBuf> {
    let safety = create_full_backup(root, Some("before-restore"))?;
    copy_tree(backup, backup, root)?;
    Ok(safety)
}

/// Recursively copy files from `src_root` into `dst_root`, skipping the backups
/// folder and previous per-file `.bak-*` copies.
fn copy_tree(base: &Path, src: &Path, dst_root: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)?.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name == BACKUPS_DIRNAME {
                continue;
            }
            copy_tree(base, &path, dst_root)?;
        } else {
            if name.to_string_lossy().contains(".bak-") {
                continue;
            }
            let rel = path.strip_prefix(base).unwrap_or(&path);
            let target = dst_root.join(rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn count_files(dir: &Path) -> usize {
    let mut n = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                n += count_files(&p);
            } else {
                n += 1;
            }
        }
    }
    n
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect()
}

/// Best-effort human-readable UTC time from a unix epoch (no chrono dependency).
fn human_time(epoch: u64) -> String {
    if epoch == 0 {
        return "unknown time".into();
    }
    let days = epoch / 86_400;
    let secs = epoch % 86_400;
    let (h, mi) = (secs / 3600, (secs % 3600) / 60);
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02} UTC")
}

/// Howard Hinnant's days-from-civil, inverted.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Open a folder (or the folder containing a file) in the OS file browser.
pub fn reveal(path: &Path) {
    let target = if path.is_dir() { path.to_path_buf() } else { path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.to_path_buf()) };
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(target).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(target).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dbheditor-test-{}-{}", tag, epoch_secs()));
        // Emulate the save layout: <root>/.../Checkpoints/<chapter>/<file>.qdsav
        let cp = dir.join("R/INTER_3/Game/Checkpoints/02A Crossroads");
        std::fs::create_dir_all(&cp).unwrap();
        std::fs::write(cp.join("02A SECU START NORTH.qdsav"), b"ORIGINAL").unwrap();
        dir
    }

    #[test]
    fn full_backup_and_restore_roundtrip() {
        let root = temp_root("fullbak");
        let file = root.join("R/INTER_3/Game/Checkpoints/02A Crossroads/02A SECU START NORTH.qdsav");

        let backup = create_full_backup(&root, Some("test note")).unwrap();
        assert!(backup.exists());
        assert_eq!(list_full_backups(&root).len(), 1);

        // Mutate the live save, then restore.
        std::fs::write(&file, b"EDITED").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"EDITED");
        restore_full_backup(&root, &backup).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"ORIGINAL");

        // Restore also snapshots the pre-restore state as a safety backup.
        assert_eq!(list_full_backups(&root).len(), 2);

        // Backups live outside the scanned tree, so they never appear as saves.
        assert_eq!(find_checkpoints(&root).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(backups_dir(&root));
    }

    #[test]
    fn per_file_backup_undo() {
        let root = temp_root("perfile");
        let file = root.join("R/INTER_3/Game/Checkpoints/02A Crossroads/02A SECU START NORTH.qdsav");
        backup(&file).unwrap();
        std::fs::write(&file, b"EDITED").unwrap();
        restore_latest_file_backup(&file).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"ORIGINAL");
        let _ = std::fs::remove_dir_all(&root);
    }
}
