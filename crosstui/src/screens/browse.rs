use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListState, Padding, Paragraph, StatefulWidget, Widget};

use crate::app::Transition;
use crate::download::{self, CROSSHARE_FEEDS, CrossharePuzzle};
use crate::theme;

/// Browses Crosshare's public listings and downloads puzzles into the library.
pub struct BrowseScreen {
    feed: usize,
    page: usize,
    puzzles: Vec<CrossharePuzzle>,
    selected: usize,
    /// `Err` if the current feed page failed to load.
    load_error: Option<String>,
    /// Outcome of the last download: `Ok(title)` or `Err(message)`.
    notice: Option<Result<String, String>>,
}

impl BrowseScreen {
    pub fn new() -> Self {
        let mut screen = Self {
            feed: 0,
            page: 1,
            puzzles: Vec::new(),
            selected: 0,
            load_error: None,
            notice: None,
        };
        screen.load();
        screen
    }

    /// (Re)loads the current feed page. Blocking, like the other download paths.
    fn load(&mut self) {
        self.selected = 0;
        match download::list_crosshare(&CROSSHARE_FEEDS[self.feed], self.page) {
            Ok(puzzles) => {
                self.puzzles = puzzles;
                self.load_error = None;
            }
            Err(e) => {
                self.puzzles.clear();
                self.load_error = Some(e);
            }
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, library_dir: &Path) -> Transition {
        match key.code {
            KeyCode::Char('q') => return Transition::Quit,
            KeyCode::Esc | KeyCode::Char('b') => return Transition::ToLibrary,
            KeyCode::Down | KeyCode::Char('j') if !self.puzzles.is_empty() => {
                self.selected = (self.selected + 1).min(self.puzzles.len() - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Tab => {
                self.feed = (self.feed + 1) % CROSSHARE_FEEDS.len();
                self.page = 1;
                self.load();
            }
            KeyCode::Right | KeyCode::Char('n') if CROSSHARE_FEEDS[self.feed].paged => {
                self.page += 1;
                self.load();
            }
            KeyCode::Left | KeyCode::Char('p')
                if CROSSHARE_FEEDS[self.feed].paged && self.page > 1 =>
            {
                self.page -= 1;
                self.load();
            }
            KeyCode::Enter => {
                if let Some(p) = self.puzzles.get(self.selected) {
                    // ponytail: blocking download; UI is frozen until it returns.
                    self.notice = Some(
                        download::download_crosshare(library_dir, &p.id, &p.title)
                            .map(|()| p.title.clone()),
                    );
                }
            }
            _ => {}
        }
        Transition::None
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let t = theme::current();
        let [title_area, list_area, notice_area, footer_area] = area.layout(&Layout::vertical([
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]));

        let feed = &CROSSHARE_FEEDS[self.feed];
        let heading = if feed.paged {
            format!("Crosshare · {} · page {}", feed.name, self.page)
        } else {
            format!("Crosshare · {}", feed.name)
        };
        Line::from(heading.bold().fg(t.accent()))
            .centered()
            .render(title_area, buf);

        let block = Block::bordered()
            .border_style(Style::new().fg(t.muted))
            .title(" Puzzles ")
            .padding(Padding::uniform(1));
        if let Some(e) = &self.load_error {
            Paragraph::new(format!("Couldn't load puzzles: {e}").fg(t.red))
                .block(block)
                .render(list_area, buf);
        } else {
            let mut list_state = ListState::default();
            list_state.select(Some(self.selected));
            let rows = self.puzzles.iter().map(|p| Line::from(p.title.clone()));
            let list = List::new(rows)
                .highlight_style(Style::new().fg(t.sel_fg()).bg(t.sel_bg()).bold())
                .block(block);
            StatefulWidget::render(list, list_area, buf, &mut list_state);
        }

        let notice = match &self.notice {
            Some(Ok(title)) => format!("✓ downloaded \"{title}\"").fg(t.green),
            Some(Err(e)) => format!("✗ {e}").fg(t.red),
            None => "Enter downloads the selected puzzle into your library.".fg(t.muted),
        };
        Paragraph::new(notice).render(notice_area, buf);

        Paragraph::new(
            "↑/↓ move   Enter download   Tab feed   n/p page   b back   q quit".fg(t.muted),
        )
        .render(footer_area, buf);
    }
}
