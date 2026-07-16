use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListState, Padding, Paragraph, StatefulWidget, Widget};
use ratatui_macros::line;

use crate::app::Transition;
use crate::download::{self, FetchedPuzzle, SOURCES};
use crate::storage;
use crate::theme;

#[derive(Clone)]
enum DownloadState {
    Idle,
    Done,
    Failed(String),
}

impl From<Result<(), String>> for DownloadState {
    fn from(result: Result<(), String>) -> Self {
        match result {
            Ok(()) => DownloadState::Done,
            Err(error) => DownloadState::Failed(error),
        }
    }
}

#[derive(Clone)]
struct PuzzleSummary {
    title: String,
    author: String,
}

impl From<&FetchedPuzzle> for PuzzleSummary {
    fn from(puzzle: &FetchedPuzzle) -> Self {
        Self {
            title: puzzle.title.clone(),
            author: puzzle.author.clone(),
        }
    }
}

enum TodayState {
    Checking,
    Available(FetchedPuzzle),
    Owned(PuzzleSummary),
    Downloading,
    Failed(String),
}

enum WorkerEvent {
    Preview(usize, Result<FetchedPuzzle, String>),
    Downloaded(usize, Result<FetchedPuzzle, String>),
}

/// What text field, if any, is currently being typed into.
enum Mode {
    Normal,
    /// Typing a `YYYY-MM-DD` date to fetch an older puzzle for the selected outlet.
    Date(String),
    /// Typing an arbitrary puzzle URL to download.
    Url(String),
}

pub struct SourcesScreen {
    selected: usize,
    states: Vec<TodayState>,
    mode: Mode,
    /// Result of the last URL/date download, shown beneath the list.
    notice: DownloadState,
    today: String,
    library_dir: PathBuf,
    known_puzzles: Vec<PuzzleSummary>,
    tx: Sender<WorkerEvent>,
    rx: Receiver<WorkerEvent>,
    bulk_active: bool,
}

impl SourcesScreen {
    pub fn new(library_dir: &Path) -> Self {
        let today = download::today();
        let known_puzzles: Vec<PuzzleSummary> = storage::scan(library_dir)
            .into_iter()
            .map(|entry| PuzzleSummary {
                title: entry.title,
                author: entry.author,
            })
            .collect();
        let (tx, rx) = mpsc::channel();
        let mut states = Vec::with_capacity(SOURCES.len());
        let mut previews = VecDeque::new();

        for (index, source) in SOURCES.iter().enumerate() {
            if let Some(entry) = storage::source_puzzle_on(library_dir, source.keyword, &today) {
                states.push(TodayState::Owned(PuzzleSummary {
                    title: entry.title,
                    author: entry.author,
                }));
            } else {
                states.push(TodayState::Checking);
                previews.push_back((index, source.keyword));
            }
        }

        // A small worker pool keeps the TUI responsive without launching every
        // outlet scraper at once. Titles and bylines appear as each page loads.
        let previews = Arc::new(Mutex::new(previews));
        for _ in 0..previews.lock().map_or(0, |tasks| tasks.len().min(4)) {
            let tx = tx.clone();
            let previews = Arc::clone(&previews);
            std::thread::spawn(move || {
                loop {
                    let task = previews
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pop_front();
                    let Some((index, keyword)) = task else {
                        break;
                    };
                    let _ = tx.send(WorkerEvent::Preview(
                        index,
                        download::fetch_source(keyword, None),
                    ));
                }
            });
        }

        Self {
            selected: 0,
            states,
            mode: Mode::Normal,
            notice: DownloadState::Idle,
            today,
            library_dir: library_dir.to_path_buf(),
            known_puzzles,
            tx,
            rx,
            bulk_active: false,
        }
    }

