//! Cells and ranges as a spreadsheet names them: `B2`, `$B$2`, `A2:D13`, and a range on another
//! sheet, `Sales!A2:A13` or `'Q1 sales'!A2:A13`. Rows and columns count from 0 here, and from
//! 1 and A where a person reads them.

use std::fmt;

/// The most rows and columns a worksheet has (Excel's limits).
pub(crate) const MAX_ROWS: u32 = 1_048_576;
pub(crate) const MAX_COLUMNS: u16 = 16_384;

/// One cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) row: u32,
    pub(crate) column: u16,
}

/// The cells from `first` to `last`, both included, `first` above and left of `last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Range {
    pub(crate) first: Cell,
    pub(crate) last: Cell,
}

/// A range that may name its sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SheetRange {
    pub(crate) sheet: Option<String>,
    pub(crate) range: Range,
}

impl Cell {
    /// `B2` or `$B$2`; `None` for anything else or a cell past Excel's limits.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('$').unwrap_or(text);
        let letters = text.find(|c: char| !c.is_ascii_alphabetic())?;
        let (column, row) = text.split_at(letters);
        let row = row.strip_prefix('$').unwrap_or(row);
        if row.starts_with('0') || !row.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let row: u32 = row.parse().ok()?;
        if row == 0 || row > MAX_ROWS {
            return None;
        }
        Some(Self {
            row: row - 1,
            column: column_number(column)?,
        })
    }
}

impl fmt::Display for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&cell_name(self.row, u32::from(self.column)))
    }
}

impl Range {
    /// `A2:D13`, or one cell as a range of one.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (first, last) = text.split_once(':').unwrap_or((text, text));
        let (first, last) = (Cell::parse(first)?, Cell::parse(last)?);
        (first.row <= last.row && first.column <= last.column).then_some(Self { first, last })
    }

    /// How many cells it holds.
    pub(crate) fn cells(&self) -> u64 {
        u64::from(self.last.row - self.first.row + 1)
            * u64::from(self.last.column - self.first.column + 1)
    }

    pub(crate) fn contains(&self, cell: Cell) -> bool {
        (self.first.row..=self.last.row).contains(&cell.row)
            && (self.first.column..=self.last.column).contains(&cell.column)
    }
}

impl SheetRange {
    /// `A2:A13`, `Sales!A2:A13` or `'Q1 sales'!A2:A13`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let Some((sheet, range)) = text.rsplit_once('!') else {
            return Some(Self {
                sheet: None,
                range: Range::parse(text)?,
            });
        };
        let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
            Some(quoted) => quoted.replace("''", "'"),
            None => sheet.to_owned(),
        };
        (!sheet.is_empty()).then_some(())?;
        Some(Self {
            sheet: Some(sheet),
            range: Range::parse(range)?,
        })
    }
}

/// Cells named in a sentence: those `named`, and how many more of `count` there are.
pub(crate) fn listed(named: &[String], count: usize) -> String {
    let (named, more) = (named.join(", "), count.saturating_sub(named.len()));
    if more == 0 {
        named
    } else {
        format!("{named} and {more} more")
    }
}

/// A cell's name from its row and column: `A1` for (0, 0).
pub(crate) fn cell_name(row: u32, column: u32) -> String {
    format!("{}{}", column_name(column), u64::from(row) + 1)
}

/// A column's letters: `A` for 0, `AA` for 26.
pub(crate) fn column_name(column: u32) -> String {
    let mut letters = Vec::new();
    let mut rest = u64::from(column) + 1;
    while rest > 0 {
        let letter = u8::try_from((rest - 1) % 26).unwrap_or(0);
        letters.push(char::from(b'A' + letter));
        rest = (rest - 1) / 26;
    }
    letters.iter().rev().collect()
}

/// A column's number from its letters, `A` being 0; `None` past Excel's last column, `XFD`.
pub(crate) fn column_number(letters: &str) -> Option<u16> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut number: u32 = 0;
    for letter in letters.bytes() {
        if !letter.is_ascii_alphabetic() {
            return None;
        }
        number = number * 26 + u32::from(letter.to_ascii_uppercase() - b'A') + 1;
    }
    u16::try_from(number - 1)
        .ok()
        .filter(|column| *column < MAX_COLUMNS)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn cells_are_named_as_a_spreadsheet_names_them() {
        let names: Vec<String> = [0, 25, 26, 701, 702, 16_383].map(column_name).into();
        assert_eq!(names, ["A", "Z", "AA", "ZZ", "AAA", "XFD"]);
        assert_eq!(cell_name(1_048_575, 3), "D1048576");
    }

    #[test]
    fn cells_and_ranges_read_as_a_person_writes_them() {
        assert_eq!(Cell::parse("A1"), Some(Cell { row: 0, column: 0 }));
        assert_eq!(
            Cell::parse("$AB$12"),
            Some(Cell {
                row: 11,
                column: 27
            })
        );
        assert_eq!(
            Cell::parse("xfd1048576").map(|c| c.to_string()),
            Some("XFD1048576".into())
        );
        for wrong in [
            "", "A", "1", "A0", "A01", "XFE1", "A1048577", "A1:B2", "A-1", "1A",
        ] {
            assert_eq!(Cell::parse(wrong), None, "{wrong}");
        }
        let range = Range::parse("B2:D13").unwrap();
        assert!(range.contains(Cell::parse("C5").unwrap()));
        assert!(!range.contains(Cell::parse("E5").unwrap()));
        assert_eq!(range.cells(), 36);
        assert_eq!(Range::parse("D13:B2"), None, "backwards");
        assert_eq!(Range::parse("C3").unwrap().cells(), 1);
        let on = SheetRange::parse("'Q1 ''best'' sales'!A2:A13").unwrap();
        assert_eq!(on.sheet.as_deref(), Some("Q1 'best' sales"));
        assert_eq!(on.range, Range::parse("A2:A13").unwrap());
        assert_eq!(
            SheetRange::parse("Sales!B2").unwrap().sheet.as_deref(),
            Some("Sales")
        );
        assert_eq!(SheetRange::parse("B2").unwrap().sheet, None);
        assert_eq!(SheetRange::parse("!B2"), None);
    }

    proptest! {
        #[test]
        fn a_cell_reads_back_as_it_is_named(row in 0..MAX_ROWS, column in 0..MAX_COLUMNS) {
            let cell = Cell { row, column };
            prop_assert_eq!(Cell::parse(&cell.to_string()), Some(cell));
        }

        #[test]
        fn nothing_read_is_past_the_limits(text in "\\$?[A-Za-z]{0,4}\\$?[0-9]{0,8}") {
            if let Some(cell) = Cell::parse(&text) {
                prop_assert!(cell.row < MAX_ROWS && cell.column < MAX_COLUMNS);
            }
        }
    }
}
