use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};

use super::*;

#[test]
fn every_sentence_has_an_entry_that_writes_exactly_its_values() {
    let mut ids = BTreeSet::new();
    for (id, values, english) in Sentence::ALL {
        assert!(ids.insert(*id), "{id} is in the table twice");
        let resource = FluentResource::try_new(format!("{id} = {english}\n"))
            .unwrap_or_else(|(_, errors)| panic!("{id} is not one Fluent message: {errors:?}"));
        let mut bundle = FluentBundle::new(Vec::new());
        bundle.set_use_isolating(false);
        bundle.add_resource(resource).unwrap();
        let pattern = bundle.get_message(id).and_then(|message| message.value());
        let pattern = pattern.unwrap_or_else(|| panic!("{id} has no words"));
        let mut args = FluentArgs::new();
        for value in *values {
            args.set(*value, format!("<{value}>"));
        }
        let mut errors = Vec::new();
        let said = bundle.format_pattern(pattern, Some(&args), &mut errors);
        // A value the words name and the sentence does not give is an error.
        assert!(errors.is_empty(), "{id}: {errors:?}");
        for value in *values {
            assert!(
                said.contains(&format!("<{value}>")),
                "{id} never writes its {value}"
            );
        }
    }
}

#[test]
fn the_pseudo_locales_write_every_sentence_from_its_english() {
    let sentence = Sentence::FolderNotOpened {
        path: "/srv/atlas".into(),
        reason: "denied".into(),
    };
    assert_eq!(
        say(Language::English, &sentence),
        "could not open the folder at /srv/atlas: denied"
    );
    let accented = say(Language::Accented, &sentence);
    assert!(accented.starts_with("[çöûļð ñöţ öþéñ ţĥé ƒöļðéŕ åţ /srv/atlas: denied "));
    assert!(accented.ends_with(']'));
    assert!(say(Language::RightToLeft, &sentence).contains("\u{202e}folder\u{202c}"));
}

#[test]
fn an_answer_is_written_again_only_where_it_holds_the_servers_own_words() {
    let mut refused: Result<Reply, ApiError> = Err(ApiError::new(
        crate::protocol::code::NO_THREAD,
        Sentence::NoThread {
            thread: "t1".into(),
        },
    ));
    assert!(says(&refused));
    translate(&mut refused, Language::Accented);
    let message = refused.err().map(|error| error.message.to_string());
    assert_eq!(
        message.as_deref(),
        Some("[ñö ţĥŕéåð t1 îñ ţĥîš þŕöĵéçţ one two]")
    );
    // Another's words stay as they are.
    let mut passed: Result<Reply, ApiError> =
        Err(ApiError::new(crate::protocol::code::INTERNAL, "disk full"));
    assert!(!says(&passed));
    translate(&mut passed, Language::Accented);
    assert_eq!(
        passed
            .err()
            .map(|error| error.message.to_string())
            .as_deref(),
        Some("disk full")
    );
    assert!(!says(&Ok(Reply::Done {})));
}

#[test]
fn the_notice_of_a_folder_gone_is_written_again_in_a_list_of_projects() {
    let gone = crate::protocol::GoneFolder {
        project: glave_ports::ProjectId::from_uuid(uuid::Uuid::nil()),
        message: Sentence::FolderGone.into(),
    };
    let mut listed: Result<Reply, ApiError> = Ok(Reply::Projects {
        projects: Vec::new(),
        gone: vec![gone],
    });
    assert!(says(&listed));
    translate(&mut listed, Language::Accented);
    let Ok(Reply::Projects { gone, .. }) = listed else {
        panic!("a list of projects")
    };
    assert!(gone[0].message.starts_with("[Ţĥîš þŕöĵéçţ'š ƒöļðéŕ"));
}

