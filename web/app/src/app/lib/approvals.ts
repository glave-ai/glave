/** What an approval asks the person: one plain sentence, and what would run. */
export interface Ask {
  readonly sentence: string;
  /** What would run, shown in monospace: the command, the file, the address. */
  readonly detail: string;
}

/** The sentence for each of the harness's own tools; any other tool's is `OTHER`. */
const SENTENCES: Readonly<Record<string, string>> = {
  bash: 'Run this command?',
  edit: 'Edit this file?',
  write: 'Write this file?',
  read: 'Read this file?',
  web_fetch: 'Open this page?',
};

const OTHER = 'Allow this?';

/**
 * What an approval for `tool` asks, from the summary the harness wrote for it (`bash: cargo
 * test`, `edit src/main.rs`): a sentence for the tool, and the summary without the tool's name
 * in front, which the sentence already says. The summary is only what would run; why a caution
 * asks comes apart from it, already in the server's words (`ApprovalItem.caution`).
 */
export function askOf(tool: string, summary: string): Ask {
  const prefixes = [`${tool}: `, `${tool} `];
  const prefix = prefixes.find((each) => tool !== '' && summary.startsWith(each));
  return {
    sentence: SENTENCES[tool] ?? OTHER,
    detail: prefix === undefined ? summary : summary.slice(prefix.length),
  };
}
