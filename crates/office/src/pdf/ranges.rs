//! Pages named as a person names them to print: `1-3,7`, `5-` to the last, `3-1` backwards.

/// Pages named in order, each a page or a run of them: its first page and its last, `None`
/// for the document's last page. Numbered from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ranges {
    text: String,
    runs: Vec<(u32, Option<u32>)>,
}

impl Ranges {
    /// The pages `text` names, or `None` when it doesn't name pages this way.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let number = |text: &str| text.trim().parse::<u32>().ok().filter(|page| *page > 0);
        let mut runs = Vec::new();
        for run in text.split(',') {
            runs.push(match run.split_once('-') {
                None => {
                    let page = number(run)?;
                    (page, Some(page))
                }
                Some((first, "")) => (number(first)?, None),
                Some((first, last)) => (number(first)?, Some(number(last)?)),
            });
        }
        Some(Self {
            text: text.split_whitespace().collect(),
            runs,
        })
    }

    /// The page numbers named, in order, in a document of `count` pages; or the first page it
    /// names that the document doesn't have.
    pub(crate) fn pages(&self, count: u32) -> Result<Vec<u32>, u32> {
        let mut pages = Vec::new();
        for &(first, last) in &self.runs {
            let last = last.unwrap_or(count);
            if let Some(&missing) = [first, last].iter().find(|page| **page > count) {
                return Err(missing);
            }
            if first <= last {
                pages.extend(first..=last);
            } else {
                pages.extend((last..=first).rev());
            }
        }
        Ok(pages)
    }
}

impl std::fmt::Display for Ranges {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(text: &str, count: u32) -> Result<Vec<u32>, u32> {
        Ranges::parse(text).unwrap().pages(count)
    }

    #[test]
    fn pages_are_named_one_by_one_in_runs_either_way_and_to_the_last() {
        assert_eq!(pages("1-3,7", 9), Ok(vec![1, 2, 3, 7]));
        assert_eq!(pages("7, 1-2", 9), Ok(vec![7, 1, 2]));
        assert_eq!(pages("8-", 9), Ok(vec![8, 9]));
        assert_eq!(pages("3-1", 9), Ok(vec![3, 2, 1]));
        assert_eq!(pages("2", 2), Ok(vec![2]));
        assert_eq!(Ranges::parse("1-3, 7").unwrap().to_string(), "1-3,7");
    }

    #[test]
    fn a_page_the_document_lacks_is_named() {
        assert_eq!(pages("1-3,7", 5), Err(7));
        assert_eq!(pages("4-6", 5), Err(6));
        assert_eq!(pages("6-", 5), Err(6));
    }

    #[test]
    fn anything_else_is_not_pages() {
        for wrong in ["", "0", "-3", "a", "1-3,", "1--3", "2-x", "1.5", "all"] {
            let parsed = Ranges::parse(wrong);
            assert!(parsed.is_none(), "{wrong:?} parsed as {parsed:?}");
        }
    }
}