#[test]
fn what_replies_drew_on_is_written_again_in_their_sources() {
    let sourced = crate::protocol::ReplySources {
        reply: glave_event::Seq::new(4).unwrap(),
        sentence: Sentence::Sources { pages: 3 }.into(),
        pages: Vec::new(),
    };
    assert_eq!(&*sourced.sentence, "Sources: 3 pages");
    let mut listed: Result<Reply, ApiError> = Ok(Reply::Sources {
        sources: vec![sourced],
    });
    assert!(says(&listed));
    translate(&mut listed, Language::Accented);
    let Ok(Reply::Sources { sources }) = listed else {
        panic!("sources")
    };
    assert!(
        sources[0].sentence.starts_with("[Šöûŕçéš: 3 þåĝéš"),
        "{}",
        &*sources[0].sentence
    );
}

#[test]
fn what_changes_leave_out_is_written_again_in_their_note() {
    let mut changed: Result<Reply, ApiError> = Ok(Reply::Changes {
        changes: crate::protocol::ThreadChanges {
            files: Vec::new(),
            turns: Vec::new(),
            note: Sentence::ChangesLeftOut.into(),
        },
    });
    assert!(says(&changed));
    translate(&mut changed, Language::Accented);
    let Ok(Reply::Changes { changes }) = changed else {
        panic!("changes")
    };
    assert!(changes.note.starts_with("[Çĥåñĝéš"), "{}", &*changes.note);
}

/// The words of its own the session crate writes in English, by file and what they say: about
/// the message itself, for whoever writes a client (JSON-RPC's own codes), what a client reads
/// of a server's answer, and misuse of the API in process.
const FOR_CLIENT_WRITERS: [(&str, &str); 8] = [
    ("rpc.rs", "a message is longer than"),
    ("rpc.rs", "a watch the connection does not hold"),
    ("rpc.rs", "the server failed:"),
    ("rpc.rs", "the server's page of events"),
    ("rpc.rs", "the server's answer is not one"),
    ("http.rs", "the Glave-Method header says"),
    ("dispatch.rs", "is followed with Server::watch"),
    ("dispatch.rs", "answered at once: only a turn"),
];

#[test]
fn every_sentence_the_server_writes_is_in_the_table() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut written = Vec::new();
    for file in sources(&src) {
        let relative = file
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        // The client is the CLI's side, which writes English; tests may say what they like.
        let tests = file.file_name().is_some_and(|name| name == "tests.rs");
        if tests || relative.starts_with("client") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        for (at, _) in code
            .match_indices("ApiError::new(")
            .chain(code.match_indices("Self::new(code"))
        {
            let message = second_argument(code.get(at..).unwrap_or_default());
            let opening: String = message.chars().take(120).collect();
            let english = FOR_CLIENT_WRITERS
                .iter()
                .any(|(file, words)| relative == *file && opening.contains(words));
            if (message.starts_with('"') || message.starts_with("format!")) && !english {
                written.push(format!("{relative}: {opening}"));
            }
        }
    }
    assert!(
        written.is_empty(),
        "sentences outside the table: {written:#?}"
    );
}

/// The `.rs` files under `dir`, in order.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(sources(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// The second argument of the call `call` starts with, trimmed: what follows its first comma
/// outside any brackets.
fn second_argument(call: &str) -> &str {
    let mut depth = 0;
    for (at, each) in call.char_indices() {
        match each {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 1 => return "",
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 1 => return call.get(at + 1..).unwrap_or_default().trim_start(),
            _ => {}
        }
    }
    ""
}

#[test]
fn why_a_folder_is_no_project_s_is_written_again_in_its_listing() {
    let mut listed: Result<Reply, ApiError> = Ok(Reply::Folders {
        path: "/home/ada".into(),
        parent: Some("/home".into()),
        folders: Vec::new(),
        refused: Some(Sentence::HomeHoldsChats.into()),
    });
    assert!(says(&listed));
    translate(&mut listed, Language::Accented);
    let Ok(Reply::Folders {
        refused: Some(refused),
        ..
    }) = listed
    else {
        panic!("a listing")
    };
    let english = say(Language::English, &Sentence::HomeHoldsChats);
    assert!(refused.starts_with('[') && *refused != english, "{refused}");
    let plain: Result<Reply, ApiError> = Ok(Reply::Folders {
        path: "/home/ada/code".into(),
        parent: None,
        folders: Vec::new(),
        refused: None,
    });
    assert!(!says(&plain));
}
