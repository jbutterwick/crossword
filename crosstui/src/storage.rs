//! Locating, scanning, and managing the local puzzle library on disk.
//!
//! Puzzles are plain `.puz` files in a per-user data directory. Solve progress
//! lives inside each `.puz` (see [`crossword::Puzzle::to_puz_bytes`]), so the
//! sidecars keep favorites plus download source/date provenance that the puzzle
//! format itself cannot represent.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Local};
use crossword::Puzzle;
use serde::{Deserialize, Serialize};

const FAVORITES_FILE: &str = "favorites.txt";
const METADATA_FILE: &str = "library.toml";
static METADATA_LOCK: Mutex<()> = Mutex::new(());

/// Download provenance kept outside the `.puz`, whose format has no fields for it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct LibraryMetadata {
    #[serde(default)]
    puzzles: Vec<PuzzleMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PuzzleMetadata {
    file: String,
    source: String,
    #[serde(default)]
    source_id: Option<String>,
    downloaded: String,
    #[serde(default)]
    puzzle_date: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Unsolved,
    InProgress,
    Complete,
    /// The file couldn't be parsed; kept in the list so the user can delete it.
    Unreadable,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Unsolved => "Unsolved",
            Status::InProgress => "In progress",
            Status::Complete => "Complete",
            Status::Unreadable => "Unreadable",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub title: String,
    pub author: String,
    pub source: String,
    /// Local download/import date, formatted as `YYYY-MM-DD`.
    pub downloaded: String,
    pub status: Status,
    pub favorite: bool,
}

