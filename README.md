# Crossword and Crosstui

[![animated gif of crosstui in action](https://asciinema.org/a/7QP9xIRD7EDB6UluaoOSbbfSc.svg)](https://asciinema.org/a/7QP9xIRD7EDB6UluaoOSbbfSc)

This is an experimental project for solving crosswords in the terminal. I built it mainly because I wanted to build something fun with [Ratatui], a library for TUIs (user interfaces that run in the terminal).

If you like crosswords and running things in the terminal, please give it a try!

## To play a crossword

Run the `crosstui` binary with no arguments to open the puzzle explorer:

```sh
cargo run --release --bin=crosstui
```

Or jump straight into a specific `.puz` file:

```sh
cargo run --release --bin=crosstui -- /path/to/your/crossword.puz
```

While solving, the grid follows the current word, `+`/`-`/`0` zoom, and a
confetti burst celebrates when you complete the puzzle.

## The puzzle explorer

The explorer is a library of the puzzles you've collected, stored under your
platform data directory (e.g. `~/.local/share/crosstui`). It shows each
puzzle's title and solve status (unsolved, in progress, or complete), and lets
you manage them entirely from the keyboard:

- `↑`/`↓` move, `Enter` opens the selected puzzle
- `f` favorite, `r` rename, `x` reset progress, `d` delete
- `s` opens **sources** — the outlet downloader (see below)
- `c` opens the **Crosshare browser** — browse an actual list of puzzles (see below)
- `t` opens **themes** — pick a color theme with live in-menu preview; themes
  are defined in TOML and your choice is remembered
- `q` quits

Progress is saved back to the `.puz` file automatically as you go.

## Downloading puzzles

The sources screen (`s`) pulls puzzles straight into your library:

- Nearly twenty outlets — Newsday, USA Today, Universal, LA Times (+ Mini),
  The New Yorker (+ Mini), Washington Post, The Atlantic, Vox, Vulture, the
  Guardian (quick/cryptic/everyman), Puzzmo, and more — each with a short
  description. `Enter` grabs the latest.
- `d` fetches an **older** puzzle by date (`YYYY-MM-DD`), for outlets whose
  archives support it.
- `u` downloads from a **URL** you paste in: a direct `.puz` link is fetched and
  validated, and outlet pages are handed to `xword-dl` to scrape.

Downloads use [`xword-dl`]. crosstui tries to install it for you (via `pipx`,
falling back to `pip`) the first time you run it; if that doesn't work, install
it yourself with `pipx install xword-dl`.

## Browsing Crosshare

Outlets mostly give you "the latest" or a specific date — not a browsable list.
[Crosshare] (a free, open community for constructors) is the exception, so the
Crosshare browser (`c`) shows real, paginated lists you can scroll and download
from directly:

- `Tab` switches between the **Newest**, **Featured**, and **Daily Minis** feeds
- `n`/`p` page through the feed, `↑`/`↓` move, `Enter` downloads into your library
- `xword-dl` isn't needed here — it's a small built-in scraper over Crosshare's
  public listing pages and its `/api/puz` endpoint (only `curl` is required)

[`xword-dl`]: https://github.com/thisisparker/xword-dl
[Crosshare]: https://crosshare.org

## Code organization

The code is separated into two crates.

The `crossword` crate understands `.puz` files and how crossword puzzles work, but has no UI. Its primary type is the `Puzzle` struct, and it should be possible to build a separate crossword app using a different UI framework, by depending on `crossword` only.

The `crosstui` crate is the interactive UI for displaying and solving puzzles, which uses the `crossword` crate to model the puzzle, and [Ratatui] for the UI.

[Ratatui]: https://ratatui.rs/
