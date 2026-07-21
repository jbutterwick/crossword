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

#[derive(Clone, Copy)]
enum SortOrder {
    Downloaded,
    Title,
    Source,
    Status,
}

impl SortOrder {
    fn next(self) -> Self {
        match self {
            Self::Downloaded => Self::Title,
            Self::Title => Self::Source,
            Self::Source => Self::Status,
            Self::Status => Self::Downloaded,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Downloaded => "Downloaded (newest)",
            Self::Title => "Title (A–Z)",
            Self::Source => "Source (A–Z)",
            Self::Status => "Solve status",
        }
    }
}

#[derive(Clone, Copy)]
enum Filter {
    All,
    Favorites,
    Unsolved,
    InProgress,
    Complete,
}

impl Filter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Favorites,
            Self::Favorites => Self::Unsolved,
            Self::Unsolved => Self::InProgress,
            Self::InProgress => Self::Complete,
            Self::Complete => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All puzzles",
            Self::Favorites => "Favorites",
            Self::Unsolved => "Unsolved",
            Self::InProgress => "In progress",
            Self::Complete => "Complete",
        }
    }

    fn matches(self, entry: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::Favorites => entry.favorite,
            Self::Unsolved => entry.status == Status::Unsolved,
            Self::InProgress => entry.status == Status::InProgress,
            Self::Complete => entry.status == Status::Complete,
        }
    }
}

enum Mode {
    Normal,
    Searching,
    /// Editing a new filename for the selected puzzle.
    Renaming(String),
    /// Awaiting y/n confirmation to delete the selected puzzle.
    ConfirmDelete,
    /// Awaiting y/n confirmation to reset the selected puzzle's progress.
    ConfirmReset,
}

pub struct LibraryScreen {
    dir: PathBuf,
    all_entries: Vec<Entry>,
    entries: Vec<Entry>,
    selected: usize,
    mode: Mode,
    query: String,
    filter: Filter,
    sort: SortOrder,
}

impl LibraryScreen {
    pub fn new(dir: &std::path::Path) -> Self {
        let all_entries = storage::scan(dir);
        let mut screen = Self {
            dir: dir.to_path_buf(),
            all_entries,
            entries: Vec::new(),
            selected: 0,
            mode: Mode::Normal,
            query: String::new(),
            filter: Filter::All,
            sort: SortOrder::Downloaded,
        };
        screen.apply_view();
        screen
    }

    fn rescan(&mut self) {
        let selected_path = self.selected_entry().map(|e| e.path.clone());
        self.all_entries = storage::scan(&self.dir);
        self.apply_view();
        if let Some(path) = selected_path {
            if let Some(index) = self.entries.iter().position(|e| e.path == path) {
                self.selected = index;
            }
        }
    }

