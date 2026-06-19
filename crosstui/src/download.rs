//! Downloading puzzles by shelling out to `xword-dl`
//! (<https://github.com/thisisparker/xword-dl>), which handles the per-outlet
//! scraping and `.puz` conversion we don't want to reimplement.

use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

/// A downloadable puzzle outlet. `keyword` is the `xword-dl` outlet name.
pub struct Source {
    pub name: &'static str,
    pub keyword: &'static str,
}

/// The outlets offered in the UI. Add an `xword-dl` keyword here to extend it.
pub const SOURCES: &[Source] = &[
    Source { name: "Newsday", keyword: "nd" },
    Source { name: "USA Today", keyword: "usa" },
    Source { name: "Universal", keyword: "uni" },
    Source { name: "The New Yorker", keyword: "tny" },
    Source { name: "LA Times", keyword: "lat" },
    Source { name: "Washington Post", keyword: "wp" },
    Source { name: "The Atlantic", keyword: "atl" },
];

/// Fetches the latest puzzle for `keyword` into `dir`, letting `xword-dl` choose
/// the filename. Blocks until the subprocess finishes.
// ponytail: synchronous; the UI freezes for the (brief) download. Move to a
// background thread + channel only if it starts to feel slow.
pub fn download(dir: &Path, keyword: &str) -> Result<(), String> {
    let result = Command::new("xword-dl")
        .arg(keyword)
        .current_dir(dir)
        .output();

    match result {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            Err("xword-dl not installed — run: pipx install xword-dl".to_string())
        }
        Err(e) => Err(e.to_string()),
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg = stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("download failed");
            Err(msg.to_string())
        }
    }
}
