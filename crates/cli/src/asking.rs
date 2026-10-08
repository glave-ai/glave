//! The model's questions in the terminal: each shown with numbered choices, answered with a
//! number, several numbers, or the person's own words.

use std::fmt::Write as _;

use glave_harness::Question;

use crate::terminal::{DIM, paint};

/// The question as the terminal shows it, ending where the answer is typed.
pub fn shown(color: bool, question: &Question) -> String {
    let mut text = format!(
        "\n\n{} {}\n",
        paint(color, "\x1b[33m", "question"),
        question.question
    );
    for (index, choice) in question.options.iter().enumerate() {
        let number = index.saturating_add(1);
        let _ = write!(text, "  {number}. {}", choice.label);
        if let Some(description) = &choice.description {
            let _ = write!(text, " {}", paint(color, DIM, &format!("— {description}")));
        }
        text.push('\n');
    }
    let how = match (question.options.is_empty(), question.multiple) {
        (true, _) => "your answer",
        (false, false) => "a number, or your own words",
        (false, true) => "numbers such as 1,3, or your own words",
    };
    format!("{text}  {} › ", paint(color, DIM, how))
}

/// The answer to `question` in what was typed: the chosen labels when it was choice numbers,
/// otherwise the words themselves.
pub fn answer(question: &Question, typed: &str) -> String {
    let typed = typed.trim();
    let numbers: Option<Vec<usize>> = typed
        .split([',', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<usize>().ok())
        .collect();
    let labels: Option<Vec<&str>> =
        numbers
            .filter(|numbers| !numbers.is_empty())
            .and_then(|numbers| {
                numbers
                    .iter()
                    .map(|number| {
                        number
                            .checked_sub(1)
                            .and_then(|index| question.options.get(index))
                            .map(|choice| choice.label.as_str())
                    })
                    .collect()
            });
    match labels {
        Some(labels) if question.multiple || labels.len() == 1 => labels.join(", "),
        _ => typed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_harness::Choice;

    fn question(multiple: bool) -> Question {
        let choice = |label: &str| Choice {
            label: label.to_owned(),
            description: None,
        };
        Question {
            question: "Which?".to_owned(),
            options: vec![choice("Postgres"), choice("SQLite"), choice("Both")],
            multiple,
        }
    }

    #[test]
    fn numbers_pick_choices_and_words_stand_as_they_are() {
        let one = question(false);
        assert_eq!(answer(&one, " 2 "), "SQLite");
        assert_eq!(answer(&one, "MySQL, please"), "MySQL, please");
        assert_eq!(answer(&one, "4"), "4", "no such choice: taken as words");
        assert_eq!(answer(&one, "1,2"), "1,2", "one choice only");
        assert_eq!(answer(&question(true), "1, 3"), "Postgres, Both");
        assert_eq!(answer(&one, ""), "");
    }

    #[test]
    fn choices_are_numbered() {
        let shown = shown(false, &question(true));
        assert!(shown.contains("question Which?\n  1. Postgres\n  2. SQLite\n  3. Both\n"));
        assert!(shown.ends_with("numbers such as 1,3, or your own words › "));
    }
}
