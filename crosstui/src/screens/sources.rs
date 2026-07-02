use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListState, Padding, Paragraph, StatefulWidget, Widget};
use ratatui_macros::line;

use crate::app::Transition;
use crate::download::{self, SOURCES};
use crate::theme;

#[derive(Clone)]
enum DownloadState {
    Idle,
    Done,
    Failed(String),
}

impl From<Result<(), String>> for DownloadState {
    fn from(r: Result<(), String>) -> Self {
        match r {
            Ok(()) => DownloadState::Done,
            Err(e) => DownloadState::Failed(e),
        }
    }
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
    states: Vec<DownloadState>,
    mode: Mode,
    /// Result of the last URL download, shown beneath the list.
    url_state: DownloadState,
}

impl SourcesScreen {
    pub fn new() -> Self {
        Self {
            selected: 0,
            states: vec![DownloadState::Idle; SOURCES.len()],
            mode: Mode::Normal,
            url_state: DownloadState::Idle,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, library_dir: &Path) -> Transition {
        match &mut self.mode {
            Mode::Date(buf) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let date = buf.trim().to_string();
                    self.mode = Mode::Normal;
                    let date = (!date.is_empty()).then_some(date.as_str());
                    // ponytail: blocking download; UI is frozen until it returns.
                    self.states[self.selected] =
                        download::download(library_dir, SOURCES[self.selected].keyword, date)
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
                    self.url_state = download::download_url(library_dir, &url).into();
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
                KeyCode::Enter => {
                    // ponytail: blocking download; UI is frozen until it returns.
                    self.states[self.selected] =
                        download::download(library_dir, SOURCES[self.selected].keyword, None)
                            .into();
                }
                KeyCode::Char('d') => self.mode = Mode::Date(String::new()),
                KeyCode::Char('u') => self.mode = Mode::Url(String::new()),
                _ => {}
            },
        }
        Transition::None
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let t = theme::current();
        let [title_area, list_area, prompt_area, footer_area] = area.layout(&Layout::vertical([
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(2),
            Constraint::Length(1),
        ]));

        Line::from("Download puzzles".bold().fg(t.accent()))
            .centered()
            .render(title_area, buf);

        let mut list_state = ListState::default();
        list_state.select(Some(self.selected));
        let rows = SOURCES.iter().enumerate().map(|(i, source)| {
            let status = match &self.states[i] {
                DownloadState::Idle => "".fg(t.muted),
                DownloadState::Done => "✓ downloaded".fg(t.green),
                DownloadState::Failed(e) => format!("✗ {e}").fg(t.red),
            };
            line![
                format!("{:<22}", source.name),
                format!("{:<22}", source.about).fg(t.muted),
                status,
            ]
        });

        let list = List::new(rows)
            .highlight_style(Style::new().fg(t.sel_fg()).bg(t.sel_bg()).bold())
            .block(
                Block::bordered()
                    .border_style(Style::new().fg(t.muted))
                    .title(" Sources ")
                    .padding(Padding::uniform(1)),
            );
        StatefulWidget::render(list, list_area, buf, &mut list_state);

        Paragraph::new(self.prompt_line(&t)).render(prompt_area, buf);
        Paragraph::new(self.footer_line(&t)).render(footer_area, buf);
    }

    /// The prompt/status line under the list: an active input field, the last
    /// URL result, or a hint about older puzzles.
    fn prompt_line(&self, t: &theme::Theme) -> Line<'static> {
        match &self.mode {
            Mode::Date(buf) => line![
                format!("Date for {} (YYYY-MM-DD): ", SOURCES[self.selected].name),
                buf.clone(),
                "_".fg(t.accent()),
            ],
            Mode::Url(buf) => line!["Puzzle URL: ", buf.clone(), "_".fg(t.accent())],
            Mode::Normal => match &self.url_state {
                DownloadState::Done => "URL: ✓ downloaded".fg(t.green).into(),
                DownloadState::Failed(e) => format!("URL: ✗ {e}").fg(t.red).into(),
                DownloadState::Idle => {
                    "Tip: d fetches an older puzzle by date; u downloads from a URL."
                        .fg(t.muted)
                        .into()
                }
            },
        }
    }

    fn footer_line(&self, t: &theme::Theme) -> Line<'static> {
        match self.mode {
            Mode::Normal => {
                "↑/↓ move   Enter latest   d by date   u from URL   b back   q quit".fg(t.muted)
            }
            _ => "Enter download   Esc cancel".fg(t.muted),
        }
        .into()
    }
}
