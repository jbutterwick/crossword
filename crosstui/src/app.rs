use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use crossterm::execute;
use crossword::Puzzle;
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Widget};

use crate::screens::{LibraryScreen, SolveScreen, SourcesScreen, ThemesScreen};
use crate::theme;

/// What an active screen asks the [`App`] to do in response to a key press.
pub enum Transition {
    None,
    Quit,
    /// Open a puzzle file in the solve screen.
    Open(PathBuf),
    ToLibrary,
    ToSources,
    ToThemes,
}

/// Redraw cadence while idle — keeps the clue marquee moving and stays responsive to keys.
const TICK: Duration = Duration::from_millis(120);

enum Screen {
    Library(LibraryScreen),
    Sources(SourcesScreen),
    Solve(SolveScreen),
    Themes(ThemesScreen),
}

pub struct App {
    screen: Screen,
    library_dir: PathBuf,
    running: bool,
}

impl App {
    /// Starts on the library screen.
    pub fn new(library_dir: PathBuf) -> Self {
        let screen = Screen::Library(LibraryScreen::new(&library_dir));
        Self {
            screen,
            library_dir,
            running: true,
        }
    }

    /// Starts straight in the solve screen for a given puzzle (CLI back-compat).
    pub fn with_puzzle(library_dir: PathBuf, puzzle: Puzzle, path: PathBuf) -> Self {
        Self {
            screen: Screen::Solve(SolveScreen::new(puzzle, path)),
            library_dir,
            running: true,
        }
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        // ratatui sets up raw mode + alternate screen but not mouse capture.
        execute!(io::stdout(), EnableMouseCapture)?;
        let result = self.event_loop(terminal);
        // ponytail: best-effort restore; ratatui's panic hook handles the rest.
        let _ = execute!(io::stdout(), DisableMouseCapture);
        result
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while self.running {
            terminal.draw(|frame| frame.render_widget(&mut *self, frame.area()))?;
            // ponytail: redraw on a timer so the clue marquee animates while
            // idle. ratatui only flushes buffer diffs, so a static screen is cheap.
            if event::poll(TICK)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        let transition = match &mut self.screen {
                            Screen::Library(s) => s.on_key(key),
                            Screen::Sources(s) => s.on_key(key, &self.library_dir),
                            Screen::Solve(s) => s.on_key(key),
                            Screen::Themes(s) => s.on_key(key),
                        };
                        self.apply(transition);
                    }
                    Event::Mouse(mouse) => {
                        if let Screen::Solve(s) = &mut self.screen {
                            s.on_mouse(mouse);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn apply(&mut self, transition: Transition) {
        match transition {
            Transition::None => {}
            Transition::Quit => {
                self.save_progress();
                self.running = false;
            }
            Transition::Open(path) => {
                if let Ok(puzzle) = load_puzzle(&path) {
                    self.screen = Screen::Solve(SolveScreen::new(puzzle, path));
                }
                // Unreadable files just don't open; the library marks them so.
            }
            Transition::ToLibrary => {
                self.save_progress();
                self.screen = Screen::Library(LibraryScreen::new(&self.library_dir));
            }
            Transition::ToSources => {
                self.screen = Screen::Sources(SourcesScreen::new());
            }
            Transition::ToThemes => {
                self.screen = Screen::Themes(ThemesScreen::new(&self.library_dir));
            }
        }
    }

    /// Writes solve progress back to disk when leaving the solve screen.
    fn save_progress(&self) {
        if let Screen::Solve(s) = &self.screen {
            // ponytail: best-effort; a failed write just loses this session's progress.
            let _ = std::fs::write(s.path(), s.puzzle().to_puz_bytes());
        }
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Paint the themed background behind every screen.
        let t = theme::current();
        Block::new()
            .style(Style::new().bg(t.bg).fg(t.fg))
            .render(area, buf);

        match &mut self.screen {
            Screen::Library(s) => s.render(area, buf),
            Screen::Sources(s) => s.render(area, buf),
            Screen::Solve(s) => s.render(area, buf),
            Screen::Themes(s) => s.render(area, buf),
        }
    }
}

/// Loads and parses a `.puz` file. Returns an error string rather than exiting,
/// so callers in the UI can recover.
pub fn load_puzzle(path: &Path) -> Result<Puzzle, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let (puzzle, _mismatches) = Puzzle::parse(data).map_err(|e| format!("{e:?}"))?;
    Ok(puzzle)
}