    fn apply_view(&mut self) {
        let query = self.query.trim().to_lowercase();
        self.entries = self
            .all_entries
            .iter()
            .filter(|entry| self.filter.matches(entry))
            .filter(|entry| {
                query.is_empty()
                    || entry.title.to_lowercase().contains(&query)
                    || entry.author.to_lowercase().contains(&query)
                    || entry.source.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();

        self.entries.sort_by(|a, b| match self.sort {
            SortOrder::Downloaded => b
                .downloaded
                .cmp(&a.downloaded)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())),
            SortOrder::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            SortOrder::Source => a
                .source
                .to_lowercase()
                .cmp(&b.source.to_lowercase())
                .then_with(|| b.downloaded.cmp(&a.downloaded)),
            SortOrder::Status => status_rank(a.status)
                .cmp(&status_rank(b.status))
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())),
        });
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Transition {
        match &mut self.mode {
            Mode::Searching => {
                match key.code {
                    KeyCode::Enter => self.mode = Mode::Normal,
                    KeyCode::Esc => {
                        self.query.clear();
                        self.mode = Mode::Normal;
                    }
                    KeyCode::Backspace => {
                        self.query.pop();
                    }
                    KeyCode::Char(c) => self.query.push(c),
                    _ => {}
                }
                self.selected = 0;
                self.apply_view();
                Transition::None
            }
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
            KeyCode::Char('/') => {
                self.query.clear();
                self.mode = Mode::Searching;
                self.apply_view();
            }
            KeyCode::Char('o') => {
                self.sort = self.sort.next();
                self.selected = 0;
                self.apply_view();
            }
            KeyCode::Char('v') => {
                self.filter = self.filter.next();
                self.selected = 0;
                self.apply_view();
            }
            KeyCode::Char('z') => {
                self.query.clear();
                self.filter = Filter::All;
                self.selected = 0;
                self.apply_view();
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
        let [
            title_area,
            toolbar_area,
            list_area,
            detail_area,
            footer_area,
        ] = area.layout(&Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(2),
            Constraint::Length(1),
        ]));

        Line::from(vec![
            "Crosstui".fg(t.accent()).bold(),
            " puzzle library ".bold(),
            format!("{} / {}", self.entries.len(), self.all_entries.len()).fg(t.muted),
        ])
        .centered()
        .render(title_area, buf);

        Line::from(vec![
            " Search ".fg(t.sel_fg()).bg(t.sel_bg()).bold(),
            format!(
                " {}  ",
                if self.query.is_empty() {
                    "Any title, author, or source"
                } else {
                    &self.query
                }
            )
            .into(),
            " Filter ".fg(t.sel_fg()).bg(t.sel_bg()).bold(),
            format!(" {}  ", self.filter.label()).into(),
            " Sort ".fg(t.sel_fg()).bg(t.sel_bg()).bold(),
            format!(" {}", self.sort.label()).into(),
        ])
        .render(toolbar_area, buf);

        if self.entries.is_empty() {
            let message = if self.all_entries.is_empty() {
                "No puzzles yet. Press 's' to visit sources and download some."
            } else {
                "No puzzles match this view. Press 'z' to clear search and filters."
            };
            Paragraph::new(message.fg(t.muted))
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
                        .title(" ★  Status       Downloaded   Source               Size    Puzzle ")
                        .padding(Padding::horizontal(1)),
                );
            StatefulWidget::render(list, list_area, buf, &mut list_state);
        }

        let detail = self.selected_entry().map_or_else(Line::default, |entry| {
            Line::from(vec![
                entry.title.clone().bold(),
                if entry.author.is_empty() {
                    Span::raw("")
                } else {
                    format!(" by {}", entry.author).fg(t.muted)
                },
                "  •  ".fg(t.muted),
                entry.source.clone().fg(t.accent()),
                format!("  •  downloaded {}", entry.downloaded).fg(t.muted),
            ])
        });
        detail.render(detail_area, buf);

        let footer = match &self.mode {
            Mode::Searching => Line::from(vec![
                "Search: ".bold(),
                self.query.clone().into(),
                "_  ".fg(t.accent()),
                "Enter apply  Esc clear".fg(t.muted),
            ]),
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
                "↑/↓ move  Enter open  / search  v filter  o sort  z clear  f favorite  s sources  c crosshare  t themes  q quit"
                    .fg(t.muted),
            ),
        };
        footer.render(footer_area, buf);
    }
}

fn status_rank(status: Status) -> u8 {
    match status {
        Status::Unsolved => 0,
        Status::InProgress => 1,
        Status::Complete => 2,
        Status::Unreadable => 3,
    }
}

fn entry_line(entry: &Entry) -> Line<'static> {
    let t = theme::current();
    let star = if entry.favorite { "★ " } else { "  " };
    let status_span: Span<'static> = {
        let label = format!("{:<13}", entry.status.label());
        match entry.status {
            Status::Complete => label.fg(t.green),
            Status::InProgress => label.fg(t.yellow),
            Status::Unreadable => label.fg(t.red),
            Status::Unsolved => label.fg(t.muted),
        }
    };

    let mut spans = vec![
        star.fg(t.yellow),
        status_span,
        format!("{:<13}", entry.downloaded).fg(t.muted),
        format!("{:<21}", truncate(&entry.source, 19)).fg(t.accent()),
        format!("{:<8}", entry.size).fg(t.muted),
        entry.title.clone().into(),
    ];
    if !entry.author.is_empty() {
        spans.push(format!(" — {}", entry.author).fg(t.muted));
    }
    Line::from(spans)
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let mut text: String = value.chars().take(max_chars.saturating_sub(1)).collect();
        text.push('…');
        text
    }
}
