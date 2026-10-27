//! CSV as spreadsheet apps write it (RFC 4180): fields between commas, a field in quotes
//! when it holds the separator, a quote (doubled) or a line break. Semicolons or tabs
//! separate the fields instead when the first line has more of them than commas, as a
//! spreadsheet app in much of Europe saves CSV, and as a .tsv file is.

use crate::error::OfficeError;

/// The rows of the CSV `text` from the file `name`, each a list of fields, blank lines left
/// out.
pub(crate) fn rows(text: &str, name: &str) -> Result<Vec<Vec<String>>, OfficeError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let separator = separator(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    // Where the open quote began, to say where one is never closed.
    let (mut line, mut opened) = (1_usize, 0_usize);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => {
                    line += usize::from(c == '\n');
                    field.push(c);
                }
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                opened = line;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                line += 1;
                row.push(std::mem::take(&mut field));
                end(&mut rows, std::mem::take(&mut row));
            }
            c if c == separator => row.push(std::mem::take(&mut field)),
            c => field.push(c),
        }
    }
    if quoted {
        return Err(OfficeError::Data(format!(
            "{name} has a quote opened on line {opened} and never closed"
        )));
    }
    row.push(field);
    end(&mut rows, row);
    Ok(rows)
}

/// `row` added to `rows`, unless it is a blank line.
fn end(rows: &mut Vec<Vec<String>>, row: Vec<String>) {
    if !(row.len() == 1 && row.iter().all(String::is_empty)) {
        rows.push(row);
    }
}

/// What separates the fields: a comma, unless the first line has more semicolons or tabs.
fn separator(text: &str) -> char {
    let first = text.lines().next().unwrap_or_default();
    let count = |separator: char| first.matches(separator).count();
    [';', '\t']
        .into_iter()
        .filter(|other| count(*other) > count(','))
        .max_by_key(|other| count(*other))
        .unwrap_or(',')
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn read(text: &str) -> Vec<Vec<String>> {
        rows(text, "a.csv").unwrap()
    }

    #[test]
    fn fields_are_read_as_spreadsheet_apps_write_them() {
        assert_eq!(
            read("\u{feff}a,b,c\r\n1,\"two, \"\"2\"\"\",\"line\nbreak\"\r\n\r\n3,,\n"),
            [
                vec!["a", "b", "c"],
                vec!["1", "two, \"2\"", "line\nbreak"],
                vec!["3", "", ""],
            ]
        );
        assert_eq!(read("a;b\n1,5;2"), [vec!["a", "b"], vec!["1,5", "2"]]);
        assert_eq!(read("a\tb\n1\t2"), [vec!["a", "b"], vec!["1", "2"]]);
        assert_eq!(read(""), Vec::<Vec<String>>::new());
    }

    #[test]
    fn a_quote_never_closed_says_where_it_opened() {
        let error = rows("a,b\n1,2\n3,\"four\n5,6\n", "a.csv").unwrap_err();
        assert_eq!(
            error.to_string(),
            "a.csv has a quote opened on line 3 and never closed"
        );
    }

    proptest! {
        #[test]
        fn fields_written_as_csv_read_back_whole(
            table in prop::collection::vec(prop::collection::vec("[a-z0-9,\" \n]{1,6}", 3), 1..5)
        ) {
            let text: String = table
                .iter()
                .map(|row| {
                    let fields: Vec<String> = row
                        .iter()
                        .map(|field| format!("\"{}\"", field.replace('"', "\"\"")))
                        .collect();
                    fields.join(",") + "\r\n"
                })
                .collect();
            prop_assert_eq!(read(&text), table);
        }
    }
}
