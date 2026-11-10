//! Rule definitions and the pure planning logic: given a list of files and a
//! list of rules, decide what would happen to each file. No file system
//! access happens here (callers pass an `exists` closure), so it is all unit
//! tested.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MIB: u64 = 1024 * 1024;

/// Extension groups offered as one-click presets in the rule editor.
pub const GROUPS: &[(&str, &str)] = &[
    ("Images", "jpg, jpeg, png, gif, bmp, webp, heic, svg, tif, tiff"),
    ("Documents", "pdf, doc, docx, xls, xlsx, ppt, pptx, txt, rtf, odt, csv"),
    ("Archives", "zip, rar, 7z, tar, gz, bz2, xz, iso"),
    ("Installers", "exe, msi, msix, appx, msixbundle"),
    ("Video", "mp4, mkv, mov, avi, wmv, webm"),
    ("Audio", "mp3, wav, flac, m4a, aac, ogg"),
];

/// Extensions browsers and download managers use while a file is incomplete.
const PARTIAL_EXTENSIONS: &[&str] = &[
    "crdownload", "part", "partial", "tmp", "download", "opdownload", "aria2", "!ut",
];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Action {
    /// Move into this subfolder of the tidied folder.
    MoveTo(String),
    /// Send to the Recycle Bin (never a permanent delete).
    Recycle,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Comma separated extensions, e.g. "jpg, png". Blank matches any type.
    #[serde(default)]
    pub extensions: String,
    /// Only files last modified at least this many days ago. 0 = any age.
    #[serde(default)]
    pub min_age_days: u32,
    /// Only files at least this large, in MB. 0 = any size.
    #[serde(default)]
    pub min_size_mb: u64,
    pub action: Action,
}

fn default_true() -> bool {
    true
}

impl Rule {
    pub fn new_move(name: &str, group: &str, folder: &str) -> Self {
        Rule {
            name: name.into(),
            enabled: true,
            extensions: group.into(),
            min_age_days: 0,
            min_size_mb: 0,
            action: Action::MoveTo(folder.into()),
        }
    }

    pub fn matches(&self, ext: &str, age: Duration, size: u64) -> bool {
        let exts = parse_extensions(&self.extensions);
        if !exts.is_empty() && !exts.iter().any(|e| e == ext) {
            return false;
        }
        if age < Duration::from_secs(u64::from(self.min_age_days) * 86_400) {
            return false;
        }
        size >= self.min_size_mb.saturating_mul(MIB)
    }
}

pub fn default_rules() -> Vec<Rule> {
    let g = |name: &str| {
        GROUPS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, e)| *e)
            .unwrap_or("")
    };
    let mut old_installers = Rule::new_move("Recycle old installers", g("Installers"), "");
    old_installers.action = Action::Recycle;
    old_installers.min_age_days = 60;
    old_installers.enabled = false;
    vec![
        old_installers,
        Rule::new_move("Installers", g("Installers"), "Installers"),
        Rule::new_move("Archives", g("Archives"), "Archives"),
        Rule::new_move("Documents", g("Documents"), "Documents"),
        Rule::new_move("Images", g("Images"), "Images"),
        Rule::new_move(
            "Video and audio",
            &format!("{}, {}", g("Video"), g("Audio")),
            "Media",
        ),
    ]
}

