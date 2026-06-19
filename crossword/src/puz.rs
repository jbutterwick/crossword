use std::collections::HashMap;
use std::fmt::{Debug, Display};

use crate::Direction::{Across, Down};
use crate::{ClueIdentifier, checksum::*};
use crate::{Error, Grid, Pos};

/// A `Puz` is essentially only the data in a .puz file. For an interactively solvable
/// puzzle, use `Puzzle` which includes a Cursor.
#[derive(Debug)]
pub(crate) struct Puz {
    pub(crate) solution: Grid,
    pub(crate) solve_state: Grid,
    pub(crate) title: String,
    pub(crate) author: String,
    pub(crate) copyright: String,
    pub(crate) notes: String,
    /// Mapping from grid positions to numbers.
    pub(crate) numbered_squares: HashMap<Pos, u8>,
    /// Mapping from [`ClueIdentifier``]s to clues.
    pub(crate) clues: HashMap<ClueIdentifier, String>,
}

impl Puz {
    /// Creates a Puz from the bytes of a `.puz` file.
    pub(crate) fn parse(data: Vec<u8>) -> Result<(Self, Vec<ChecksumMismatch>), Error> {
        // There is no official spec for the puz file format but I'm following
        // <https://gist.github.com/sliminality/dab21fa834eae0a70193c7cd69c356d5>
        // here and it seems to work well.

        let mut checksum_mismatches = vec![];

        let cib_checksum_expected = checksum_region(&data[0x2C..0x34], 0);

        let mut scanner = Scanner::new(data);

        let overall_checksum = scanner.parse_short()?;
        scanner.take_exact(b"ACROSS&DOWN\0")?;

        let cib_checksum = scanner.parse_short()?;
        if cib_checksum != cib_checksum_expected {
            checksum_mismatches.push(ChecksumMismatch {
                checksum: Checksum::CIB,
                expected: cib_checksum_expected,
                actual: cib_checksum,
            });
        }

        let masked_checksums = scanner.take_n_bytes(8)?;

        // Version string
        let _ = scanner.take_n_bytes(4)?;
        // Reserved 1C
        let _ = scanner.take_n_bytes(2)?;
        // Scrambled checksum
        let _ = scanner.take_n_bytes(2)?;

        // Nothing listed for these but it says the scrambled checksum ends at 0x1F and the
        // width should be at 0x2C so I guess we're just supposed to skip 0x20 through 0x2B?
        let _ = scanner.take_n_bytes(12)?;

        let width = *scanner.pop()? as usize;
        let height = *scanner.pop()? as usize;

        let num_clues = scanner.parse_short()?;

        // Unknown bitmask
        let _ = scanner.parse_short()?;

        let scrambled_tag = scanner.parse_short()?;
        if scrambled_tag != 0 {
            return Err(Error::ScrambledError);
        }

        let solution_bytes = scanner.take_n_bytes(width * height)?;
        let solution = Grid::parse(&solution_bytes, width, height);

        let solve_state_bytes = scanner.take_n_bytes(width * height)?;
        let solve_state = Grid::parse(&solve_state_bytes, width, height);

        let title = scanner.parse_nul_terminated_string()?;
        let author = scanner.parse_nul_terminated_string()?;
        let copyright = scanner.parse_nul_terminated_string()?;

        let mut clues = Vec::with_capacity(num_clues as usize);
        for _ in 0..num_clues {
            clues.push(scanner.parse_nul_terminated_string()?);
        }

        let notes = scanner.parse_nul_terminated_string()?;

        let overall_checksum_expected: u16 = {
            let mut c = checksum_region(&solution_bytes, cib_checksum);
            c = checksum_region(&solve_state_bytes, c);
            c = checksum_metadata_string(&title, c);
            c = checksum_metadata_string(&author, c);
            c = checksum_metadata_string(&copyright, c);
            for clue in clues.iter() {
                c = checksum_clue(clue, c);
            }
            c = checksum_metadata_string(&notes, c);
            c
        };

        if overall_checksum != overall_checksum_expected {
            checksum_mismatches.push(ChecksumMismatch {
                checksum: Checksum::Overall,
                expected: overall_checksum_expected,
                actual: overall_checksum,
            })
        }

        let solution_checksum = checksum_region(&solution_bytes, 0);
        let grid_checksum = checksum_region(&solve_state_bytes, 0);
        let partial_board_checksum = {
            let mut c = 0;
            c = checksum_metadata_string(&title, c);
            c = checksum_metadata_string(&author, c);
            c = checksum_metadata_string(&copyright, c);
            for clue in clues.iter() {
                c = checksum_clue(clue, c);
            }
            c = checksum_metadata_string(&notes, c);
            c
        };

        let expected_masked_checksums = [
            0x49 ^ (cib_checksum & 0xFF) as u8,
            0x43 ^ (solution_checksum & 0xFF) as u8,
            0x48 ^ (grid_checksum & 0xFF) as u8,
            0x45 ^ (partial_board_checksum & 0xFF) as u8,
            0x41 ^ ((cib_checksum & 0xFF00) >> 8) as u8,
            0x54 ^ ((solution_checksum & 0xFF00) >> 8) as u8,
            0x45 ^ ((grid_checksum & 0xFF00) >> 8) as u8,
            0x44 ^ ((partial_board_checksum & 0xFF00) >> 8) as u8,
        ];

        assert_eq!(expected_masked_checksums.len(), masked_checksums.len());
        for (i, (expected, actual)) in expected_masked_checksums
            .iter()
            .zip(masked_checksums.iter())
            .enumerate()
        {
            if expected != actual {
                checksum_mismatches.push(ChecksumMismatch {
                    checksum: Checksum::Masked(i),
                    expected: *expected as u16,
                    actual: *actual as u16,
                })
            }
        }

        let clues: Vec<String> = clues.iter().map(|clue| decode_str(clue)).collect();

        let (numbered_squares, clues) = allocate_clues(&solution, &clues);

        let puz = Self {
            solution,
            solve_state,
            title: decode_str(&title),
            author: decode_str(&author),
            copyright: decode_str(&copyright),
            notes: decode_str(&notes),
            numbered_squares,
            clues,
        };
        Ok((puz, checksum_mismatches))
    }

