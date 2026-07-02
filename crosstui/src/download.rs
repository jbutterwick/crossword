//! Downloading puzzles by shelling out to `xword-dl`
//! (<https://github.com/thisisparker/xword-dl>), which handles the per-outlet
//! scraping and `.puz` conversion we don't want to reimplement.

use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

use crossword::Puzzle;

/// A downloadable puzzle outlet. `keyword` is the `xword-dl` outlet name;
/// `about` is a one-line hint shown in the UI.
pub struct Source {
    pub name: &'static str,
    pub keyword: &'static str,
    pub about: &'static str,
}

/// The outlets offered in the UI. Keywords come from `xword-dl --help`; add a
/// row here to extend it. Descriptions are kept vague on purpose — schedules and
/// paywalls change, and xword-dl surfaces the real error if a fetch fails.
pub const SOURCES: &[Source] = &[
    Source {
        name: "Newsday",
        keyword: "nd",
        about: "Daily; gentle",
    },
    Source {
        name: "USA Today",
        keyword: "usa",
        about: "Daily; easy",
    },
    Source {
        name: "Universal",
        keyword: "uni",
        about: "Daily",
    },
    Source {
        name: "LA Times",
        keyword: "lat",
        about: "Daily",
    },
    Source {
        name: "LA Times Mini",
        keyword: "latm",
        about: "Daily; 5x5",
    },
    Source {
        name: "The New Yorker",
        keyword: "tny",
        about: "Weekdays; themeless",
    },
    Source {
        name: "New Yorker Mini",
        keyword: "tnym",
        about: "Daily; mini",
    },
    Source {
        name: "Washington Post",
        keyword: "wp",
        about: "Daily",
    },
    Source {
        name: "The Atlantic",
        keyword: "atl",
        about: "Several per week",
    },
    Source {
        name: "Vox",
        keyword: "vox",
        about: "Occasional",
    },
    Source {
        name: "Vulture",
        keyword: "vult",
        about: "Weekly; pop culture",
    },
    Source {
        name: "The Daily Beast",
        keyword: "db",
        about: "Daily",
    },
    Source {
        name: "Crossword Club",
        keyword: "club",
        about: "Daily",
    },
    Source {
        name: "Guardian Quick",
        keyword: "grdq",
        about: "Daily; UK",
    },
    Source {
        name: "Guardian Cryptic",
        keyword: "grdc",
        about: "Daily; cryptic",
    },
    Source {
        name: "Guardian Everyman",
        keyword: "grde",
        about: "Weekly; cryptic",
    },
    Source {
        name: "Puzzmo",
        keyword: "pzm",
        about: "Daily",
    },
    Source {
        name: "The Walrus",
        keyword: "wal",
        about: "Monthly; Canada",
    },
    Source {
        name: "Simply Daily Puzzles",
        keyword: "sdp",
        about: "Daily",
    },
];

const NOT_INSTALLED: &str = "xword-dl not installed — run: pipx install xword-dl";

/// Fetches a puzzle for `keyword` into `dir`. `date` (`YYYY-MM-DD`) requests an
/// older puzzle where the outlet supports it; `None` gets the latest. Blocks
/// until the subprocess finishes.
// ponytail: synchronous; the UI freezes for the (brief) download. Move to a
// background thread + channel only if it starts to feel slow.
pub fn download(dir: &Path, keyword: &str, date: Option<&str>) -> Result<(), String> {
    let mut cmd = Command::new("xword-dl");
    cmd.arg(keyword);
    if let Some(d) = date {
        cmd.args(["-d", d]);
    }
    cmd.current_dir(dir);
    run(&mut cmd)
}

/// Downloads a puzzle from an arbitrary `url`. Direct `.puz` links are fetched
/// with `curl` and validated as real puzzles; anything else is handed to
/// `xword-dl`, which knows how to scrape many outlet pages.
pub fn download_url(dir: &Path, url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("no URL entered".to_string());
    }
    if is_puz_url(url) {
        fetch_puz(dir, url)
    } else {
        run(Command::new("xword-dl").arg(url).current_dir(dir))
    }
}

