use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListState, Padding, Paragraph, StatefulWidget, Widget};

use crate::app::Transition;
use crate::storage::{self, Entry, Status};
use crate::theme;

enum Mode {
    Normal,
    /// Editing a new filename for the selected puzzle.
    Renaming(String),
    /// Awaiting y/n confirmation to delete the selected puzzle.
    ConfirmDelete,
    /// Awaiting y/n confirmation to reset the selected puzzle's progress.
    ConfirmReset,
}

pub struct LibraryScreen {
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    mode: Mode,
}

impl LibraryScreen {
    pub fn new(dir: &std::path::Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            entries: storage::scan(dir),
            selected: 0,
            mode: Mode::Normal,
        }
    }

    fn rescan(&mut self) {
        self.entries = storage::scan(&self.dir);
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Transition {
        match &mut self.mode {
            Mode::Renaming(buffer) => {
                match key.code {
                    KeyCode::Enter => {
                        let buffer = buffer.clone();
                        if let Some(entry) = self.entries.get(self.selected) {
                            let _ = storage::rename(&self.dir, &entry.path.clone(), &buffer);
                        }
                        self.mode = Mode::Normal;
                        self.rescan();
                    }
                    KeyCode::Esc => self.mode = Mode::Normal,
                    KeyCode::Backspace => {
                        buffer.pop();
                    }
                    KeyCode::Char(c) => buffer.push(c),
                    _ => {}
                }
                Transition::None
            }
            Mode::ConfirmDelete => {
                if key.code == KeyCode::Char('y') {
                    if let Some(entry) = self.selected_entry() {
                        let _ = storage::delete(&self.dir, &entry.path.clone());
                    }
                    self.rescan();
                }
                self.mode = Mode::Normal;
                Transition::None
            }
            Mode::ConfirmReset => {
                if key.code == KeyCode::Char('y') {
                    if let Some(entry) = self.selected_entry() {
                        let _ = storage::reset(&entry.path.clone());
                    }
                    self.rescan();
                }
                self.mode = Mode::Normal;
                Transition::None
            }
            Mode::Normal => self.on_key_normal(key),
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) -> Transition {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Transition::Quit,
            KeyCode::Char('s') => return Transition::ToSources,
            KeyCode::Char('c') => return Transition::ToBrowse,
            KeyCode::Char('t') => return Transition::ToThemes,
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(entry) = self.selected_entry() {
                    if entry.status != Status::Unreadable {
                        return Transition::Open(entry.path.clone());
                    }
                }
            }
            KeyCode::Char('f') => {
                if let Some(entry) = self.selected_entry() {
                    let _ = storage::toggle_favorite(&self.dir, &entry.file_name());
                    self.rescan();
                }
            }
            KeyCode::Char('r') => {
                if let Some(entry) = self.selected_entry() {
                    let stem = entry
                        .path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.mode = Mode::Renaming(stem);
                }
            }
            KeyCode::Char('d') => {
                if self.selected_entry().is_some() {
                    self.mode = Mode::ConfirmDelete;
                }
            }
            KeyCode::Char('x') => {
                // Only worth confirming if there's actually progress to clear.
                if self
                    .selected_entry()
                    .is_some_and(|e| e.status == Status::InProgress || e.status == Status::Complete)
                {
                    self.mode = Mode::ConfirmReset;
                }
            }
            _ => {}
        }
        Transition::None
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let t = theme::current();
        let [title_area, list_area, footer_area] = area.layout(&Layout::vertical([
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(2),
        ]));

        Line::from(vec!["Crosstui".fg(t.accent()).bold(), " library".bold()])
            .centered()
            .render(title_area, buf);

        if self.entries.is_empty() {
            Paragraph::new("No puzzles yet. Press 's' to download some.".fg(t.muted))
                .block(
                    Block::bordered()
                        .border_style(Style::new().fg(t.muted))
                        .padding(Padding::uniform(1)),
                )
                .render(list_area, buf);
        } else {
            let mut list_state = ListState::default();
            list_state.select(Some(self.selected));
            let rows = self.entries.iter().map(entry_line);
            let list = List::new(rows)
                .highlight_style(Style::new().fg(t.sel_fg()).bg(t.sel_bg()).bold())
                .block(
                    Block::bordered()
                        .border_style(Style::new().fg(t.muted))
                        .padding(Padding::uniform(1)),
                );
            StatefulWidget::render(list, list_area, buf, &mut list_state);
        }

        let footer = match &self.mode {
            Mode::Renaming(buffer) => {
                Line::from(vec!["Rename to: ".bold(), buffer.clone().into(), "_".into()])
            }
            Mode::ConfirmDelete => Line::from(
                self.selected_entry()
                    .map(|e| format!("Delete \"{}\"? (y/n)", e.title))
                    .unwrap_or_default()
                    .fg(t.red)
                    .bold(),
            ),
            Mode::ConfirmReset => Line::from(
                self.selected_entry()
                    .map(|e| format!("Reset progress on \"{}\"? (y/n)", e.title))
                    .unwrap_or_default()
                    .fg(t.red)
                    .bold(),
            ),
            Mode::Normal => Line::from(
                "↑/↓ move   Enter open   f favorite   r rename   x reset   d delete   s sources   c crosshare   t themes   q quit"
                    .fg(t.muted),
            ),
        };
        footer.render(footer_area, buf);
    }
}

fn entry_line(entry: &Entry) -> Line<'static> {
    let t = theme::current();
    let star = if entry.favorite { "★ " } else { "  " };
    let status_span: Span<'static> = {
        let label = format!("{:<12}", entry.status.label());
        match entry.status {
            Status::Complete => label.fg(t.green),
            Status::InProgress => label.fg(t.yellow),
            Status::Unreadable => label.fg(t.red),
            Status::Unsolved => label.fg(t.muted),
        }
    };

    let mut spans = vec![star.fg(t.yellow), status_span, entry.title.clone().into()];
    if !entry.author.is_empty() {
        spans.push(format!(" — {}", entry.author).fg(t.muted));
    }
    Line::from(spans)
}
