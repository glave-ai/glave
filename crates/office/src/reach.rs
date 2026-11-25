//! A workbook's sheets read within a budget of cells for the whole workbook. calamine holds every
//! cell of a range from its first used cell to its last, empty or not, so two cells far apart
//! (A1 and XFD1048576) would take hundreds of gigabytes. An xlsx or xlsb sheet is read once, as
//! a stream of the cells it holds, stopped as soon as they span more than is left, and only then
//! made a range. calamine reads an xls or ods workbook whole when it opens it, xls within the
//! format's 65,536 rows by 256 columns and ods within a cap of its own (100 million cells), and
//! its sheets spend the budget all the same. The strings an xlsx or xlsb workbook shares among
//! its cells are also read whole when it opens, so their part is measured first, unpacked
//! ([`shared_strings`]).

use std::io::{Cursor, Read as _};

use calamine::{Cell, CellType, Data, DataRef, Range, Reader as _, Sheets};
use zip::ZipArchive;

use crate::error::OfficeError;
use crate::read::Book;

/// The most cells glave holds of a workbook, each sheet's from its first used cell to its
/// last, values and formulas apart: a few hundred MB as calamine holds them.
const MOST_CELLS: u64 = 10_000_000;

/// The most bytes the strings an xlsx or xlsb workbook shares may take, unpacked.
const MOST_SHARED_STRINGS: u64 = 128 * 1024 * 1024;

/// Where an xlsx and an xlsb workbook keep the strings their cells share.
const SHARED_STRINGS: [&str; 2] = ["xl/sharedStrings.xml", "xl/sharedStrings.bin"];

/// What is left to read of a workbook, in cells.
pub(crate) struct Budget {
    left: u64,
}

impl Budget {
    pub(crate) fn new() -> Self {
        Self { left: MOST_CELLS }
    }

    /// Spends what `cells` span, refusing `sheet` when that is more than is left.
    fn spend(&mut self, sheet: &str, cells: u64) -> Result<(), OfficeError> {
        self.left = self
            .left
            .checked_sub(cells)
            .ok_or_else(|| OfficeError::TooWide {
                sheet: sheet.to_owned(),
                most: MOST_CELLS,
            })?;
        Ok(())
    }
}

/// The first and last row and column of the cells read so far.
#[derive(Default)]
struct Span {
    corners: Option<((u32, u32), (u32, u32))>,
}

impl Span {
    /// Takes in the cell at `(row, column)`: how many cells the span now covers.
    fn take(&mut self, (row, column): (u32, u32)) -> u64 {
        let ((top, left), (bottom, right)) = self.corners.unwrap_or(((row, column), (row, column)));
        let ((top, left), (bottom, right)) = (
            (top.min(row), left.min(column)),
            (bottom.max(row), right.max(column)),
        );
        self.corners = Some(((top, left), (bottom, right)));
        u64::from(bottom.saturating_sub(top))
            .saturating_add(1)
            .saturating_mul(u64::from(right.saturating_sub(left)).saturating_add(1))
    }
}

/// The cells `next` gives that `kept` keeps, as a range, the cells they span spent from
/// `budget`: stopped, refusing `sheet`, as soon as they span more than is left.
fn collected<S: CellType, T: CellType, E: ToString>(
    sheet: &str,
    budget: &mut Budget,
    mut next: impl FnMut() -> Result<Option<Cell<S>>, E>,
    kept: impl Fn(&Cell<S>) -> Option<Cell<T>>,
) -> Result<Range<T>, OfficeError> {
    let mut span = Span::default();
    let mut spanned = 0;
    let mut cells = Vec::new();
    while let Some(cell) =
        next().map_err(|error| OfficeError::Sheet(sheet.to_owned(), error.to_string()))?
    {
        let Some(cell) = kept(&cell) else {
            continue;
        };
        spanned = span.take(cell.get_position());
        if spanned > budget.left {
            budget.spend(sheet, spanned)?;
        }
        cells.push(cell);
    }
    budget.spend(sheet, spanned)?;
    Ok(Range::from_sparse(cells))
}