/// Best-effort: ensure `xword-dl` is on `PATH`, installing it via pipx (or pip
/// as a fallback) if missing. Called once at startup, before the TUI takes over
/// the screen, so its progress is visible. Never fatal — downloads just fail
/// with [`NOT_INSTALLED`] if this couldn't get it installed.
pub fn ensure_installed() {
    if on_path("xword-dl") {
        return;
    }
    eprintln!("xword-dl not found — attempting to install it for puzzle downloads…");
    for (bin, args) in [
        ("pipx", &["install", "xword-dl"][..]),
        ("pip3", &["install", "--user", "xword-dl"][..]),
        ("pip", &["install", "--user", "xword-dl"][..]),
    ] {
        if !on_path(bin) {
            continue;
        }
        eprintln!("  installing with {bin}…");
        if matches!(Command::new(bin).args(args).status(), Ok(s) if s.success()) {
            return;
        }
    }
    eprintln!(
        "Couldn't auto-install xword-dl; downloads stay disabled. Install it yourself with: pipx install xword-dl"
    );
}

/// A URL points at a raw `.puz` if its path (ignoring any query/fragment) ends
/// in `.puz`.
fn is_puz_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.to_ascii_lowercase().ends_with(".puz")
}

/// Fetches a direct `.puz` link with curl and confirms it parses before keeping
/// it, so a redirected HTML error page never lands in the library as a puzzle.
fn fetch_puz(dir: &Path, url: &str) -> Result<(), String> {
    let dest = dir.join(url_filename(url));
    curl_to(&dest, url)?;
    validate_puz(&dest)
}

// --- Crosshare -------------------------------------------------------------
//
// Crosshare (<https://crosshare.org>, open source, no auth) is the one outlet
// we found that publishes a browsable *list* of puzzles: its listing pages are
// plain HTML full of `/crosswords/{id}/{slug}` links, and `/api/puz/{id}`
// returns a real `.puz`. xword-dl doesn't support it, so we scrape it ourselves.

const CROSSHARE: &str = "https://crosshare.org";

/// A browsable Crosshare listing. `paged` feeds accept a 1-based page suffix.
pub struct CrosshareFeed {
    pub name: &'static str,
    pub path: &'static str,
    pub paged: bool,
}

pub const CROSSHARE_FEEDS: &[CrosshareFeed] = &[
    CrosshareFeed {
        name: "Newest",
        path: "newest",
        paged: true,
    },
    CrosshareFeed {
        name: "Featured",
        path: "featured",
        paged: true,
    },
    CrosshareFeed {
        name: "Daily Minis",
        path: "dailyminis",
        paged: false,
    },
];

/// One puzzle in a Crosshare listing.
pub struct CrossharePuzzle {
    pub id: String,
    pub title: String,
}

/// Lists puzzles from a Crosshare feed page (1-based) by scraping its public
/// listing HTML.
pub fn list_crosshare(feed: &CrosshareFeed, page: usize) -> Result<Vec<CrossharePuzzle>, String> {
    let url = if feed.paged {
        format!("{CROSSHARE}/{}/{}", feed.path, page.max(1))
    } else {
        format!("{CROSSHARE}/{}", feed.path)
    };
    let out = Command::new("curl")
        .args(["-fsSL", &url])
        .output()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => "curl not found".to_string(),
            _ => e.to_string(),
        })?;
    if !out.status.success() {
        return Err(last_line(&out.stderr, "couldn't reach crosshare.org"));
    }
    let puzzles = parse_crosshare_list(&String::from_utf8_lossy(&out.stdout));
    if puzzles.is_empty() {
        Err("no puzzles here (past the end of the list?)".to_string())
    } else {
        Ok(puzzles)
    }
}

/// Downloads a Crosshare puzzle by id into `dir`, naming the file after its
/// title, and validates it.
pub fn download_crosshare(dir: &Path, id: &str, title: &str) -> Result<(), String> {
    let dest = dir.join(format!("{}.puz", sanitize(title)));
    curl_to(&dest, &format!("{CROSSHARE}/api/puz/{id}"))?;
    validate_puz(&dest)
}

