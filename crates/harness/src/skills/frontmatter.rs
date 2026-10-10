//! A SKILL.md's frontmatter: the YAML block between two `---` lines at the top of the file.
//! Only the part of YAML that frontmatter uses is read: one mapping of keys to plain, quoted or
//! block (`|`, `>`) text, plus nested values, which are kept only as "something else" because
//! no field glave reads is nested. Anything else is an error naming the line.

use std::collections::BTreeMap;

/// A value in the frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Text(String),
    /// A mapping, a list or a flow collection: not text.
    Other,
}

/// A SKILL.md split into its fields and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Document {
    pub(crate) fields: BTreeMap<String, Value>,
    pub(crate) body: String,
}

/// Splits `text` into its frontmatter fields and the body after them.
pub(crate) fn parse(text: &str) -> Result<Document, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().unwrap_or_default();
    if trimmed_end(first) != "---" {
        return Err("it does not start with a --- line opening its frontmatter".to_owned());
    }
    let mut block = Vec::new();
    let mut body = None;
    let mut offset = first.len();
    for line in lines {
        offset = offset.saturating_add(line.len());
        if trimmed_end(line) == "---" {
            body = Some(text.get(offset..).unwrap_or_default());
            break;
        }
        block.push(trimmed_end(line));
    }
    let body = body.ok_or("its frontmatter has no closing --- line")?;
    Ok(Document {
        fields: mapping(&block)?,
        body: body.to_owned(),
    })
}

/// A line without its line ending.
fn trimmed_end(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

/// The top-level mapping of the frontmatter's lines.
fn mapping(lines: &[&str]) -> Result<BTreeMap<String, Value>, String> {
    let mut fields = BTreeMap::new();
    let mut at = 0;
    while let Some(line) = lines.get(at) {
        let number = at.saturating_add(2);
        at = at.saturating_add(1);
        if is_blank(line) {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            return Err(format!("line {number} is indented but belongs to no field"));
        }
        let (key, rest) = line
            .split_once(':')
            .filter(|(key, rest)| is_key(key) && (rest.is_empty() || rest.starts_with(' ')))
            .ok_or_else(|| format!("line {number} is not `name: value`"))?;
        let nested = lines.get(at..).unwrap_or_default();
        let taken = nested.iter().take_while(|line| continues(line)).count();
        let nested = nested.get(..taken).unwrap_or_default();
        at = at.saturating_add(taken);
        let value = value(rest.trim(), nested).map_err(|why| format!("{key}: {why}"))?;
        if fields.insert(key.to_owned(), value).is_some() {
            return Err(format!("{key} is given twice"));
        }
    }
    Ok(fields)
}

fn is_blank(line: &str) -> bool {
    let line = line.trim();
    line.is_empty() || line.starts_with('#')
}

fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether a line carries on the field above it: indented, or blank.
fn continues(line: &str) -> bool {
    line.trim().is_empty() || line.starts_with([' ', '\t'])
}

/// A field's value from the rest of its line and the indented lines under it.
fn value(rest: &str, nested: &[&str]) -> Result<Value, String> {
    if nested.iter().any(|line| line.starts_with('\t')) {
        return Err("tabs cannot indent YAML".to_owned());
    }
    let nested_text = || nested.iter().any(|line| !line.trim().is_empty());
    let rest = without_comment(rest);
    match rest.chars().next() {
        None if nested_text() => Ok(Value::Other),
        None => Ok(Value::Text(String::new())),
        Some('|' | '>') => block(rest, nested).map(Value::Text),
        Some('[' | '{') => Ok(Value::Other),
        Some('&' | '*' | '!' | '%' | '@' | '`') => {
            Err("anchors, aliases, tags and reserved characters are not supported".to_owned())
        }
        Some(quote @ ('"' | '\'')) => {
            if nested_text() {
                return Err("a quoted value must end on its own line".to_owned());
            }
            quoted(rest, quote).map(Value::Text)
        }
        Some(_) => {
            // A plain value may carry on over more indented lines, joined by spaces.
            let mut text = rest.to_owned();
            for line in nested.iter().map(|line| without_comment(line.trim())) {
                if !line.is_empty() {
                    text.push(' ');
                    text.push_str(line);
                }
            }
            Ok(Value::Text(text))
        }
    }
}

/// `rest` without a trailing `# comment`, unless it is quoted.
fn without_comment(rest: &str) -> &str {
    if rest.starts_with(['"', '\'']) {
        return rest;
    }
    if rest.starts_with('#') {
        return "";
    }
    rest.find(" #")
        .and_then(|end| rest.get(..end))
        .unwrap_or(rest)
        .trim_end()
}

/// A block value: `|` keeps the lines, `>` folds them into one, as YAML does.
fn block(header: &str, lines: &[&str]) -> Result<String, String> {
    let style = header.get(..1).unwrap_or_default();
    let chomp = header.get(1..).unwrap_or_default();
    if !matches!(chomp, "" | "-" | "+") {
        return Err(format!(
            "{header:?} is not a block style glave reads (|, |-, >, >-)"
        ));
    }
    let indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.len()
                .saturating_sub(line.trim_start_matches(' ').len())
        })
        .min()
        .unwrap_or(0);
    let lines: Vec<&str> = lines
        .iter()
        .map(|line| line.get(indent..).unwrap_or_default())
        .collect();
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|before| lines.get(before));
        let joins = style == ">"
            && !line.is_empty()
            && !line.starts_with(' ')
            && previous.is_some_and(|before| !before.is_empty() && !before.starts_with(' '));
        if index > 0 {
            text.push(if joins { ' ' } else { '\n' });
        }
        text.push_str(line);
    }
    let text = match chomp {
        "+" => format!("{text}\n"),
        "-" => text.trim_end_matches('\n').to_owned(),
        _ => format!("{}\n", text.trim_end_matches('\n')),
    };
    Ok(text)
}

