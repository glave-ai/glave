//! Tables as a model reads them: a row to a line between bars, the first row the header, each
//! cell's text kept whole on its line.

/// `rows` as lines of a table, every row as wide as the widest.
pub(crate) fn lines(rows: &[Vec<String>]) -> Vec<String> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for (index, row) in rows.iter().enumerate() {
        let cells: Vec<String> = (0..width)
            .map(|column| {
                row.get(column)
                    .map(|cell| escaped(cell.trim()))
                    .unwrap_or_default()
            })
            .collect();
        lines.push(format!("| {} |", cells.join(" | ")));
        if index == 0 {
            lines.push(format!("|{}", "---|".repeat(width)));
        }
    }
    lines
}

/// A cell's text safe inside a table row.
pub(crate) fn escaped(text: &str) -> String {
    text.replace('|', "\\|")
        .replace('\r', "")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_as_wide_as_the_widest_and_cells_kept_whole() {
        let rows = vec![
            vec!["Region".to_owned(), "Revenue".to_owned()],
            vec!["North | East".to_owned(), "1,200\n(est.)".to_owned()],
            vec!["West".to_owned()],
        ];
        assert_eq!(
            lines(&rows),
            [
                "| Region | Revenue |",
                "|---|---|",
                "| North \\| East | 1,200\\n(est.) |",
                "| West |  |",
            ]
        );
        assert_eq!(lines(&[]), Vec::<String>::new());
        assert_eq!(escaped("a|b\nc"), "a\\|b\\nc");
    }
}
