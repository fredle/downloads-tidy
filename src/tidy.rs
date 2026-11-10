//! File system side: scan a folder, carry out a plan, and keep the undo log
//! of the last run in `%LOCALAPPDATA%\DownloadsTidy\last_run.json`.

use crate::rules::{Dest, FileInfo, PlannedMove, unique_path};
use serde::{Deserialize, Serialize};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

/// List the files directly inside `folder` (subfolders are never touched).
pub fn scan(folder: &Path) -> Result<Vec<FileInfo>, String> {
    let read = std::fs::read_dir(folder).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "This folder does not exist.".to_string(),
        _ => format!("Cannot read this folder: {e}"),
    })?;
    let mut files = Vec::new();
    for entry in read.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || meta.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // Share mode 0 fails if anyone else has the file open, e.g. a browser
        // that is still downloading it.
        let in_use = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .is_err();
        files.push(FileInfo {
            path,
            name,
            size: meta.len(),
            modified: meta.modified().unwrap_or(UNIX_EPOCH),
            in_use,
        });
    }
    files.sort_by_key(|f| f.name.to_lowercase());
    Ok(files)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub original: PathBuf,
    /// Where it went; None when it was sent to the Recycle Bin.
    pub moved_to: Option<PathBuf>,
    pub rule: String,
    #[serde(default)]
    pub restored: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RunLog {
    /// Unix seconds when the run happened.
    pub time: u64,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub entries: Vec<Entry>,
    pub errors: Vec<String>,
}

/// Carry out a plan. Failures are collected, never fatal to the rest.
pub fn execute(moves: &[PlannedMove]) -> Outcome {
    let mut out = Outcome::default();
    for m in moves {
        let result = match &m.dest {
            Dest::Recycle => crate::recycle::send(&m.src).map(|_| None),
            Dest::Folder(dest) => move_file(&m.src, dest).map(Some),
        };
        match result {
            Ok(moved_to) => out.entries.push(Entry {
                original: m.src.clone(),
                moved_to,
                rule: m.rule.clone(),
                restored: false,
            }),
            Err(e) => out.errors.push(format!("{}: {e}", m.name)),
        }
    }
    out
}

fn move_file(src: &Path, dest: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Re-check at the last moment so we never overwrite anything.
    let final_dest = unique_path(dest, &|p| p.exists());
    std::fs::rename(src, &final_dest).map_err(|e| e.to_string())?;
    Ok(final_dest)
}

/// Put one moved file back where it came from.
pub fn restore(entry: &Entry) -> Result<PathBuf, String> {
    let from = entry.moved_to.as_ref().ok_or("It is in the Recycle Bin: restore it from there.")?;
    if !from.exists() {
        return Err("The file is no longer there.".into());
    }
    if let Some(parent) = entry.original.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let target = unique_path(&entry.original, &|p| p.exists());
    std::fs::rename(from, &target).map_err(|e| e.to_string())?;
    // Tidy up the subfolder if we just emptied it (fails harmlessly if not).
    if let Some(dir) = from.parent() {
        let _ = std::fs::remove_dir(dir);
    }
    Ok(target)
}

fn log_path() -> PathBuf {
    crate::settings::data_dir().join("last_run.json")
}

pub fn load_log() -> Option<RunLog> {
    let s = std::fs::read_to_string(log_path()).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn save_log(log: &RunLog) {
    let _ = std::fs::create_dir_all(crate::settings::data_dir());
    if let Ok(json) = serde_json::to_string_pretty(log) {
        let _ = std::fs::write(log_path(), json);
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

pub fn format_ago(secs: u64) -> String {
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Rule, plan};
    use std::time::{Duration, SystemTime};

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dtidy-{tag}-{}-{}", std::process::id(), now_secs()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn formats_sizes_and_ages() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_ago(10), "just now");
        assert_eq!(format_ago(120), "2 min ago");
        assert_eq!(format_ago(7200), "2 h ago");
        assert_eq!(format_ago(3 * 86_400), "3 days ago");
    }

    #[test]
    fn scan_missing_folder_is_an_error() {
        assert!(scan(Path::new(r"C:\definitely\not\here\xyz")).is_err());
    }

    #[test]
    fn scan_plan_execute_and_restore_round_trip() {
        let dir = temp_dir("rt");
        std::fs::write(dir.join("a.pdf"), b"hello").unwrap();
        std::fs::write(dir.join("b.zip"), b"zip").unwrap();
        std::fs::create_dir(dir.join("Documents")).unwrap();
        std::fs::write(dir.join("Documents").join("a.pdf"), b"existing").unwrap();
        std::fs::create_dir(dir.join("subdir")).unwrap();

        let files = scan(&dir).unwrap();
        assert_eq!(files.len(), 2, "subfolders and their contents are not listed");

        let rules = vec![
            Rule::new_move("Docs", "pdf", "Documents"),
            Rule::new_move("Zips", "zip", "Archives"),
        ];
        let now = SystemTime::now() + Duration::from_secs(3600);
        let p = plan(&dir, &files, &rules, now, 10, &|p| p.exists());
        // Documents\a.pdf (existing) is a file in a subfolder, not scanned.
        assert_eq!(p.moves.len(), 2);

        let out = execute(&p.moves);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert!(!dir.join("a.pdf").exists());
        assert_eq!(std::fs::read(dir.join("Documents").join("a (1).pdf")).unwrap(), b"hello");
        assert_eq!(std::fs::read(dir.join("Documents").join("a.pdf")).unwrap(), b"existing");
        assert!(dir.join("Archives").join("b.zip").exists());

        for e in &out.entries {
            restore(e).unwrap();
        }
        assert_eq!(std::fs::read(dir.join("a.pdf")).unwrap(), b"hello");
        assert!(dir.join("b.zip").exists());
        assert!(!dir.join("Archives").exists(), "emptied subfolder removed");
        assert!(dir.join("Documents").join("a.pdf").exists(), "pre-existing file kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_avoids_overwriting_and_reports_missing() {
        let dir = temp_dir("rs");
        let moved = dir.join("Sub").join("x.txt");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::write(&moved, b"moved").unwrap();
        std::fs::write(dir.join("x.txt"), b"new one").unwrap();
        let entry = Entry {
            original: dir.join("x.txt"),
            moved_to: Some(moved.clone()),
            rule: "r".into(),
            restored: false,
        };
        let back = restore(&entry).unwrap();
        assert_eq!(back, dir.join("x (1).txt"));
        assert_eq!(std::fs::read(dir.join("x.txt")).unwrap(), b"new one");
        assert!(restore(&entry).is_err(), "already gone");
        let recycled = Entry { moved_to: None, ..entry };
        assert!(restore(&recycled).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn execute_reports_failures_without_stopping() {
        let dir = temp_dir("ex");
        std::fs::write(dir.join("ok.txt"), b"1").unwrap();
        let moves = vec![
            PlannedMove {
                src: dir.join("gone.txt"),
                name: "gone.txt".into(),
                size: 0,
                rule: "r".into(),
                dest: Dest::Folder(dir.join("T").join("gone.txt")),
            },
            PlannedMove {
                src: dir.join("ok.txt"),
                name: "ok.txt".into(),
                size: 1,
                rule: "r".into(),
                dest: Dest::Folder(dir.join("T").join("ok.txt")),
            },
        ];
        let out = execute(&moves);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.entries.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