    fn drain_workers(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                WorkerEvent::Preview(index, result) => {
                    // A download request takes precedence over a late preview.
                    if !matches!(self.states[index], TodayState::Checking) {
                        continue;
                    }
                    self.states[index] = match result {
                        Ok(puzzle) => {
                            if let Some(owned) = self.known_puzzles.iter().find(|known| {
                                known.title == puzzle.title && known.author == puzzle.author
                            }) {
                                TodayState::Owned(owned.clone())
                            } else {
                                TodayState::Available(puzzle)
                            }
                        }
                        Err(error) => TodayState::Failed(error),
                    };
                }
                WorkerEvent::Downloaded(index, result) => {
                    self.states[index] = match result {
                        Ok(puzzle) => TodayState::Owned(PuzzleSummary::from(&puzzle)),
                        Err(error) => TodayState::Failed(error),
                    };
                }
            }
        }

        if self.bulk_active
            && !self
                .states
                .iter()
                .any(|state| matches!(state, TodayState::Downloading))
        {
            self.bulk_active = false;
            let failed = self
                .states
                .iter()
                .filter(|state| matches!(state, TodayState::Failed(_)))
                .count();
            self.notice = if failed == 0 {
                DownloadState::Done
            } else {
                DownloadState::Failed(format!("finished; {failed} sources unavailable"))
            };
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, library_dir: &Path) -> Transition {
        self.drain_workers();
        match &mut self.mode {
            Mode::Date(buf) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let date = buf.trim().to_string();
                    self.mode = Mode::Normal;
                    let date = (!date.is_empty()).then_some(date.as_str());
                    self.notice =
                        download::download(library_dir, SOURCES[self.selected].keyword, date)
                            .map(|_| ())
                            .into();
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => buf.push(c),
                _ => {}
            },
            Mode::Url(buf) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let url = buf.clone();
                    self.mode = Mode::Normal;
                    self.notice = download::download_url(library_dir, &url).into();
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => buf.push(c),
                _ => {}
            },
            Mode::Normal => match key.code {
                KeyCode::Char('q') => return Transition::Quit,
                KeyCode::Esc | KeyCode::Char('b') => return Transition::ToLibrary,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selected = (self.selected + 1).min(SOURCES.len().saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.selected = self.selected.saturating_sub(1);
                }
                KeyCode::Enter => self.start_download(self.selected),
                KeyCode::Char('a') | KeyCode::Char('A') => self.download_all(),
                KeyCode::Char('d') => self.mode = Mode::Date(String::new()),
                KeyCode::Char('u') => self.mode = Mode::Url(String::new()),
                _ => {}
            },
        }
        Transition::None
    }

    fn start_download(&mut self, index: usize) {
        if matches!(
            self.states[index],
            TodayState::Owned(_) | TodayState::Downloading
        ) {
            return;
        }
        let cached = match &self.states[index] {
            TodayState::Available(puzzle) => Some(puzzle.clone()),
            _ => None,
        };
        self.states[index] = TodayState::Downloading;
        self.notice = DownloadState::Idle;
        let tx = self.tx.clone();
        let dir = self.library_dir.clone();
        let date = self.today.clone();
        let source = &SOURCES[index];
        std::thread::spawn(move || {
            let result = cached
                .map(Ok)
                .unwrap_or_else(|| download::fetch_source(source.keyword, None))
                .and_then(|puzzle| {
                    download::save_source(&dir, source, &date, &puzzle).map(|_| puzzle)
                });
            let _ = tx.send(WorkerEvent::Downloaded(index, result));
        });
    }

    fn download_all(&mut self) {
        if self.bulk_active {
            return;
        }
        self.notice = DownloadState::Idle;
        self.bulk_active = true;
        let mut tasks = VecDeque::new();
        for index in 0..SOURCES.len() {
            if matches!(
                self.states[index],
                TodayState::Owned(_) | TodayState::Downloading
            ) {
                continue;
            }
            let cached = match &self.states[index] {
                TodayState::Available(puzzle) => Some(puzzle.clone()),
                _ => None,
            };
            self.states[index] = TodayState::Downloading;
            tasks.push_back((index, cached));
        }

        if tasks.is_empty() {
            if !self
                .states
                .iter()
                .any(|state| matches!(state, TodayState::Downloading))
            {
                self.bulk_active = false;
                self.notice = DownloadState::Done;
            }
            return;
        }

        let worker_count = tasks.len().min(4);
        let tasks = Arc::new(Mutex::new(tasks));
        for _ in 0..worker_count {
            let tasks = Arc::clone(&tasks);
            let tx = self.tx.clone();
            let dir = self.library_dir.clone();
            let date = self.today.clone();
            std::thread::spawn(move || {
                loop {
                    let task = tasks.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
                    let Some((index, cached)) = task else {
                        break;
                    };
                    let source = &SOURCES[index];
                    let result = cached
                        .map(Ok)
                        .unwrap_or_else(|| download::fetch_source(source.keyword, None))
                        .and_then(|puzzle| {
                            download::save_source(&dir, source, &date, &puzzle).map(|_| puzzle)
                        });
                    let _ = tx.send(WorkerEvent::Downloaded(index, result));
                }
            });
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.drain_workers();
        let t = theme::current();
        let [title_area, action_area, list_area, prompt_area, footer_area] =
            area.layout(&Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Fill(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ]));

        Line::from(vec![
            "Puzzle sources".bold().fg(t.accent()),
            format!("  •  {}", self.today).fg(t.muted),
        ])
        .centered()
        .render(title_area, buf);

        let missing = self
            .states
            .iter()
            .filter(|state| !matches!(state, TodayState::Owned(_)))
            .count();
        let action = if self.bulk_active {
            " Downloading today's puzzles… "
        } else if missing == 0 {
            " ✓ You have today's puzzle from every available source "
        } else {
            " [ A ]  Download today's puzzle from every source "
        };
        Paragraph::new(
            action
                .bold()
                .fg(if self.bulk_active { t.yellow } else { t.green }),
        )
        .centered()
        .block(Block::bordered().border_style(Style::new().fg(t.accent())))
        .render(action_area, buf);

        let mut list_state = ListState::default();
        list_state.select(Some(self.selected));
        let puzzle_width = (list_area.width as usize).saturating_sub(42).max(12);
        let rows = SOURCES.iter().enumerate().map(|(index, source)| {
            let (puzzle, puzzle_color, status) = match &self.states[index] {
                TodayState::Checking => {
                    (source.about.to_string(), t.muted, "◌ checking…".fg(t.muted))
                }
                TodayState::Available(info) => (
                    puzzle_label(&info.title, &info.author, puzzle_width.saturating_sub(2)),
                    t.fg,
                    "↓ ready".fg(t.accent()),
                ),
                TodayState::Owned(info) => (
                    puzzle_label(&info.title, &info.author, puzzle_width.saturating_sub(2)),
                    t.fg,
                    "✓ in library".fg(t.green),
                ),
                TodayState::Downloading => (
                    source.about.to_string(),
                    t.muted,
                    "↓ downloading…".fg(t.yellow),
                ),
                TodayState::Failed(_) => {
                    (source.about.to_string(), t.muted, "— unavailable".fg(t.red))
                }
            };
            line![
                format!("{:<22}", source.name),
                format!("{puzzle:<puzzle_width$}").fg(puzzle_color),
                status
            ]
        });

        let list = List::new(rows)
            .highlight_style(Style::new().fg(t.sel_fg()).bg(t.sel_bg()).bold())
            .block(
                Block::bordered()
                    .border_style(Style::new().fg(t.muted))
                    .title(" Source                 Today’s puzzle / author                              Status ")
                    .padding(Padding::horizontal(1)),
            );
        StatefulWidget::render(list, list_area, buf, &mut list_state);

        Paragraph::new(self.prompt_line(&t)).render(prompt_area, buf);
        Paragraph::new(self.footer_line(&t)).render(footer_area, buf);
    }

    fn prompt_line(&self, t: &theme::Theme) -> Line<'static> {
        match &self.mode {
            Mode::Date(buf) => line![
                format!("Date for {} (YYYY-MM-DD): ", SOURCES[self.selected].name),
                buf.clone(),
                "_".fg(t.accent()),
            ],
            Mode::Url(buf) => line!["Puzzle URL: ", buf.clone(), "_".fg(t.accent())],
            Mode::Normal => match &self.notice {
                DownloadState::Done => "✓ Download complete".fg(t.green).into(),
                DownloadState::Failed(error) => format!("✗ {error}").fg(t.red).into(),
                DownloadState::Idle => match &self.states[self.selected] {
                    TodayState::Failed(error) => {
                        format!("{}: {error}", SOURCES[self.selected].name)
                            .fg(t.red)
                            .into()
                    }
                    _ => format!(
                        "{} — {}",
                        SOURCES[self.selected].name, SOURCES[self.selected].about
                    )
                    .fg(t.muted)
                    .into(),
                },
            },
        }
    }

    fn footer_line(&self, t: &theme::Theme) -> Line<'static> {
        match self.mode {
            Mode::Normal => {
                "↑/↓ move  Enter download  A download all  d by date  u URL  b back  q quit"
                    .fg(t.muted)
            }
            _ => "Enter download   Esc cancel".fg(t.muted),
        }
        .into()
    }
}

fn puzzle_label(title: &str, author: &str, max_chars: usize) -> String {
    let title = if title.trim().is_empty() {
        "Untitled crossword"
    } else {
        title.trim()
    };
    let value = if author.trim().is_empty() {
        title.to_string()
    } else {
        format!("{title} — {}", author.trim())
    };
    truncate(&value, max_chars)
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