/// A value in quotes: `'` doubles a quote to escape it, `"` takes backslash escapes.
fn quoted(rest: &str, quote: char) -> Result<String, String> {
    let mut chars = rest.chars().skip(1);
    let mut text = String::new();
    loop {
        match chars.next() {
            None => return Err("its quote is not closed".to_owned()),
            Some(c) if c == quote => {
                if quote == '\'' && chars.clone().next() == Some('\'') {
                    chars.next();
                    text.push('\'');
                    continue;
                }
                break;
            }
            Some('\\') if quote == '"' => text.push(escape(&mut chars)?),
            Some(c) => text.push(c),
        }
    }
    let after: String = chars.collect();
    if !without_comment(&format!(" {after}")).trim().is_empty() {
        return Err(format!("{after:?} follows the closing quote"));
    }
    Ok(text)
}

/// One backslash escape in a double-quoted value.
fn escape(chars: &mut impl Iterator<Item = char>) -> Result<char, String> {
    let c = chars.next().ok_or("it ends in a backslash")?;
    Ok(match c {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        '0' => '\0',
        'b' => '\u{8}',
        'f' => '\u{c}',
        '"' | '\\' | '/' | ' ' => c,
        'u' => {
            let hex: String = chars.take(4).collect();
            u32::from_str_radix(&hex, 16)
                .ok()
                .filter(|_| hex.len() == 4)
                .and_then(char::from_u32)
                .ok_or_else(|| format!("\\u{hex} is not a character"))?
        }
        other => return Err(format!("\\{other} is not an escape glave reads")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn fields(text: &str) -> Result<BTreeMap<String, Value>, String> {
        parse(text).map(|document| document.fields)
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn plain_quoted_and_block_values_are_read() {
        let document = parse(
            "---\nname: pdf # a comment\ndescription: >-\n  Fills PDF forms.\n  Use for PDFs.\n\
             license: 'it''s MIT'\ncompatibility: \"needs \\\"qpdf\\\"\"\nnotes: |\n  one\n  two\n\
             metadata:\n  author: me\nallowed-tools: [read, bash]\n---\n# Body\n",
        )
        .unwrap();
        let get = |key: &str| document.fields.get(key).cloned();
        assert_eq!(get("name"), Some(text("pdf")));
        assert_eq!(
            get("description"),
            Some(text("Fills PDF forms. Use for PDFs."))
        );
        assert_eq!(get("license"), Some(text("it's MIT")));
        assert_eq!(get("compatibility"), Some(text("needs \"qpdf\"")));
        assert_eq!(get("notes"), Some(text("one\ntwo\n")));
        assert_eq!(get("metadata"), Some(Value::Other));
        assert_eq!(get("allowed-tools"), Some(Value::Other));
        assert_eq!(document.body, "# Body\n");
    }

    #[test]
    fn a_plain_value_may_carry_on_over_indented_lines() {
        let read = fields("---\ndescription: Reads logs\n  and sums them up.\n---\n").unwrap();
        assert_eq!(
            read.get("description"),
            Some(&text("Reads logs and sums them up."))
        );
    }

    #[test]
    fn windows_line_endings_and_a_byte_order_mark_are_read() {
        let document = parse("\u{feff}---\r\nname: a\r\n---\r\nbody\r\n").unwrap();
        assert_eq!(document.fields.get("name"), Some(&text("a")));
        assert_eq!(document.body, "body\r\n");
    }

    #[test]
    fn what_is_not_frontmatter_is_an_error() {
        let cases = [
            ("# Title\n", "does not start"),
            ("---\nname: a\n", "no closing"),
            ("---\nname: a\nname: b\n---\n", "given twice"),
            ("---\n  name: a\n---\n", "indented"),
            ("---\njust words\n---\n", "line 2 is not"),
            ("---\nname: \"open\n---\n", "not closed"),
            ("---\nname: 'a' b\n---\n", "follows the closing quote"),
            ("---\nname: \"\\q\"\n---\n", "\\q"),
            ("---\nname: *alias\n---\n", "anchors"),
            ("---\nname: |2\n  a\n---\n", "block style"),
            ("---\nmetadata:\n\tkey: a\n---\n", "tabs"),
        ];
        for (input, why) in cases {
            let error = fields(input).unwrap_err();
            assert!(error.contains(why), "{input:?}: {error}");
        }
    }

    proptest! {
        #[test]
        fn any_text_written_in_double_quotes_reads_back(value in "\\PC*") {
            let quoted = serde_json::to_string(&value).unwrap();
            let read = fields(&format!("---\ndescription: {quoted}\n---\n")).unwrap();
            prop_assert_eq!(read.get("description"), Some(&Value::Text(value)));
        }

        #[test]
        fn any_input_is_read_or_refused_without_panicking(input in "(---\n)?[ -~\n\t]{0,200}") {
            let _ = parse(&input);
        }
    }
}
