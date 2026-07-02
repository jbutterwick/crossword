use std::path::{Path, PathBuf};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use crossword::{Direction, Puzzle, Square, SquareStyle};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListState, Padding, Paragraph, StatefulWidget, Widget, Wrap};
use ratatui_macros::{line, text};

use crate::app::Transition;
use crate::theme;

/// One grid cell's footprint: `w`×`h` characters plus trailing gaps.
#[derive(Clone, Copy)]
struct CellSize {
    w: u16,
    h: u16,
    gap_x: u16,
    gap_y: u16,
}

impl CellSize {
    /// Per-cell stride including the gap to the next cell.
    fn footprint(self) -> (u16, u16) {
        (self.w + self.gap_x, self.h + self.gap_y)
    }
}

// ponytail: tune these presets to taste; ordered small → large.
const ZOOM_LEVELS: &[CellSize] = &[
    CellSize {
        w: 1,
        h: 1,
        gap_x: 0,
        gap_y: 0,
    }, // glyph-only
    CellSize {
        w: 3,
        h: 1,
        gap_x: 1,
        gap_y: 0,
    },
    CellSize {
        w: 5,
        h: 2,
        gap_x: 1,
        gap_y: 1,
    },
    CellSize {
        w: 7,
        h: 3,
        gap_x: 2,
        gap_y: 1,
    }, // ~ the original look
];

enum Zoom {
    /// Auto-scale so the whole grid fits the pane.
    Fit,
    /// A fixed zoom level (index into [`ZOOM_LEVELS`]).
    Manual(usize),
}

/// On-screen geometry of the grid from the last render, used to map a mouse
/// click back to a grid cell.
#[derive(Clone, Copy)]
struct GridView {
    origin_x: u16,
    origin_y: u16,
    /// Per-cell stride (cell size + gap) in columns/rows.
    stride_x: u16,
    stride_y: u16,
    /// Drawn cell size in columns/rows (the clickable part of each stride).
    cell_w: u16,
    cell_h: u16,
    scroll: (usize, usize),
    /// Visible cell counts (rows, cols).
    visible: (usize, usize),
}

/// The puzzle-solving screen. Owns the puzzle and the path it was loaded from
/// (so progress can be saved back).
pub struct SolveScreen {
    puzzle: Puzzle,
    path: PathBuf,
    zoom: Zoom,
    /// Top-left visible cell `(row, col)`; persists so scrolling is edge-triggered.
    scroll: (usize, usize),
    /// Largest level that fully fits, recomputed each render (used by `+` from Fit).
    fit_level: usize,
    /// When the screen opened; drives the highlighted-clue marquee.
    opened_at: Instant,
    /// When the puzzle was first seen solved; drives the confetti burst.
    solved_at: Option<Instant>,
    /// Grid geometry from the last render, for hit-testing mouse clicks.
    grid_view: Option<GridView>,
}

/// Milliseconds the marquee dwells on each character position. Lower = faster.
const MARQUEE_STEP_MS: u128 = 220;

/// How long the confetti rains after a puzzle is solved.
const CONFETTI_MS: u128 = 5000;
const CONFETTI_CHARS: &[char] = &['*', '+', 'o', '.', '✦', '❉'];

impl SolveScreen {
    pub fn new(puzzle: Puzzle, path: PathBuf) -> Self {
        Self {
            puzzle,
            path,
            zoom: Zoom::Fit,
            scroll: (0, 0),
            fit_level: ZOOM_LEVELS.len() - 1,
            opened_at: Instant::now(),
            solved_at: None,
            grid_view: None,
        }
    }