impl Entry {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// The library directory (e.g. `~/.local/share/crosstui`), created if missing.
pub fn library_dir() -> PathBuf {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("crosstui");
    // ponytail: ignore the error here; scan() surfaces a real failure to read it.
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Scans the library for `*.puz` files, deriving status and favorite flags.
/// Unreadable files are included (marked [`Status::Unreadable`]) so they can be
/// deleted from the UI. The library screen applies the user-selected ordering.
pub fn scan(dir: &Path) -> Vec<Entry> {
    let favorites = load_favorites(dir);
    let metadata = load_metadata(dir);
    let mut entries: Vec<Entry> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "puz"))
            .map(|path| {
                let favorite = path
                    .file_name()
                    .is_some_and(|n| favorites.contains(&n.to_string_lossy().into_owned()));
                let meta = path
                    .file_name()
                    .and_then(|name| metadata.puzzles.iter().find(|m| name == m.file.as_str()));
                read_entry(path, favorite, meta)
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    entries.sort_by(|a, b| {
        b.favorite
            .cmp(&a.favorite)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    entries
}

fn read_entry(path: PathBuf, favorite: bool, metadata: Option<&PuzzleMetadata>) -> Entry {
    let source = metadata
        .map(|m| m.source.clone())
        .unwrap_or_else(|| "Local file".to_string());
    let downloaded = metadata
        .map(|m| m.downloaded.clone())
        .unwrap_or_else(|| file_date(&path));

    match fs::read(&path).map(Puzzle::parse) {
        Ok(Ok((puzzle, _mismatches))) => {
            let status = if puzzle.is_solved() {
                Status::Complete
            } else if puzzle.is_started() {
                Status::InProgress
            } else {
                Status::Unsolved
            };
            let mut title = puzzle.title().to_string();
            if title.is_empty() {
                title = file_stem(&path);
            }
            Entry {
                path,
                title,
                author: puzzle.author().to_string(),
                source,
                downloaded,
                status,
                favorite,
            }
        }
        _ => Entry {
            title: file_stem(&path),
            author: String::new(),
            source,
            downloaded,
            status: Status::Unreadable,
            favorite,
            path,
        },
    }
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_date(path: &Path) -> String {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|time| DateTime::<Local>::from(time).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| "Unknown".to_string())
}

fn load_metadata(dir: &Path) -> LibraryMetadata {
    fs::read_to_string(dir.join(METADATA_FILE))
        .ok()
        .and_then(|body| toml::from_str(&body).ok())
        .unwrap_or_default()
}

fn save_metadata(dir: &Path, metadata: &LibraryMetadata) -> std::io::Result<()> {
    let body = toml::to_string_pretty(metadata)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let temporary = dir.join(format!("{METADATA_FILE}.tmp"));
    let destination = dir.join(METADATA_FILE);
    fs::write(&temporary, body)?;
    // Unix renames over the destination atomically. Windows requires removing
    // the old file first; the process-local lock still prevents lost updates.
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(&destination)?;
    }
    fs::rename(temporary, destination)
}

/// Records where a newly downloaded puzzle came from.
pub fn record_download(
    dir: &Path,
    path: &Path,
    source: &str,
    source_id: Option<&str>,
    puzzle_date: Option<&str>,
) -> std::io::Result<()> {
    let Some(file) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Ok(());
    };
    let _guard = METADATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut metadata = load_metadata(dir);
    metadata.puzzles.retain(|m| m.file != file);
    metadata.puzzles.push(PuzzleMetadata {
        file,
        source: source.to_string(),
        source_id: source_id.map(str::to_string),
        downloaded: Local::now().format("%Y-%m-%d").to_string(),
        puzzle_date: puzzle_date.map(str::to_string),
    });
    save_metadata(dir, &metadata)
}

/// Returns the locally stored puzzle for a source/date, if the file still exists.
pub fn source_puzzle_on(dir: &Path, source_id: &str, date: &str) -> Option<Entry> {
    let metadata = load_metadata(dir);
    let record = metadata.puzzles.iter().find(|m| {
        m.source_id.as_deref() == Some(source_id) && m.puzzle_date.as_deref() == Some(date)
    })?;
    let path = dir.join(&record.file);
    path.exists().then(|| {
        let favorite = load_favorites(dir).contains(&record.file);
        read_entry(path, favorite, Some(record))
    })
}

fn load_favorites(dir: &Path) -> HashSet<String> {
    fs::read_to_string(dir.join(FAVORITES_FILE))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn save_favorites(dir: &Path, favorites: &HashSet<String>) -> std::io::Result<()> {
    let mut list: Vec<&String> = favorites.iter().collect();
    list.sort();
    let body = list
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.join(FAVORITES_FILE), body)
}

/// Toggles the favorite flag for `file_name` and persists the change.
pub fn toggle_favorite(dir: &Path, file_name: &str) -> std::io::Result<()> {
    let mut favorites = load_favorites(dir);
    if !favorites.remove(file_name) {
        favorites.insert(file_name.to_string());
    }
    save_favorites(dir, &favorites)
}

/// Renames a puzzle file (keeping the `.puz` extension), carrying its favorite
/// flag over to the new name.
pub fn rename(dir: &Path, old_path: &Path, new_stem: &str) -> std::io::Result<()> {
    let new_stem = new_stem.trim();
    if new_stem.is_empty() {
        return Ok(());
    }
    let new_path = dir.join(format!("{new_stem}.puz"));
    fs::rename(old_path, &new_path)?;

    let old_name = old_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    let mut favorites = load_favorites(dir);
    if old_name.as_ref().is_some_and(|n| favorites.remove(n)) {
        favorites.insert(format!("{new_stem}.puz"));
        save_favorites(dir, &favorites)?;
    }

    if let Some(old_name) = old_name {
        let _guard = METADATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut metadata = load_metadata(dir);
        if let Some(record) = metadata.puzzles.iter_mut().find(|m| m.file == old_name) {
            record.file = format!("{new_stem}.puz");
            save_metadata(dir, &metadata)?;
        }
    }
    Ok(())
}

/// Resets a puzzle back to its not-started state, clearing all entered letters
/// and writing the cleared puzzle back to disk.
pub fn reset(path: &Path) -> std::io::Result<()> {
    let data = fs::read(path)?;
    let (mut puzzle, _) = Puzzle::parse(data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
    puzzle.reset();
    fs::write(path, puzzle.to_puz_bytes())
}

/// Deletes a puzzle file and drops it from the favorites list.
pub fn delete(dir: &Path, path: &Path) -> std::io::Result<()> {
    fs::remove_file(path)?;
    if let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) {
        let mut favorites = load_favorites(dir);
        if favorites.remove(&name) {
            save_favorites(dir, &favorites)?;
        }
        let _guard = METADATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut metadata = load_metadata(dir);
        let old_len = metadata.puzzles.len();
        metadata.puzzles.retain(|m| m.file != name);
        if metadata.puzzles.len() != old_len {
            save_metadata(dir, &metadata)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{delete, record_download, rename, scan, source_puzzle_on};

    #[test]
    fn provenance_follows_rename_and_delete() {
        let dir = std::env::temp_dir().join(format!(
            "crosstui-storage-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("original.puz");
        std::fs::write(
            &path,
            include_bytes!("../../crossword/puzzles/version-1.2-puzzle.puz"),
        )
        .unwrap();

        record_download(
            &dir,
            &path,
            "Test Gazette",
            Some("test"),
            Some("2026-07-13"),
        )
        .unwrap();
        let entries = scan(&dir);
        assert_eq!(entries[0].source, "Test Gazette");
        assert_eq!(entries[0].downloaded.len(), 10);

        rename(&dir, &path, "renamed").unwrap();
        let renamed = source_puzzle_on(&dir, "test", "2026-07-13").unwrap();
        assert_eq!(renamed.file_name(), "renamed.puz");

        delete(&dir, &renamed.path).unwrap();
        assert!(source_puzzle_on(&dir, "test", "2026-07-13").is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
