//! A script as a list of the commands in it, found with tree-sitter-bash wherever they are:
//! in pipelines and lists, subshells, substitutions, loops, and scripts given to `bash -c`.
//! Each command keeps its words, the files it writes by redirection, and the folder it runs
//! in when that can be known.

use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser};

use crate::paths::{basename, resolve};

/// How deep `bash -c` scripts may nest before the script counts as unreadable.
const MOST_NESTING: usize = 8;

/// Commands that run a script given as an argument.
const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash", "ksh"];

/// Programs that fetch from the network.
const FETCHERS: &[&str] = &["curl", "wget", "fetch"];

/// Programs that run whatever code they are given.
const INTERPRETERS: &[&str] = &[
    "bash", "sh", "zsh", "dash", "ksh", "fish", "python", "python3", "perl", "ruby", "node", "php",
];

/// One word of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Word {
    /// Known before the command runs.
    Literal(String),
    /// Depends on a variable, a substitution or a glob the shell expands: its source text.
    Dynamic(String),
}

impl Word {
    pub(crate) fn literal(&self) -> Option<&str> {
        match self {
            Self::Literal(text) => Some(text),
            Self::Dynamic(_) => None,
        }
    }

    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Literal(text) | Self::Dynamic(text) => text,
        }
    }
}

/// One command in a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Command {
    pub(crate) words: Vec<Word>,
    /// Files the command writes to by redirection (`>`, `>>`, `&>`).
    pub(crate) writes: Vec<Word>,
    /// Files the command reads by redirection (`<`).
    pub(crate) reads: Vec<Word>,
    /// The folder it runs in; `None` once a `cd` to somewhere unknown came before it.
    pub(crate) cwd: Option<PathBuf>,
}

/// What a script holds.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub(crate) commands: Vec<Command>,
    /// Only plain commands joined by `&&`, `||`, `;` or `|`, with literal words and no
    /// redirection: the only scripts that can be proven read-only.
    pub(crate) plain: bool,
    /// Something fetched from the network runs as code: `curl … | sh`, `bash <(curl …)`.
    pub(crate) runs_fetched_code: bool,
    /// `bash -c` nested deeper than glave reads.
    pub(crate) too_deep: bool,
}

/// Parses `script`, run in `cwd`.
pub(crate) fn parse(script: &str, cwd: &Path) -> Parsed {
    let mut parsed = Parsed {
        plain: true,
        ..Parsed::default()
    };
    let mut walk = Walk {
        cwd: Some(cwd.to_path_buf()),
        parsed: &mut parsed,
    };
    walk.script(script, 0);
    parsed
}

struct Walk<'p> {
    cwd: Option<PathBuf>,
    parsed: &'p mut Parsed,
}