    pub fn puzzle(&self) -> &Puzzle {
        &self.puzzle
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Handles the key events and updates the state of the puzzle.
    pub fn on_key(&mut self, key: KeyEvent) -> Transition {
        match key.code {
            KeyCode::Esc => return Transition::ToLibrary,
            KeyCode::Up => self.puzzle.cursor_up(),
            KeyCode::Down => self.puzzle.cursor_down(),
            KeyCode::Left => self.puzzle.cursor_left(),
            KeyCode::Right => self.puzzle.cursor_right(),
            KeyCode::Home => self.puzzle.cursor_to_word_start(),
            KeyCode::End => self.puzzle.cursor_to_word_end(),
            KeyCode::Backspace => {
                self.puzzle.erase_letter();
                self.puzzle.backup_cursor();
            }
            KeyCode::Delete => self.puzzle.erase_letter(),
            KeyCode::Tab => self.puzzle.advance_cursor_to_next_word(),
            KeyCode::BackTab => self.puzzle.retreat_cursor_to_prev_word(),
            KeyCode::Char(' ') => self.puzzle.swap_cursor_direction(),
            KeyCode::Char('+') | KeyCode::Char('=') => self.zoom_in(),
            KeyCode::Char('-') | KeyCode::Char('_') => self.zoom_out(),
            KeyCode::Char('0') => self.zoom = Zoom::Fit,
            KeyCode::Char(letter) => {
                if letter.is_ascii_alphabetic() {
                    self.puzzle.add_letter(letter);
                    self.puzzle.move_cursor_to_next_empty_in_current_word();
                }
            }
            _ => {}
        }
        Transition::None
    }

    /// Left-click jumps the cursor to the clicked square (the highlighted clue
    /// follows, since it's derived from the cursor).
    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }
        let Some(view) = self.grid_view else {
            return;
        };
        if mouse.column < view.origin_x || mouse.row < view.origin_y {
            return;
        }
        let rel_x = (mouse.column - view.origin_x) as usize;
        let rel_y = (mouse.row - view.origin_y) as usize;