    /// Serializes this puzzle back into `.puz` file bytes, recomputing all
    /// checksums so the result is internally consistent and re-parseable.
    ///
    /// Header fields the parser ignores (version, reserved, scrambled checksum,
    /// the unknown bitmask) are written as fixed zero/`1.3` values rather than
    /// preserved, so this is a semantic round-trip, not a byte-for-byte one.
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let solution_bytes = self.solution.to_bytes();
        let solve_state_bytes = self.solve_state.to_bytes();
        let (width, height) = self.solution.size();

        // Strings as NUL-terminated UTF-8.
        let nul = |s: &str| {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            b
        };
        let title = nul(&self.title);
        let author = nul(&self.author);
        let copyright = nul(&self.copyright);
        let notes = nul(&self.notes);
        let clues: Vec<Vec<u8>> = self.clues_in_file_order().iter().map(|c| nul(c)).collect();
        let num_clues = clues.len() as u16;

        // The CIB checksum covers the 8 header bytes at 0x2C..0x34. We write the
        // bitmask and scrambled tag as 0, so compute over the same bytes.
        let mut header_2c = vec![width as u8, height as u8];
        header_2c.extend_from_slice(&num_clues.to_le_bytes());
        header_2c.extend_from_slice(&[0, 0]); // bitmask
        header_2c.extend_from_slice(&[0, 0]); // scrambled tag
        let cib = checksum_region(&header_2c, 0);

        let overall = {
            let mut c = checksum_region(&solution_bytes, cib);
            c = checksum_region(&solve_state_bytes, c);
            c = checksum_metadata_string(&title, c);
            c = checksum_metadata_string(&author, c);
            c = checksum_metadata_string(&copyright, c);
            for clue in &clues {
                c = checksum_clue(clue, c);
            }
            checksum_metadata_string(&notes, c)
        };

        let solution_checksum = checksum_region(&solution_bytes, 0);
        let grid_checksum = checksum_region(&solve_state_bytes, 0);
        let partial_board_checksum = {
            let mut c = checksum_metadata_string(&title, 0);
            c = checksum_metadata_string(&author, c);
            c = checksum_metadata_string(&copyright, c);
            for clue in &clues {
                c = checksum_clue(clue, c);
            }
            checksum_metadata_string(&notes, c)
        };
        let masked = [
            0x49 ^ (cib & 0xFF) as u8,
            0x43 ^ (solution_checksum & 0xFF) as u8,
            0x48 ^ (grid_checksum & 0xFF) as u8,
            0x45 ^ (partial_board_checksum & 0xFF) as u8,
            0x41 ^ ((cib & 0xFF00) >> 8) as u8,
            0x54 ^ ((solution_checksum & 0xFF00) >> 8) as u8,
            0x45 ^ ((grid_checksum & 0xFF00) >> 8) as u8,
            0x44 ^ ((partial_board_checksum & 0xFF00) >> 8) as u8,
        ];