impl Walk<'_> {
    fn script(&mut self, script: &str, depth: usize) {
        if depth > MOST_NESTING {
            self.parsed.too_deep = true;
            return;
        }
        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .is_err()
        {
            self.parsed.plain = false;
            return;
        }
        let Some(tree) = parser.parse(script, None) else {
            self.parsed.plain = false;
            return;
        };
        let root = tree.root_node();
        if root.has_error() {
            self.parsed.plain = false;
        }
        self.node(root, script.as_bytes(), depth);
    }

    fn node(&mut self, node: Node<'_>, source: &[u8], depth: usize) {
        if !plain_kind(node.kind()) {
            self.parsed.plain = false;
        }
        match node.kind() {
            "command" => self.command(node, &[], &[], source, depth),
            "redirected_statement" => self.redirected(node, source, depth),
            "pipeline" => {
                self.pipeline(node, source);
                self.children(node, source, depth);
            }
            _ => self.children(node, source, depth),
        }
    }

    fn children(&mut self, node: Node<'_>, source: &[u8], depth: usize) {
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        for child in children {
            self.node(child, source, depth);
        }
    }

    /// A statement with redirections: the files it writes and reads go with its command.
    fn redirected(&mut self, node: Node<'_>, source: &[u8], depth: usize) {
        self.parsed.plain = false;
        let mut writes = Vec::new();
        let mut reads = Vec::new();
        let mut cursor = node.walk();
        for redirect in node.children_by_field_name("redirect", &mut cursor) {
            if redirect.kind() != "file_redirect" {
                continue;
            }
            let Some(destination) = redirect.child_by_field_name("destination") else {
                continue;
            };
            let operator = redirect
                .utf8_text(source)
                .unwrap_or_default()
                .trim_start_matches(|c: char| c.is_ascii_digit());
            let target = word(destination, source);
            if operator.starts_with('<') && !operator.starts_with("<<") {
                reads.push(target);
            } else if !operator.starts_with(">&") {
                writes.push(target);
            }
        }
        match node.child_by_field_name("body") {
            Some(body) if body.kind() == "command" => {
                self.command(body, &writes, &reads, source, depth);
            }
            Some(body) => {
                self.node(body, source, depth);
                // Writes from a compound body, such as `{ …; } > file`, still count.
                self.parsed.commands.push(Command {
                    words: Vec::new(),
                    writes,
                    reads,
                    cwd: self.cwd.clone(),
                });
            }
            None => {}
        }
    }

    fn command(
        &mut self,
        node: Node<'_>,
        writes: &[Word],
        reads: &[Word],
        source: &[u8],
        depth: usize,
    ) {
        let mut words = Vec::new();
        let mut cursor = node.walk();
        let parts: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        for part in &parts {
            match part.kind() {
                // Assignments before the command change its environment, not what it is.
                "variable_assignment" => self.parsed.plain = false,
                "command_name"
                | "word"
                | "string"
                | "raw_string"
                | "concatenation"
                | "number"
                | "simple_expansion"
                | "expansion"
                | "command_substitution"
                | "process_substitution"
                | "ansi_c_string"
                | "arithmetic_expansion" => {
                    let target = if part.kind() == "command_name" {
                        part.named_child(0).unwrap_or(*part)
                    } else {
                        *part
                    };
                    words.push(word(target, source));
                }
                _ => {}
            }
            // Commands inside substitutions run too.
            self.substitutions(*part, source, depth);
        }
        self.nested(&words, &parts, source, depth);
        let cwd = self.cwd.clone();
        self.change_folder(&words);
        self.parsed.commands.push(Command {
            words,
            writes: writes.to_vec(),
            reads: reads.to_vec(),
            cwd,
        });
    }

    fn substitutions(&mut self, node: Node<'_>, source: &[u8], depth: usize) {
        match node.kind() {
            "command_substitution" | "process_substitution" => {
                self.parsed.plain = false;
                self.children(node, source, depth);
            }
            _ => {
                let mut cursor = node.walk();
                let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
                for child in children {
                    self.substitutions(child, source, depth);
                }
            }
        }
    }

    /// `bash -c '…'` and `eval '…'` run a script of their own, which is read the same way;
    /// a shell or interpreter fed by a fetch from the network runs fetched code.
    fn nested(&mut self, words: &[Word], parts: &[Node<'_>], source: &[u8], depth: usize) {
        let Some(name) = words.first().and_then(Word::literal).map(basename) else {
            return;
        };
        if INTERPRETERS.contains(&name)
            && parts
                .iter()
                .skip(1)
                .any(|part| contains_fetch(*part, source))
        {
            self.parsed.runs_fetched_code = true;
        }
        let script = if SHELLS.contains(&name) {
            let flag = words.iter().skip(1).position(|word| {
                word.literal()
                    .is_some_and(|w| w.starts_with('-') && !w.starts_with("--") && w.contains('c'))
            });
            flag.and_then(|at| words.get(at.saturating_add(2)))
                .map(Word::text)
        } else if name == "eval" {
            words.get(1).map(Word::text)
        } else {
            None
        };
        if let Some(script) = script {
            self.parsed.plain = false;
            let saved = self.cwd.clone();
            self.script(strip_quotes(script), depth.saturating_add(1));
            self.cwd = saved;
        }
    }

    /// `cd` moves the folder later commands run in.
    fn change_folder(&mut self, words: &[Word]) {
        if words.first().and_then(Word::literal) != Some("cd") {
            return;
        }
        self.cwd = match (words.get(1), &self.cwd) {
            (Some(Word::Literal(to)), Some(cwd)) => Some(resolve(cwd, to, None)),
            _ => None,
        };
    }

    /// `curl … | sh`: a fetch earlier in a pipeline feeding a shell or interpreter later.
    fn pipeline(&mut self, node: Node<'_>, source: &[u8]) {
        let mut cursor = node.walk();
        let mut fetched = false;
        for child in node.named_children(&mut cursor) {
            let name = first_word(child, source);
            if fetched
                && name
                    .as_deref()
                    .is_some_and(|name| INTERPRETERS.contains(&name))
            {
                self.parsed.runs_fetched_code = true;
            }
            if name.as_deref().is_some_and(|name| FETCHERS.contains(&name)) {
                fetched = true;
            }
        }
    }
}