        // Ignore clicks that land in the gap between cells.
        if rel_x % view.stride_x as usize >= view.cell_w as usize
            || rel_y % view.stride_y as usize >= view.cell_h as usize
        {
            return;
        }
        let col_in_view = rel_x / view.stride_x as usize;
        let row_in_view = rel_y / view.stride_y as usize;
        if row_in_view >= view.visible.0 || col_in_view >= view.visible.1 {
            return;
        }
        self.puzzle
            .move_cursor_to((view.scroll.0 + row_in_view, view.scroll.1 + col_in_view));
    }

    fn current_level(&self) -> usize {
        match self.zoom {
            Zoom::Fit => self.fit_level,
            Zoom::Manual(i) => i,
        }
    }

    fn zoom_in(&mut self) {
        self.zoom = Zoom::Manual((self.current_level() + 1).min(ZOOM_LEVELS.len() - 1));
    }

    fn zoom_out(&mut self) {
        self.zoom = Zoom::Manual(self.current_level().saturating_sub(1));
    }

    /// Bounds `(row_lo, row_hi, col_lo, col_hi)` of the current word.
    fn word_span(&self) -> (usize, usize, usize, usize) {
        let (r, c) = self.puzzle.cursor_pos();
        let grid = self.puzzle.grid();
        let is_black = |pos| matches!(grid.get(pos), Square::Black);
        match self.puzzle.cursor_direction() {
            Direction::Across => {
                let mut lo = c;
                while lo > 0 && !is_black((r, lo - 1)) {
                    lo -= 1;
                }
                let mut hi = c;
                while hi + 1 < grid.width() && !is_black((r, hi + 1)) {
                    hi += 1;
                }
                (r, r, lo, hi)
            }
            Direction::Down => {
                let mut lo = r;
                while lo > 0 && !is_black((lo - 1, c)) {
                    lo -= 1;
                }
                let mut hi = r;
                while hi + 1 < grid.height() && !is_black((hi + 1, c)) {
                    hi += 1;
                }
                (lo, hi, c, c)
            }
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let t = theme::current();
        let vertical = Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]);
        let [title_area, main_area] = area.layout(&vertical);

        let title = text![
            "",
            line!["Crosstui".fg(t.accent()), ": ", self.puzzle.title()]
                .bold()
                .centered(),
        ];
        title.render(title_area, buf);

        let horizontal = Layout::horizontal([Constraint::Length(45), Constraint::Percentage(100)]);
        let [left_area, puzzle_area] = main_area.layout(&horizontal);

        self.render_grid(puzzle_area, buf);

        if self.puzzle.is_solved() {
            let solved_at = *self.solved_at.get_or_insert_with(Instant::now);
            let elapsed = solved_at.elapsed().as_millis();
            if elapsed < CONFETTI_MS {
                render_confetti(puzzle_area, buf, elapsed);
            }
        } else {
            self.solved_at = None;
        }

        let layout = Layout::vertical([
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Fill(1),
            Constraint::Fill(1),
            Constraint::Length(8),
        ]);
        let [
            instructions_area,
            current_clue_area,
            across_clue_area,
            down_clue_area,
            metadata_area,
        ] = left_area.layout(&layout);

        let instructions = line![
            "Instructions: ".bold(),
            "Arrows/space/tab to navigate. +/- to zoom, 0 to fit. Escape returns to the library."
                .fg(t.muted),
        ];
        Paragraph::new(instructions)
            .wrap(Wrap::default())
            .render(instructions_area, buf);

        if self.puzzle.is_solved() {
            Paragraph::new("You solved it!")
                .block(
                    Block::bordered()
                        .title(Line::from(" Congratulations! ").centered())
                        .padding(Padding::uniform(2)),
                )
                .render(current_clue_area, buf)
        } else {
            let (num, direction) = self.puzzle.current_clue_identifier();
            Paragraph::new(line![
                format!("{}{}", num, direction.to_char()).fg(t.clue_number()),
                ". ",
                self.puzzle.current_clue()
            ])
            .wrap(Wrap::default())
            .render(current_clue_area, buf);
        }

        let marquee_step = (self.opened_at.elapsed().as_millis() / MARQUEE_STEP_MS) as usize;
        ClueList::new(&self.puzzle, Direction::Across, marquee_step).render(across_clue_area, buf);
        ClueList::new(&self.puzzle, Direction::Down, marquee_step).render(down_clue_area, buf);

        let mut metadata: Vec<Line> = vec!["".into()];
        let author = self.puzzle.author();
        if !author.is_empty() {
            metadata.push("".into());
            metadata.push(Line::from(vec!["Author: ".bold(), author.into()]))
        }
        let notes = self.puzzle.notes();
        if !notes.is_empty() {
            metadata.push("".into());
            metadata.push(notes.into());
        }
        let copyright = self.puzzle.copyright();
        if !copyright.is_empty() {
            metadata.push("".into());
            metadata.push(copyright.into());
        }

        Paragraph::new(metadata)
            .wrap(Wrap::default())
            .render(metadata_area, buf);
    }

    /// Renders the grid into `area`, choosing the cell size, updating the
    /// edge-triggered scroll offset, and clipping to the pane.
    fn render_grid(&mut self, area: Rect, buf: &mut Buffer) {
        let (gw, gh) = {
            let g = self.puzzle.grid();
            (g.width(), g.height())
        };
        if area.width == 0 || area.height == 0 {
            return;
        }

        // 1. Cell size: explicit level, or the largest that fully fits.
        let level = match self.zoom {
            Zoom::Manual(i) => i,
            Zoom::Fit => {
                let fit = ZOOM_LEVELS
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, c)| {
                        let (fx, fy) = c.footprint();
                        gw as u16 * fx <= area.width && gh as u16 * fy <= area.height
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.fit_level = fit;
                fit
            }
        };
        let cell = ZOOM_LEVELS[level];
        let (fx, fy) = cell.footprint();

        // 2. How many whole cells fit.
        let vc = ((area.width / fx) as usize).clamp(1, gw);
        let vr = ((area.height / fy) as usize).clamp(1, gh);

        // 3. Follow the current word, collapsing to the cursor if it can't fit.
        let (crow, ccol) = self.puzzle.cursor_pos();
        let (mut r_lo, mut r_hi, mut c_lo, mut c_hi) = self.word_span();
        if r_hi - r_lo + 1 > vr {
            r_lo = crow;
            r_hi = crow;
        }
        if c_hi - c_lo + 1 > vc {
            c_lo = ccol;
            c_hi = ccol;
        }

        // 4. Edge-triggered scroll.
        let row_off = scroll_offset(r_lo, r_hi, vr, gh, self.scroll.0);
        let col_off = scroll_offset(c_lo, c_hi, vc, gw, self.scroll.1);
        self.scroll = (row_off, col_off);

        // 5. Center the grid when it fits the pane; saturating_sub anchors it
        // top-left (offset 0) once it's larger than the pane and we scroll.
        let grid_px_w = gw as u16 * fx;
        let grid_px_h = gh as u16 * fy;
        let origin_x = area.x + area.width.saturating_sub(grid_px_w) / 2;
        let origin_y = area.y + area.height.saturating_sub(grid_px_h) / 2;

        let max_r = (row_off + vr).min(gh);
        let max_c = (col_off + vc).min(gw);
        for r in row_off..max_r {
            for c in col_off..max_c {
                let cell_rect = Rect {
                    x: origin_x + (c - col_off) as u16 * fx,
                    y: origin_y + (r - row_off) as u16 * fy,
                    width: cell.w,
                    height: cell.h,
                };
                let square = self.puzzle.grid().get((r, c));
                let style = to_ratatui_style(self.puzzle.square_style((r, c)));
                render_square(square, style, cell_rect.intersection(area), buf);
            }
        }

        // Remember the geometry so mouse clicks can be mapped back to cells.
        self.grid_view = Some(GridView {
            origin_x,
            origin_y,
            stride_x: fx,
            stride_y: fy,
            cell_w: cell.w,
            cell_h: cell.h,
            scroll: (row_off, col_off),
            visible: (max_r - row_off, max_c - col_off),
        });
    }
}

