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

pub struct SourcesScreen {
    selected: usize,
    states: Vec<DownloadState>,
}

impl SourcesScreen {
    pub fn new() -> Self {
        Self {
            selected: 0,
            states: vec![DownloadState::Idle; SOURCES.len()],
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, library_dir: &Path) -> Transition {
        match key.code {
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
                self.states[self.selected] = match download::download(
                    library_dir,
                    SOURCES[self.selected].keyword,
                ) {
                    Ok(()) => DownloadState::Done,
                    Err(e) => DownloadState::Failed(e),
                };
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
            line![format!("{:<18}", source.name), status]
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

        Paragraph::new("↑/↓ move   Enter download   b/Esc back   q quit".fg(t.muted))
            .render(footer_area, buf);
    }
}
