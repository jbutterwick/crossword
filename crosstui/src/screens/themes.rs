use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListState, Padding, Paragraph, StatefulWidget, Widget};
use ratatui_macros::line;

use crate::app::Transition;
use crate::theme::{self, Theme, ThemeEntry};

/// Theme picker. Moving the selection previews the theme live (the whole UI
/// re-themes); Enter keeps and persists it, Esc reverts to the previous theme.
pub struct ThemesScreen {
    dir: PathBuf,
    entries: Vec<ThemeEntry>,
    selected: usize,
    /// Theme active on entry, restored if the user cancels.
    previous: Theme,
}

impl ThemesScreen {
    pub fn new(dir: &Path) -> Self {
        let entries = theme::load(dir);
        let selected = theme::load_selected(dir)
            .and_then(|name| entries.iter().position(|e| e.name == name))
            .unwrap_or(0);

        let screen = Self {
            dir: dir.to_path_buf(),
            entries,
            selected,
            previous: theme::current(),
        };
        screen.preview();
        screen
    }

    /// Applies the highlighted theme so the user sees it immediately.
    fn preview(&self) {
        if let Some(entry) = self.entries.get(self.selected) {
            theme::set(entry.theme);
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Transition {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                theme::set(self.previous); // cancel: revert preview
                return Transition::ToLibrary;
            }
            KeyCode::Enter => {
                if let Some(entry) = self.entries.get(self.selected) {
                    theme::save_selected(&self.dir, &entry.name);
                }
                return Transition::ToLibrary;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
                self.preview();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.preview();
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

        Line::from("Themes".bold().fg(t.accent()))
            .centered()
            .render(title_area, buf);

        let mut list_state = ListState::default();
        list_state.select(Some(self.selected));
        let rows = self.entries.iter().map(|entry| {
            // A row of swatches previews each theme's palette inline.
            let p = &entry.theme;
            line![
                format!("{:<22}", entry.name),
                "██".fg(p.bg),
                "██".fg(p.fg),
                "██".fg(p.red),
                "██".fg(p.green),
                "██".fg(p.yellow),
                "██".fg(p.blue),
                "██".fg(p.magenta),
            ]
        });
        let list = List::new(rows)
            .highlight_style(Style::new().fg(t.sel_fg()).bg(t.sel_bg()).bold())
            .block(
                Block::bordered()
                    .border_style(Style::new().fg(t.muted))
                    .title(" Select a theme ")
                    .padding(Padding::uniform(1)),
            );
        StatefulWidget::render(list, list_area, buf, &mut list_state);

        Paragraph::new(
            "↑/↓ preview   Enter apply   Esc cancel   (edit themes.toml to customize)".fg(t.muted),
        )
        .render(footer_area, buf);
    }
}