/// Edge-triggered scroll offset for one axis: keeps the `lo..=hi` span visible
/// in a window of `visible` cells over a line of `len`, moving the previous
/// offset as little as possible.
fn scroll_offset(lo: usize, hi: usize, visible: usize, len: usize, prev: usize) -> usize {
    let max_off = len.saturating_sub(visible);
    let mut off = prev.min(max_off);
    if lo < off {
        off = lo;
    } else if hi >= off + visible {
        off = hi + 1 - visible;
    }
    off.min(max_off)
}

/// Returns the `width`-character window of `text` for marquee position `step`.
/// If `text` fits, it's returned unchanged; otherwise it scrolls left, looping
/// with a small gap so it reads as a repeating marquee.
fn marquee(text: &str, width: usize, step: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if width == 0 || chars.len() <= width {
        return text.to_string();
    }

    const GAP: usize = 4;
    let cycle = chars.len() + GAP;
    let start = step % cycle;
    (0..width)
        .map(|i| {
            let idx = (start + i) % cycle;
            chars.get(idx).copied().unwrap_or(' ') // the gap region
        })
        .collect()
}

/// A widget that renders a list of clues
struct ClueList<'a> {
    puzzle: &'a Puzzle,
    direction: Direction,
    /// Marquee position (advances over time) for the highlighted clue.
    marquee_step: usize,
}

impl<'a> ClueList<'a> {
    pub fn new(puzzle: &'a Puzzle, direction: Direction, marquee_step: usize) -> Self {
        Self {
            puzzle,
            direction,
            marquee_step,
        }
    }
}

impl Widget for ClueList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = theme::current();
        let current_clue_identifier = self.puzzle.current_clue_identifier();
        let cross_clue_identifier = self.puzzle.cross_clue_identifier();

        // Width available for clue text inside the block (borders + padding = 6).
        let inner_width = (area.width as usize).saturating_sub(6);

        let mut list_state = ListState::default();
        let lines = self
            .puzzle
            .clues(self.direction)
            .into_iter()
            .enumerate()
            .map(|(index, (num, clue))| {
                let highlighted = current_clue_identifier == (num, self.direction)
                    || cross_clue_identifier.is_some_and(|cross_clue_identifier| {
                        cross_clue_identifier == (num, self.direction)
                    });
                if highlighted {
                    list_state.select(Some(index));
                }

                // Scroll only the highlighted row, and only when it overflows.
                // The "N. " prefix stays put so the clue number is always visible.
                let prefix_width = num.to_string().len() + 2;
                let clue = if highlighted {
                    marquee(
                        &clue,
                        inner_width.saturating_sub(prefix_width),
                        self.marquee_step,
                    )
                } else {
                    clue
                };
                line![num.to_string().fg(t.clue_number()), ". ", clue]
            })
            .collect::<Vec<_>>();

        let highlight_style = if self.direction == self.puzzle.cursor_direction() {
            Style::new().fg(t.sel_fg()).bg(t.sel_bg()).bold()
        } else {
            Style::new().fg(t.accent()).bg(t.muted).bold()
        };

        let clue_list = List::new(lines).highlight_style(highlight_style).block(
            Block::bordered()
                .border_style(Style::new().fg(t.muted))
                .title(line![" ", self.direction.to_string(), " clues "].centered())
                .padding(Padding {
                    left: 2,
                    right: 2,
                    top: 1,
                    bottom: 1,
                }),
        );

        StatefulWidget::render(clue_list, area, buf, &mut list_state);
    }
}