/// A range calamine already holds, its size spent from `budget`.
fn spent<T: CellType>(
    sheet: &str,
    budget: &mut Budget,
    range: Range<T>,
) -> Result<Range<T>, OfficeError> {
    let (rows, columns) = range.get_size();
    let size = u64::try_from(rows.saturating_mul(columns)).unwrap_or(u64::MAX);
    budget.spend(sheet, size)?;
    Ok(range)
}

/// A value that is there, as calamine gives it to keep.
fn value(cell: &Cell<DataRef<'_>>) -> Option<Cell<Data>> {
    let value = cell.get_value();
    (!matches!(value, DataRef::Empty)).then(|| Cell::new(cell.get_position(), value.clone().into()))
}

/// The values of `sheet`, within `budget`.
pub(crate) fn values(
    book: &mut Book,
    sheet: &str,
    budget: &mut Budget,
) -> Result<Range<Data>, OfficeError> {
    let unread = |error: String| OfficeError::Sheet(sheet.to_owned(), error);
    match book {
        Sheets::Xlsx(xlsx) => {
            let mut cells = xlsx
                .worksheet_cells_reader(sheet)
                .map_err(|error| unread(error.to_string()))?;
            collected(sheet, budget, || cells.next_cell(), value)
        }
        Sheets::Xlsb(xlsb) => {
            let mut cells = xlsb
                .worksheet_cells_reader(sheet)
                .map_err(|error| unread(error.to_string()))?;
            collected(sheet, budget, || cells.next_cell(), value)
        }
        Sheets::Xls(_) | Sheets::Ods(_) => {
            let range = book
                .worksheet_range(sheet)
                .map_err(|error| unread(error.to_string()))?;
            spent(sheet, budget, range)
        }
    }
}

/// A formula that is there.
fn formula(cell: &Cell<String>) -> Option<Cell<String>> {
    (!cell.get_value().is_empty()).then(|| cell.clone())
}

/// The formulas of `sheet`, within `budget`; none when they can't be read, as a kind of
/// workbook that keeps none has none.
pub(crate) fn formulas(
    book: &mut Book,
    sheet: &str,
    budget: &mut Budget,
) -> Result<Range<String>, OfficeError> {
    let read = match book {
        Sheets::Xlsx(xlsx) => match xlsx.worksheet_cells_reader(sheet) {
            Ok(mut cells) => collected(sheet, budget, || cells.next_formula(), formula),
            Err(error) => Err(OfficeError::Sheet(sheet.to_owned(), error.to_string())),
        },
        Sheets::Xlsb(xlsb) => match xlsb.worksheet_cells_reader(sheet) {
            Ok(mut cells) => collected(sheet, budget, || cells.next_formula(), formula),
            Err(error) => Err(OfficeError::Sheet(sheet.to_owned(), error.to_string())),
        },
        Sheets::Xls(_) | Sheets::Ods(_) => match book.worksheet_formula(sheet) {
            Ok(range) => spent(sheet, budget, range),
            Err(error) => Err(OfficeError::Sheet(sheet.to_owned(), error.to_string())),
        },
    };
    match read {
        Err(OfficeError::Sheet(..)) => Ok(Range::default()),
        read => read,
    }
}

/// Refuses a workbook whose shared strings take more than [`MOST_SHARED_STRINGS`] unpacked,
/// read as a stream before calamine holds them all. A file that is not a zip (xls) has none.
pub(crate) fn shared_strings(bytes: &[u8]) -> Result<(), OfficeError> {
    let Ok(mut zip) = ZipArchive::new(Cursor::new(bytes)) else {
        return Ok(());
    };
    for name in SHARED_STRINGS {
        let Ok(part) = zip.by_name(name) else {
            continue;
        };
        let unpacked = std::io::copy(
            &mut part.take(MOST_SHARED_STRINGS + 1),
            &mut std::io::sink(),
        )
        .map_err(|error| OfficeError::NotAWorkbook(error.to_string()))?;
        if unpacked > MOST_SHARED_STRINGS {
            return Err(OfficeError::TooMuchText {
                most: MOST_SHARED_STRINGS / (1024 * 1024),
            });
        }
    }
    Ok(())
}