/// Pulls unique `/crosswords/{id}/{slug}` links out of a listing page, in order.
// ponytail: a hand-rolled scan, not an HTML parser — the markup is stable and a
// parser dependency would dwarf this. If Crosshare's HTML changes, fix here.
fn parse_crosshare_list(html: &str) -> Vec<CrossharePuzzle> {
    const NEEDLE: &str = "/crosswords/";
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut rest = html;
    while let Some(pos) = rest.find(NEEDLE) {
        rest = &rest[pos + NEEDLE.len()..];
        let Some(slash) = rest.find('/') else {
            continue;
        };
        let id = &rest[..slash];
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let after = &rest[slash + 1..];
        let end = after
            .find(['"', '\'', '<', ' ', '?', '#'])
            .unwrap_or(after.len());
        let slug = &after[..end];
        if !slug.is_empty() && seen.insert(id.to_string()) {
            out.push(CrossharePuzzle {
                id: id.to_string(),
                title: slug.replace('-', " "),
            });
        }
    }
    out
}

/// Downloads `url` to `dest` with curl.
fn curl_to(dest: &Path, url: &str) -> Result<(), String> {
    let out = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(dest)
        .arg(url)
        .output()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => "curl not found".to_string(),
            _ => e.to_string(),
        })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(last_line(&out.stderr, "download failed"))
    }
}

/// Confirms a freshly-downloaded file parses as a `.puz`, deleting it if not so
/// an HTML error page never lands in the library as a puzzle.
fn validate_puz(dest: &Path) -> Result<(), String> {
    match std::fs::read(dest).map(Puzzle::parse) {
        Ok(Ok(_)) => Ok(()),
        _ => {
            let _ = std::fs::remove_file(dest);
            Err("that didn't return a valid .puz file".to_string())
        }
    }
}

/// Makes a title safe to use as a filename.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "crosshare".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Filename to save a downloaded URL under: the last path segment if it looks
/// like a `.puz`, else a generic name.
// ponytail: clobbers an existing file of the same name; rare enough to ignore.
fn url_filename(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    match path.rsplit('/').next() {
        Some(name) if name.to_ascii_lowercase().ends_with(".puz") => name.to_string(),
        _ => "downloaded.puz".to_string(),
    }
}

/// Runs an `xword-dl` command, mapping a missing binary and non-zero exits to
/// friendly messages.
fn run(cmd: &mut Command) -> Result<(), String> {
    match cmd.output() {
        Err(e) if e.kind() == ErrorKind::NotFound => Err(NOT_INSTALLED.to_string()),
        Err(e) => Err(e.to_string()),
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(last_line(&out.stderr, "download failed")),
    }
}

/// The last non-blank line of some command output, or `fallback`.
fn last_line(bytes: &[u8], fallback: &str) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Whether `bin` can be spawned at all (exists on `PATH`), regardless of exit code.
fn on_path(bin: &str) -> bool {
    !matches!(
        Command::new(bin).arg("--help").output(),
        Err(e) if e.kind() == ErrorKind::NotFound
    )
}

#[cfg(test)]
mod tests {
    use super::{is_puz_url, parse_crosshare_list, sanitize, url_filename};

    #[test]
    fn recognizes_puz_urls() {
        assert!(is_puz_url("https://example.com/a/today.PUZ"));
        assert!(is_puz_url("https://example.com/x.puz?v=2#frag"));
        assert!(!is_puz_url(
            "https://crosshare.org/crosswords/abc/some-title"
        ));
        assert_eq!(url_filename("https://x.com/dir/mon.puz?t=1"), "mon.puz");
        assert_eq!(url_filename("https://x.com/scrape/page"), "downloaded.puz");
    }

    #[test]
    fn scrapes_crosshare_links() {
        let html = r#"
            <a href="/crosswords/abc123/daily-mini-2-july">Mini</a>
            <a href="/crosswords/abc123/daily-mini-2-july">dup, same id</a>
            <a href="/crosswords/XYZ9/im-a-belieber">Belieber</a>
            <a href="/crosswords/">bare, no id</a>
        "#;
        let puzzles = parse_crosshare_list(html);
        assert_eq!(puzzles.len(), 2, "dedups by id and skips the bare link");
        assert_eq!(puzzles[0].id, "abc123");
        assert_eq!(puzzles[0].title, "daily mini 2 july");
        assert_eq!(puzzles[1].title, "im a belieber");
        assert_eq!(
            sanitize("Prank Call: Victim/Mini"),
            "Prank Call_ Victim_Mini"
        );
    }
}