fn to_ratatui_style(value: SquareStyle) -> Style {
    let t = theme::current();
    let bg = match value {
        SquareStyle::Standard => t.square(),
        SquareStyle::Cursor => t.cursor(),
        SquareStyle::Word => t.word(),
    };
    Style::new().bg(bg).fg(t.square_fg()).bold()
}

/// Rains confetti over `area`. Each particle's column, colour, and glyph are
/// derived from its index via a cheap integer hash, and it falls with time so
/// the whole thing animates off the app's redraw tick — no per-frame state.
fn render_confetti(area: Rect, buf: &mut Buffer, elapsed_ms: u128) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let t = theme::current();
    let colors = [t.green, t.yellow, t.blue, t.magenta, t.red, t.accent()];
    // One row of fall per ~80ms.
    let frame = (elapsed_ms / 80) as u32;
    // Density: roughly one particle per 10 cells.
    let count = (area.width as u32 * area.height as u32) / 10;
    for i in 0..count {
        let h = i.wrapping_mul(2_654_435_761);
        let col = area.x + (h % area.width as u32) as u16;
        let phase = (h >> 8) % area.height as u32;
        let y = area.y + ((frame + phase) % area.height as u32) as u16;
        let ch = CONFETTI_CHARS[(h >> 16) as usize % CONFETTI_CHARS.len()];
        let color = colors[(h >> 20) as usize % colors.len()];
        buf[(col, y)].set_char(ch).set_fg(color);
    }
}

/// Draws a single square, scaling to whatever `area` it's given (down to 1×1).
fn render_square(square: Square, style: Style, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let fill = match square {
        Square::Black => Style::new().bg(theme::current().block()),
        _ => style,
    };
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            buf[(x, y)].set_char(' ').set_style(fill);
        }
    }

    if let Square::Letter(c) = square {
        let cx = area.x + area.width / 2;
        let cy = area.y + area.height / 2;
        buf[(cx, cy)].set_char(c).set_style(style);
    }
}

#[cfg(test)]
mod tests {
    use super::{marquee, render_confetti, scroll_offset};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    #[test]
    fn confetti_stays_in_bounds() {
        // Offset area so we'd catch off-by-one origin bugs, at several frames.
        let area = Rect::new(3, 2, 20, 10);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        for elapsed in [0, 80, 400, 4999] {
            render_confetti(area, &mut buf, elapsed); // panics on OOB index
        }
        // Zero-size area is a no-op, not a panic.
        render_confetti(Rect::new(0, 0, 0, 0), &mut buf, 100);
    }

    #[test]
    fn marquee_scrolls_overflowing_text() {
        // Fits: returned unchanged regardless of step.
        assert_eq!(marquee("short", 10, 7), "short");
        // Overflows: step 0 shows the start.
        assert_eq!(marquee("abcdefgh", 4, 0), "abcd");
        // Advancing the step scrolls left one char at a time.
        assert_eq!(marquee("abcdefgh", 4, 1), "bcde");
        assert_eq!(marquee("abcdefgh", 4, 2), "cdef");
        // Cycles back to the start after the text + gap.
        let cycle = "abcdefgh".len() + 4;
        assert_eq!(marquee("abcdefgh", 4, cycle), marquee("abcdefgh", 4, 0));
        // Zero width never panics.
        assert_eq!(marquee("abc", 0, 5), "abc");
    }

    #[test]
    fn scroll_offset_follows_cursor() {
        // Window of 5 over a length-20 line.
        // Target below the window scrolls down just enough to reveal it.
        assert_eq!(scroll_offset(7, 7, 5, 20, 0), 3); // shows 3..8
        // Target above the window scrolls up to it.
        assert_eq!(scroll_offset(4, 4, 5, 20, 10), 4);
        // Target already inside the window: no movement.
        assert_eq!(scroll_offset(11, 11, 5, 20, 10), 10);
        // Clamps at the far edge.
        assert_eq!(scroll_offset(19, 19, 5, 20, 0), 15);
        // Whole line fits: always 0.
        assert_eq!(scroll_offset(3, 3, 20, 10, 0), 0);
    }
}
