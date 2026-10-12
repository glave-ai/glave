//! How memories read to the model: the block at the start of a thread, after the project's
//! instructions, and the note when a thread is returned to after its memories changed. Both are
//! built only from ids, scopes, dates and text, so the same versions always read the same.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glave_event::{At, MemoryId};

use super::Remembered;

const START: &str = "# Memory\n\n<MEMORY>\nWhat you saved with the memory tools in earlier \
     threads, about the person (personal) and this space (space). The person can see, edit and \
     forget these. They are background: the person's messages and the project's instructions \
     come first; when a memory conflicts with the instructions, follow the instructions and say \
     so. When the person corrects or updates one, replace it (`remember` with `replaces`, or \
     `edit`); when they ask you to forget one, `forget` it. Each takes the id in brackets.";

const END: &str = "</MEMORY>";

const UNSURE: &str = "Unsure: tidy-up found memories that disagree and could not tell which \
     holds. Before relying on either, ask the person which is right (with the question tool \
     when you can), then forget or replace the wrong one:";

const NOTE: &str = "[Memory updates since we last spoke]";

const NOTE_END: &str = "[End of memory updates]";

/// One line of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    Added(Remembered),
    Changed(Remembered),
    Forgotten(MemoryId),
    /// A contradiction tidy-up left for the person.
    Asked([MemoryId; 2]),
}

/// The block that opens a thread, or `None` when there is nothing to show: each memory, then
/// any contradiction left for the person.
pub(crate) fn start(memories: &[Remembered], questions: &[[MemoryId; 2]]) -> Option<String> {
    if memories.is_empty() {
        return None;
    }
    let mut text = format!("{START}\n");
    for memory in memories {
        let _ = write!(text, "\n- {}", line(memory));
    }
    if !questions.is_empty() {
        let _ = write!(text, "\n\n{UNSURE}");
        for [one, other] in questions {
            let _ = write!(text, "\n- [{one}] or [{other}]");
        }
    }
    let _ = write!(text, "\n{END}");
    Some(text)
}

/// The note for a thread returned to: what changed since it last looked.
pub(crate) fn note(lines: &[Line]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let mut text = NOTE.to_owned();
    for change in lines {
        let _ = match change {
            Line::Added(memory) => write!(text, "\n- added {}", line(memory)),
            Line::Changed(memory) => write!(text, "\n- changed {}", line(memory)),
            Line::Forgotten(memory) => write!(text, "\n- forgotten [{memory}]"),
            Line::Asked([one, other]) => write!(
                text,
                "\n- unsure which holds: [{one}] or [{other}]; ask the person, then forget or \
                 replace the wrong one"
            ),
        };
    }
    let _ = write!(text, "\n{NOTE_END}");
    Some(text)
}

/// `[id] (scope, saved date, edited date) text`: the date shows how old it is, so a stale
/// memory looks stale.
fn line(memory: &Remembered) -> String {
    let mut dates = format!("saved {}", day(memory.saved_at));
    if let Some(edited) = memory.edited_at {
        let _ = write!(dates, ", edited {}", day(edited));
    }
    format!(
        "[{}] ({}, {dates}) {}",
        memory.memory,
        memory.scope.as_str(),
        memory.text
    )
}

fn day(at: At) -> String {
    at.to_string().chars().take(10).collect()
}

/// What changed from `known` (ids and versions the thread knows) to `now`, in `now`'s order,
/// then the forgotten ones in id order.
pub(crate) fn changes(known: &BTreeMap<MemoryId, u32>, now: &[Remembered]) -> Vec<Line> {
    let mut lines = Vec::new();
    for memory in now {
        match known.get(&memory.memory) {
            None => lines.push(Line::Added(memory.clone())),
            Some(version) if *version != memory.version => {
                lines.push(Line::Changed(memory.clone()));
            }
            Some(_) => {}
        }
    }
    for memory in known.keys() {
        if !now.iter().any(|current| current.memory == *memory) {
            lines.push(Line::Forgotten(*memory));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use glave_event::MemoryScope;

    use super::*;

    fn memory(n: u128, version: u32, text: &str) -> Remembered {
        Remembered {
            memory: MemoryId::try_from(uuid::Uuid::from_u128(
                0x0192_f5c3_0000_7000_8000_0000_0000_0000 | n,
            ))
            .unwrap(),
            version,
            scope: MemoryScope::Space,
            text: text.into(),
            saved_at: At::try_from("2026-10-02T10:00:00Z".to_owned()).unwrap(),
            edited_at: (version > 1)
                .then(|| At::try_from("2026-10-05T10:00:00Z".to_owned()).unwrap()),
            untrusted_read: false,
            source: None,
        }
    }

    #[test]
    fn the_start_lists_each_memory_with_its_id_scope_and_dates() {
        assert_eq!(start(&[], &[]), None);
        let text = start(
            &[memory(1, 1, "use tabs"), memory(2, 2, "make db first")],
            &[],
        )
        .unwrap();
        assert!(text.starts_with("# Memory\n\n<MEMORY>\n"), "{text}");
        assert!(
            text.ends_with(
                "\n- [0192f5c3-0000-7000-8000-000000000001] (space, saved 2026-10-02) use tabs\
                 \n- [0192f5c3-0000-7000-8000-000000000002] (space, saved 2026-10-02, edited \
                 2026-10-05) make db first\n</MEMORY>"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_note_lists_what_was_added_changed_and_forgotten() {
        let known: BTreeMap<MemoryId, u32> =
            [(memory(1, 1, "").memory, 1), (memory(2, 1, "").memory, 1)]
                .into_iter()
                .collect();
        let now = [memory(2, 2, "edited"), memory(3, 1, "new")];
        let lines = changes(&known, &now);
        let text = note(&lines).unwrap();
        assert_eq!(
            text,
            "[Memory updates since we last spoke]\
             \n- changed [0192f5c3-0000-7000-8000-000000000002] (space, saved 2026-10-02, edited 2026-10-05) edited\
             \n- added [0192f5c3-0000-7000-8000-000000000003] (space, saved 2026-10-02) new\
             \n- forgotten [0192f5c3-0000-7000-8000-000000000001]\
             \n[End of memory updates]"
        );
        assert_eq!(
            note(&changes(&known, &[memory(1, 1, ""), memory(2, 1, "")])),
            None
        );
    }
}
