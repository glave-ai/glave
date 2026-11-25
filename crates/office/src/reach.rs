//! Whether a worksheet is small enough to read whole. calamine holds every cell from a sheet's
//! first used cell to its last, empty or not, so two cells far apart (A1 and XFD1048576) would
//! take hundreds of gigabytes; a sheet's cells are first read as a stream, holding none, to
//! find how far they span.

use calamine::{DataRef, Sheets};

use crate::error::OfficeError;
use crate::read::Book;

/// The most cells glave reads of a sheet, from its first used cell to its last: a few hundred
/// MB as calamine holds them.
const MOST_CELLS: u64 = 10_000_000;

/// The first and last row and column a sheet uses.
#[derive(Default)]
struct Span {
    corners: Option<((u32, u32), (u32, u32))>,
}

impl Span {
    fn take(&mut self, (row, column): (u32, u32)) {
        let ((top, left), (bottom, right)) = self.corners.unwrap_or(((row, column), (row, column)));
        self.corners = Some((
            (top.min(row), left.min(column)),
            (bottom.max(row), right.max(column)),
        ));
    }

    /// How many rows and columns it spans.
    fn size(&self) -> (u64, u64) {
        self.corners
            .map_or((0, 0), |((top, left), (bottom, right))| {
                (
                    u64::from(bottom.saturating_sub(top)).saturating_add(1),
                    u64::from(right.saturating_sub(left)).saturating_add(1),
                )
            })
    }
}

/// Refuses `sheet` of `book` when the cells it uses, its values or its formulas, span more than
/// [`MOST_CELLS`]. Only xlsx and xlsb are read by cell: calamine reads an xls or ods workbook
/// whole when it opens it, xls within the format's 65,536 rows by 256 columns and ods within a
/// cap of its own.
pub(crate) fn within_reach(book: &mut Book, sheet: &str) -> Result<(), OfficeError> {
    let unread = |error: String| OfficeError::Sheet(sheet.to_owned(), error);
    let (mut values, mut formulas) = (Span::default(), Span::default());
    // xlsx and xlsb read alike, through readers of different types.
    macro_rules! spans {
        ($workbook:expr) => {{
            {
                let mut cells = $workbook
                    .worksheet_cells_reader(sheet)
                    .map_err(|error| unread(error.to_string()))?;
                while let Some(cell) = cells.next_cell().map_err(|e| unread(e.to_string()))? {
                    if !matches!(cell.get_value(), DataRef::Empty) {
                        values.take(cell.get_position());
                    }
                }
            }
            let mut cells = $workbook
                .worksheet_cells_reader(sheet)
                .map_err(|error| unread(error.to_string()))?;
            while let Some(cell) = cells.next_formula().map_err(|e| unread(e.to_string()))? {
                if !cell.get_value().is_empty() {
                    formulas.take(cell.get_position());
                }
            }
        }};
    }
    match book {
        Sheets::Xlsx(xlsx) => spans!(xlsx),
        Sheets::Xlsb(xlsb) => spans!(xlsb),
        Sheets::Xls(_) | Sheets::Ods(_) => {}
    }
    for span in [values, formulas] {
        let (rows, columns) = span.size();
        if rows.saturating_mul(columns) > MOST_CELLS {
            return Err(OfficeError::TooWide {
                sheet: sheet.to_owned(),
                rows,
                columns,
                most: MOST_CELLS,
            });
        }
    }
    Ok(())
}
