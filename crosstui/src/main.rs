use std::io;
use std::path::PathBuf;

use clap::Parser;
use clap::builder::Styles;
use clap::builder::styling::AnsiColor;

mod app;
mod download;
mod screens;
mod storage;
mod theme;

use app::{App, load_puzzle};

const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Blue.on_default().bold())
    .usage(AnsiColor::Blue.on_default().bold())
    .literal(AnsiColor::White.on_default())
    .placeholder(AnsiColor::Green.on_default());

#[derive(Parser, Debug)]
#[command(version, about, author, styles = HELP_STYLES)]
struct Cli {
    /// The path to a .puz file to open directly. Omit to browse your library.
    path: Option<PathBuf>,
}

fn main() -> io::Result<()> {
    let args = Cli::parse();

    // Best-effort: get the downloader in place before the TUI takes the screen.
    download::ensure_installed();

    let library_dir = storage::library_dir();

    // Apply the previously selected theme, if any.
    let themes = theme::load(&library_dir);
    if let Some(name) = theme::load_selected(&library_dir) {
        if let Some(entry) = themes.iter().find(|e| e.name == name) {
            theme::set(entry.theme);
        }
    }

    let app = match args.path {
        Some(path) => {
            let puzzle = load_puzzle(&path).unwrap_or_else(|e| {
                eprintln!("Failed to open {}: {e}", path.display());
                std::process::exit(1);
            });
            App::with_puzzle(library_dir, puzzle, path)
        }
        None => App::new(library_dir),
    };

    ratatui::run(|terminal| app.run(terminal))
}