        let mut out = Vec::new();
        out.extend_from_slice(&overall.to_le_bytes()); // 0x00
        out.extend_from_slice(b"ACROSS&DOWN\0"); // 0x02
        out.extend_from_slice(&cib.to_le_bytes()); // 0x0E
        out.extend_from_slice(&masked); // 0x10
        out.extend_from_slice(b"1.3\0"); // 0x18 version
        out.extend_from_slice(&[0, 0]); // 0x1C reserved
        out.extend_from_slice(&[0, 0]); // 0x1E scrambled checksum
        out.extend_from_slice(&[0; 12]); // 0x20..0x2C
        out.extend_from_slice(&header_2c); // 0x2C..0x34 (width/height/clues/bitmask/scrambled tag)
        out.extend_from_slice(&solution_bytes);
        out.extend_from_slice(&solve_state_bytes);
        out.extend_from_slice(&title);
        out.extend_from_slice(&author);
        out.extend_from_slice(&copyright);
        for clue in &clues {
            out.extend_from_slice(clue);
        }
        out.extend_from_slice(&notes);
        out
    }

    /// Clues in the order they appear in a `.puz` file: scanning the grid
    /// top-to-bottom, left-to-right, across before down at each numbered square.
    /// Mirrors the consumption order in [`allocate_clues`].
    fn clues_in_file_order(&self) -> Vec<&String> {
        let mut out = Vec::with_capacity(self.clues.len());
        for pos in self.solution.positions() {
            let num = match self.numbered_squares.get(&pos) {
                Some(n) => *n,
                None => continue,
            };
            if self.solution.starts_across(pos) {
                out.push(self.clues.get(&(num, Across)).unwrap());
            }
            if self.solution.starts_down(pos) {
                out.push(self.clues.get(&(num, Down)).unwrap());
            }
        }
        out
    }
}

/// Turns a NUL-terminated string into a standard String. Tries UTF-8, falling
/// back to ISO-8859-1 (where every byte maps directly to the same codepoint, so
/// it never fails).
fn decode_str(bytes: &[u8]) -> String {
    assert_eq!(0x0, *bytes.last().unwrap());

    let bytes = &bytes[0..bytes.len() - 1];
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect())
}

fn allocate_clues(
    grid: &Grid,
    clue_list: &[String],
) -> (HashMap<Pos, u8>, HashMap<ClueIdentifier, String>) {
    let mut clue_number: u8 = 1;
    let mut numbered_squares = HashMap::new();
    let mut clues = HashMap::with_capacity(clue_list.len());

    let mut clue_iter = clue_list.iter();

    for pos in grid.positions() {
        let starts_across = grid.starts_across(pos);
        let starts_down = grid.starts_down(pos);

        if starts_across || starts_down {
            numbered_squares.insert(pos, clue_number);

            if starts_across {
                let clue = clue_iter.next().unwrap();
                clues.insert((clue_number, Across), clue.clone());
            }

            if starts_down {
                let clue = clue_iter.next().unwrap();
                clues.insert((clue_number, Down), clue.clone());
            }

            clue_number += 1;
        }
    }

    (numbered_squares, clues)
}

// Loosely based on
// https://depth-first.com/articles/2021/12/16/a-beginners-guide-to-parsing-in-rust/
struct Scanner {
    cursor: usize,
    data: Vec<u8>,
}

