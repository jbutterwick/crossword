//! Locating, scanning, and managing the local puzzle library on disk.
//!
//! Puzzles are plain `.puz` files in a per-user data directory. Solve progress
//! lives inside each `.puz` (see [`crossword::Puzzle::to_puz_bytes`]), so the
//! only sidecar state we keep is a newline-delimited list of favorited
//! filenames in `favorites.txt`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crossword::Puzzle;

const FAVORITES_FILE: &str = "favorites.txt";

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
/// deleted from the UI. Sorted favorites-first, then by title.
pub fn scan(dir: &Path) -> Vec<Entry> {
    let favorites = load_favorites(dir);
    let mut entries: Vec<Entry> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "puz"))
            .map(|path| {
                let favorite = path
                    .file_name()
                    .is_some_and(|n| favorites.contains(&n.to_string_lossy().into_owned()));
                read_entry(path, favorite)
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

fn read_entry(path: PathBuf, favorite: bool) -> Entry {
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
                status,
                favorite,
            }
        }
        _ => Entry {
            title: file_stem(&path),
            author: String::new(),
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
    if old_name.is_some_and(|n| favorites.remove(&n)) {
        favorites.insert(format!("{new_stem}.puz"));
        save_favorites(dir, &favorites)?;
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
    }
    Ok(())
}