/// The node kinds a plain script is made of.
fn plain_kind(kind: &str) -> bool {
    matches!(
        kind,
        "program"
            | "list"
            | "pipeline"
            | "command"
            | "command_name"
            | "word"
            | "string"
            | "string_content"
            | "raw_string"
            | "number"
            | "concatenation"
            | "comment"
    )
}

/// The command's name, after any wrapper, of a pipeline stage.
fn first_word(node: Node<'_>, source: &[u8]) -> Option<String> {
    let command = if node.kind() == "redirected_statement" {
        node.child_by_field_name("body")?
    } else {
        node
    };
    if command.kind() != "command" {
        return None;
    }
    let name = command.child_by_field_name("name")?;
    let text = name.utf8_text(source).ok()?;
    let words: Vec<Word> = std::iter::once(Word::Literal(text.to_owned()))
        .chain(
            command
                .children_by_field_name("argument", &mut command.walk())
                .map(|arg| word(arg, source)),
        )
        .collect();
    crate::rules::unwrapped(&words)
        .first()
        .and_then(Word::literal)
        .map(|name| basename(name).to_owned())
}

/// Whether a fetch from the network runs anywhere inside `node`.
fn contains_fetch(node: Node<'_>, source: &[u8]) -> bool {
    if node.kind() == "command"
        && let Some(name) = node.child_by_field_name("name")
        && let Ok(text) = name.utf8_text(source)
        && FETCHERS.contains(&basename(text))
    {
        return true;
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    children
        .into_iter()
        .any(|child| contains_fetch(child, source))
}

/// A word as the shell would see it, when that can be known without running anything.
fn word(node: Node<'_>, source: &[u8]) -> Word {
    let text = node.utf8_text(source).unwrap_or_default().to_owned();
    match node.kind() {
        "word" | "number" => {
            if text.contains(['*', '?', '[']) {
                Word::Dynamic(text)
            } else {
                Word::Literal(unescape(&text))
            }
        }
        "raw_string" => Word::Literal(strip_quotes(&text).to_owned()),
        "string" => {
            let mut cursor = node.walk();
            let literal = node
                .named_children(&mut cursor)
                .all(|part| part.kind() == "string_content");
            if literal {
                Word::Literal(unescape(strip_quotes(&text)))
            } else {
                Word::Dynamic(text)
            }
        }
        "concatenation" => {
            let mut cursor = node.walk();
            let parts: Vec<Word> = node
                .named_children(&mut cursor)
                .map(|part| word(part, source))
                .collect();
            if parts.iter().all(|part| matches!(part, Word::Literal(_))) {
                Word::Literal(parts.iter().map(Word::text).collect())
            } else {
                Word::Dynamic(text)
            }
        }
        _ => Word::Dynamic(text),
    }
}

fn strip_quotes(text: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

/// Backslash escapes removed, as the shell removes them from an unquoted word.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}