impl Debug for Scanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scanner")
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl Scanner {
    fn new(data: Vec<u8>) -> Self {
        Self { cursor: 0, data }
    }

    /// Consume and return the next byte, or return None if that can't be done.
    fn pop(&mut self) -> Result<&u8, Error> {
        match self.data.get(self.cursor) {
            Some(byte) => {
                self.cursor += 1;
                Ok(byte)
            }
            None => Err(Error::EofError(self.cursor)),
        }
    }

    /// Consume the next two bytes and return them as a `u16`, interpreted as little-endian.
    fn parse_short(&mut self) -> Result<u16, Error> {
        if let Some(b1) = self.data.get(self.cursor) {
            if let Some(b2) = self.data.get(self.cursor + 1) {
                self.cursor += 2;
                return Ok((*b2 as u16) << 8 | (*b1 as u16));
            }
        }
        Err(Error::EofError(self.cursor))
    }

    /// Take the next `expected.len()` bytes, if they match `expected`.
    fn take_exact(&mut self, expected: &[u8]) -> Result<(), Error> {
        for (i, expected_byte) in expected.iter().enumerate() {
            if let Some(b) = self.data.get(self.cursor + i) {
                if b != expected_byte {
                    return Err(Error::ParseError(format!(
                        "Expected byte 0x{:X} at position 0x{:X} but got 0x{:X}",
                        expected_byte,
                        self.cursor + i,
                        b
                    )));
                }
            }
        }
        self.cursor += expected.len();
        Ok(())
    }

    /// Take the next `n`` bytes.
    fn take_n_bytes(&mut self, n: usize) -> Result<Vec<u8>, Error> {
        if self.cursor >= self.data.len() {
            return Err(Error::EofError(self.cursor));
        }

        if self.cursor + n >= self.data.len() {
            return Err(Error::EofError(self.data.len()));
        }

        let data = &self.data[self.cursor..self.cursor + n];
        self.cursor += n;
        Ok(Vec::from(data))
    }

    /// Parses a C-style NUL-terminated string, including the NUL byte. In
    /// this function, we return just the raw bytes as they appear in the
    /// file. Converting them to a string is done later.
    fn parse_nul_terminated_string(&mut self) -> Result<Vec<u8>, Error> {
        for (index, byte) in self.data[self.cursor..].iter().enumerate() {
            if *byte == 0 {
                let bytes = &self.data[self.cursor..self.cursor + index + 1];
                self.cursor += index + 1;
                return Ok(bytes.to_vec());
            }
        }
        Err(Error::EofError(self.data.len()))
    }
}

/// Returned when parsing a .puz file succeeded, but one or more of the checksums
/// in the file didn't match the expected value. May indicate a corrupted .puz file,
/// or a bug in this crate.
#[derive(Eq, PartialEq)]
pub struct ChecksumMismatch {
    checksum: Checksum,
    expected: u16,
    actual: u16,
}

impl Debug for ChecksumMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Mismatch on checksum {}: Expected {:#x} but got {:#x}",
            self.checksum, self.expected, self.actual
        )
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Checksum {
    CIB,
    Overall,
    Masked(usize),
}

impl Display for Checksum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use super::*;

    #[test]
    fn parse_v12_puzzle() {
        let data: Vec<u8> = fs::read("puzzles/version-1.2-puzzle.puz").unwrap();
        let (puz, checksum_mismatches) = Puz::parse(data).unwrap();
        assert_eq!(checksum_mismatches, []);
        assert_eq!(puz.title, "Reference PUZ File");
        assert_eq!(puz.author, "Josh Myer");
        assert_eq!(puz.copyright, "Copyright (c) 2005 Josh Myer");
        assert_eq!(puz.notes, "");

        #[rustfmt::skip]
        assert_eq!(puz.numbered_squares, HashMap::from([
          ((0, 1), 1),
          ((1, 0), 2),
          ((1, 3), 3),
          ((3, 1), 4),
        ]));

        #[rustfmt::skip]
        assert_eq!(
          puz.clues,
          HashMap::from([
            ((1, Down), "Pumps your basement".into()), // SUMP
            ((2, Across), "I'm ___, thanks for asking\\!".into()), // SUPER
            ((3, Down), "Until".into()), // ERE
            ((4, Across), "One step short of a pier".into()), // PIE
          ])
        );

        #[rustfmt::skip]
        assert_eq!(
          puz.solution.to_string(),
          concat!(
            "\n",
            "■S■■■\n",
            "SUPER\n",
            "■M■R■\n",
            "■PIE■\n",
          )
        );

        #[rustfmt::skip]
        assert_eq!(
          puz.solve_state.to_string(),
          concat!(
            "\n",
            "■S■■■\n",
            " U   \n",
            "■M■ ■\n",
            "■P  ■\n",
          )
        );
    }

    #[test]
    fn serialize_round_trips() {
        let data: Vec<u8> = fs::read("puzzles/version-1.2-puzzle.puz").unwrap();
        let (puz, _) = Puz::parse(data).unwrap();

        let bytes = puz.serialize();
        let (reparsed, mismatches) = Puz::parse(bytes).unwrap();

        // A re-parse of our output must have valid checksums and identical content.
        assert_eq!(mismatches, []);
        assert_eq!(reparsed.solution.to_string(), puz.solution.to_string());
        assert_eq!(reparsed.solve_state.to_string(), puz.solve_state.to_string());
        assert_eq!(reparsed.title, puz.title);
        assert_eq!(reparsed.author, puz.author);
        assert_eq!(reparsed.copyright, puz.copyright);
        assert_eq!(reparsed.notes, puz.notes);
        assert_eq!(reparsed.clues, puz.clues);
        assert_eq!(reparsed.numbered_squares, puz.numbered_squares);
    }
}