/// "JPG, .png; *.gif" -> ["jpg", "png", "gif"]
pub fn parse_extensions(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let e = part.trim().trim_start_matches('*').trim_start_matches('.').to_lowercase();
        if !e.is_empty() && !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

/// Append a preset group's extensions to an extension list, without repeats.
pub fn add_group(existing: &str, group: &str) -> String {
    let mut all = parse_extensions(existing);
    for e in parse_extensions(group) {
        if !all.contains(&e) {
            all.push(e);
        }
    }
    all.join(", ")
}

/// Validate a destination subfolder: relative, no `..`, no invalid characters.
/// Returns the cleaned path with backslash separators, or None if unusable.
pub fn sanitize_subfolder(s: &str) -> Option<String> {
    let s = s.trim().replace('/', "\\");
    let mut parts = Vec::new();
    for comp in s.split('\\') {
        let comp = comp.trim();
        if comp.is_empty() {
            continue;
        }
        if comp == "." || comp == ".." || comp.chars().any(|c| "<>:\"|?*".contains(c) || c.is_control()) {
            return None;
        }
        if comp.ends_with('.') {
            return None;
        }
        parts.push(comp);
    }
    if parts.is_empty() { None } else { Some(parts.join("\\")) }
}

/// If `dest` is taken, return "name (1).ext", "name (2).ext", ...
pub fn unique_path(dest: &Path, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    if !exists(dest) {
        return dest.to_path_buf();
    }
    let parent = dest.parent().unwrap_or(Path::new(""));
    let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = dest.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let mut n = 1u32;
    loop {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// A file found in the folder being tidied.
#[derive(Clone, Debug)]
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified: SystemTime,
    /// Another program has the file open (e.g. a browser still writing it).
    pub in_use: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Dest {
    Folder(PathBuf),
    Recycle,
}

#[derive(Clone, Debug)]
pub struct PlannedMove {
    pub src: PathBuf,
    pub name: String,
    pub size: u64,
    pub rule: String,
    pub dest: Dest,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub moves: Vec<PlannedMove>,
    /// Files left alone because they are recent, in use, or look incomplete.
    pub skipped_busy: usize,
    /// Files no rule applies to.
    pub unmatched: usize,
}

impl Plan {
    pub fn recycle_count(&self) -> usize {
        self.moves.iter().filter(|m| m.dest == Dest::Recycle).count()
    }
    pub fn move_count(&self) -> usize {
        self.moves.len() - self.recycle_count()
    }
    pub fn total_size(&self) -> u64 {
        self.moves.iter().map(|m| m.size).sum()
    }
}

/// Work out what each rule would do. The first enabled matching rule wins.
pub fn plan(
    base: &Path,
    files: &[FileInfo],
    rules: &[Rule],
    now: SystemTime,
    quiet_minutes: u32,
    exists: &dyn Fn(&Path) -> bool,
) -> Plan {
    let mut result = Plan::default();
    let mut reserved: HashSet<String> = HashSet::new();
    let quiet = Duration::from_secs(u64::from(quiet_minutes) * 60);

    for file in files {
        let ext = Path::new(&file.name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let age = now.duration_since(file.modified).unwrap_or(Duration::ZERO);

        if file.in_use || age < quiet || PARTIAL_EXTENSIONS.contains(&ext.as_str()) {
            result.skipped_busy += 1;
            continue;
        }

        let mut chosen = None;
        for rule in rules.iter().filter(|r| r.enabled) {
            if !rule.matches(&ext, age, file.size) {
                continue;
            }
            match &rule.action {
                Action::Recycle => {
                    chosen = Some((rule, Dest::Recycle));
                    break;
                }
                Action::MoveTo(folder) => {
                    // A rule with an unusable folder is ignored, not guessed at.
                    let Some(sub) = sanitize_subfolder(folder) else { continue };
                    let taken = |p: &Path| exists(p) || reserved.contains(&key(p));
                    let dest = unique_path(&base.join(sub).join(&file.name), &taken);
                    reserved.insert(key(&dest));
                    chosen = Some((rule, Dest::Folder(dest)));
                    break;
                }
            }
        }

        match chosen {
            Some((rule, dest)) => result.moves.push(PlannedMove {
                src: file.path.clone(),
                name: file.name.clone(),
                size: file.size,
                rule: rule.name.clone(),
                dest,
            }),
            None => result.unmatched += 1,
        }
    }
    result
}

fn key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, age_days: u64, now: SystemTime) -> FileInfo {
        FileInfo {
            path: PathBuf::from(r"C:\dl").join(name),
            name: name.into(),
            size,
            modified: now - Duration::from_secs(age_days * 86_400),
            in_use: false,
        }
    }

    fn never(_: &Path) -> bool {
        false
    }

    #[test]
    fn parses_extensions() {
        assert_eq!(parse_extensions("JPG, .png; *.gif  jpg"), vec!["jpg", "png", "gif"]);
        assert!(parse_extensions("  , ").is_empty());
    }

    #[test]
    fn adds_groups_without_duplicates() {
        assert_eq!(add_group("png", "jpg, png"), "png, jpg");
    }

    #[test]
    fn sanitizes_subfolders() {
        assert_eq!(sanitize_subfolder(" Docs/Old "), Some("Docs\\Old".into()));
        assert_eq!(sanitize_subfolder(""), None);
        assert_eq!(sanitize_subfolder("..\\x"), None);
        assert_eq!(sanitize_subfolder("C:\\x"), None);
        assert_eq!(sanitize_subfolder("a?b"), None);
    }

    #[test]
    fn makes_unique_paths() {
        let taken = |p: &Path| {
            let s = p.to_string_lossy();
            s.ends_with("a.txt") || s.ends_with("a (1).txt")
        };
        assert_eq!(unique_path(Path::new(r"C:\x\a.txt"), &taken), PathBuf::from(r"C:\x\a (2).txt"));
        assert_eq!(unique_path(Path::new(r"C:\x\b.txt"), &taken), PathBuf::from(r"C:\x\b.txt"));
    }

    #[test]
    fn rule_matching_by_type_age_and_size() {
        let mut r = Rule::new_move("x", "zip", "Z");
        r.min_age_days = 7;
        r.min_size_mb = 10;
        let day = Duration::from_secs(86_400);
        assert!(r.matches("zip", day * 8, 11 * MIB));
        assert!(!r.matches("rar", day * 8, 11 * MIB));
        assert!(!r.matches("zip", day * 6, 11 * MIB));
        assert!(!r.matches("zip", day * 8, 9 * MIB));
        let any = Rule::new_move("any", "", "Z");
        assert!(any.matches("whatever", Duration::ZERO, 0));
    }

    #[test]
    fn first_matching_rule_wins_and_unmatched_counted() {
        let now = SystemTime::now();
        let files = vec![file("a.pdf", 10, 1, now), file("b.xyz", 10, 1, now)];
        let rules = vec![
            Rule::new_move("Docs", "pdf", "Documents"),
            Rule::new_move("All", "", "Other"),
        ];
        let p = plan(Path::new(r"C:\dl"), &files, &rules, now, 0, &never);
        assert_eq!(p.moves.len(), 2);
        assert_eq!(p.moves[0].rule, "Docs");
        assert_eq!(p.moves[1].rule, "All");
        let p = plan(Path::new(r"C:\dl"), &files, &rules[..1], now, 0, &never);
        assert_eq!(p.unmatched, 1);
    }

    #[test]
    fn skips_recent_in_use_and_partial_files() {
        let now = SystemTime::now();
        let mut busy = file("c.pdf", 1, 1, now);
        busy.in_use = true;
        let mut recent = file("d.pdf", 1, 0, now);
        recent.modified = now - Duration::from_secs(60);
        let files = vec![
            busy,
            recent,
            file("e.crdownload", 1, 5, now),
            file("f.pdf", 1, 5, now),
        ];
        let rules = vec![Rule::new_move("All", "", "Other")];
        let p = plan(Path::new(r"C:\dl"), &files, &rules, now, 10, &never);
        assert_eq!(p.skipped_busy, 3);
        assert_eq!(p.moves.len(), 1);
        assert_eq!(p.moves[0].name, "f.pdf");
    }

    #[test]
    fn disabled_rules_and_bad_folders_are_ignored() {
        let now = SystemTime::now();
        let files = vec![file("a.pdf", 1, 1, now)];
        let mut off = Rule::new_move("off", "pdf", "X");
        off.enabled = false;
        let bad = Rule::new_move("bad", "pdf", "..\\escape");
        let good = Rule::new_move("good", "pdf", "Docs");
        let p = plan(Path::new(r"C:\dl"), &files, &[off, bad, good], now, 0, &never);
        assert_eq!(p.moves[0].rule, "good");
    }

    #[test]
    fn recycle_rule_and_totals() {
        let now = SystemTime::now();
        let files = vec![file("old.exe", 100, 90, now), file("new.exe", 50, 1, now)];
        let mut recycle = Rule::new_move("old", "exe", "");
        recycle.action = Action::Recycle;
        recycle.min_age_days = 60;
        let rules = vec![recycle, Rule::new_move("inst", "exe", "Installers")];
        let p = plan(Path::new(r"C:\dl"), &files, &rules, now, 0, &never);
        assert_eq!(p.recycle_count(), 1);
        assert_eq!(p.move_count(), 1);
        assert_eq!(p.total_size(), 150);
        assert_eq!(p.moves[0].dest, Dest::Recycle);
    }

    #[test]
    fn planned_destinations_do_not_collide() {
        let now = SystemTime::now();
        // Two files that would map to the same name cannot come from one
        // folder, but an existing file at the destination must be avoided.
        let files = vec![file("a.pdf", 1, 1, now)];
        let rules = vec![Rule::new_move("Docs", "pdf", "Documents")];
        let exists = |p: &Path| p.to_string_lossy().ends_with("Documents\\a.pdf");
        let p = plan(Path::new(r"C:\dl"), &files, &rules, now, 0, &exists);
        assert_eq!(
            p.moves[0].dest,
            Dest::Folder(PathBuf::from(r"C:\dl\Documents\a (1).pdf"))
        );
    }

    #[test]
    fn default_rules_are_valid() {
        for r in default_rules() {
            if let Action::MoveTo(f) = &r.action {
                assert!(sanitize_subfolder(f).is_some(), "{}", r.name);
            }
        }
    }
}
