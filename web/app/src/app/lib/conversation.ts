import type { RawEvent, Update } from '@glave/client';
import type { GlzDecision, GlzQuestion, GlzReceiptStatus } from '@glave/glaze';

/** What every item of a thread has: its key, and the `seq` of the event that put it there. */
interface Placed {
  readonly key: string;
  readonly seq: number;
}

/** A message in a thread, as the thread view shows it. */
export interface MessageItem extends Placed {
  readonly kind: 'message';
  readonly from: 'person' | 'model';
  readonly text: string;
  readonly reasoning: string;
  /** Whether the person sent it while the turn ran, and the model read it as it worked. */
  readonly steered: boolean;
  /** The files sent with it, in order. */
  readonly files: readonly SentFile[];
}

/** A file a message was sent with: its name, and its size in bytes. */
export interface SentFile {
  readonly name: string;
  readonly bytes: number;
}

/** A tool call: what it did and what came of it. */
export interface CallItem extends Placed {
  readonly kind: 'call';
  readonly tool: string;
  readonly summary: string;
  readonly status: GlzReceiptStatus;
  readonly output: string;
}

/** An action waiting for, or answered by, the person. */
export interface ApprovalItem extends Placed {
  readonly kind: 'approval';
  readonly approval: string;
  readonly tool: string;
  /** What would run, never why it asks. */
  readonly summary: string;
  /** Why it asks whatever the mode, in the server's sentences; empty when only the mode asks. */
  readonly caution: string;
  /** Whether the command would run outside the sandbox: each one asks, and is allowed once. */
  readonly outsideSandbox: boolean;
  /** The model's own words for why it must run outside the sandbox; empty when it gave none. */
  readonly modelReason: string;
  readonly decision: GlzDecision | null;
}

/** Questions the model asked the person. */
export interface QuestionItem extends Placed {
  readonly kind: 'question';
  readonly call: string;
  readonly questions: readonly GlzQuestion[];
  readonly answers: readonly string[] | null;
}

/** One thing in a thread, in order. */
export type ThreadItem = MessageItem | CallItem | ApprovalItem | QuestionItem;

/** The reply being written while a turn streams, before its message is recorded. */
export interface Draft {
  readonly text: string;
  readonly reasoning: string;
}

export const NO_DRAFT: Draft = { text: '', reasoning: '' };

type Data = Readonly<Record<string, unknown>>;

function isRecord(value: unknown): value is Data {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function text(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

/**
 * `text` without the blank lines at its start and its end, which text drawn as it was written
 * would show as empty space in what holds it; the first line keeps its indent.
 */
export function trimBlankLines(text: string): string {
  return text.replace(/^\s*\n/u, '').trimEnd();
}

/** The text and reasoning of a list of content parts, without blank lines at their ends. */
export function partsText(parts: unknown): { readonly text: string; readonly reasoning: string } {
  const list = Array.isArray(parts) ? (parts as readonly unknown[]) : [];
  const of = (type: string): string =>
    trimBlankLines(
      list
        .filter(isRecord)
        .filter((part) => part['type'] === type)
        .map((part) => text(part['text']))
        .filter((part) => part !== '')
        .join('\n'),
    );
  return { text: of('text'), reasoning: of('reasoning') };
}

/** The files of a message's `parts`, in order. */
export function filesOf(parts: unknown): readonly SentFile[] {
  const list = Array.isArray(parts) ? (parts as readonly unknown[]) : [];
  return list
    .filter(isRecord)
    .filter((part) => part['type'] === 'file')
    .map((part) => {
      const blob = isRecord(part['blob']) ? part['blob'] : {};
      const bytes = typeof blob['bytes'] === 'number' ? blob['bytes'] : 0;
      return { name: text(part['name']) || $localize`:@@conversation.file:file`, bytes };
    });
}

/**
 * What the person wrote in a message: its last text part. A thread's first message also
 * carries, before it, what glave opened the thread with (the project's AGENTS.md).
 */
export function promptOf(parts: unknown): string {
  const list = Array.isArray(parts) ? (parts as readonly unknown[]) : [];
  const texts = list.filter(isRecord).filter((part) => part['type'] === 'text');
  return text(texts.at(-1)?.['text']);
}

/** A tool call's arguments in a line: the first text argument, or the whole of them. */
export function argumentsSummary(argumentsValue: unknown): string {
  if (!isRecord(argumentsValue)) {
    return '';
  }
  const json = argumentsValue['json'];
  if (!isRecord(json)) {
    return text(argumentsValue['malformed']);
  }
  const first = Object.values(json).find((value) => typeof value === 'string');
  const line = typeof first === 'string' ? first : JSON.stringify(json);
  return line.split('\n')[0] ?? '';
}

/** The questions a `question` call asks. */
export function questionsOf(argumentsValue: unknown): readonly GlzQuestion[] {
  const json = isRecord(argumentsValue) ? argumentsValue['json'] : undefined;
  const questions = isRecord(json) ? json['questions'] : undefined;
  if (!Array.isArray(questions)) {
    return [];
  }
  return (questions as readonly unknown[]).filter(isRecord).map((question) => ({
    question: text(question['question']),
    options: (Array.isArray(question['options']) ? (question['options'] as readonly unknown[]) : [])
      .filter(isRecord)
      .map((option) => ({
        label: text(option['label']),
        description: text(option['description']),
      })),
    multiple: question['multiple'] === true,
  }));
}

const DECISIONS: readonly string[] = ['allow_once', 'allow_for_thread', 'deny'];

function isDecision(value: unknown): value is GlzDecision {
  return typeof value === 'string' && DECISIONS.includes(value);
}

/** Whether glave wrote a message the person's side of the conversation holds, such as a note. */
function byGlave(data: Data): boolean {
  const author = data['author'];
  return isRecord(author) && author['kind'] === 'glave';
}

/** What memory calls' results read to this person, by call (`thread/events`' `results`). */
export type Results = ReadonlyMap<string, string>;

/** No memory call's result read again. */
export const NO_RESULTS: Results = new Map();

/** `known` with the words a page of events reads for memory calls' results. */
export function withResults(
  known: Results,
  more: Readonly<Record<string, string>> | undefined,
): Results {
  const read = Object.entries(more ?? {});
  return read.length === 0 ? known : new Map([...known, ...read]);
}

/**
 * Whether `event` is a memory call's result the log keeps by reference, without the text of
 * the memories or past conversations it showed: the server reads its words for the person
 * with `thread/events`.
 */
export function keptByReference(event: RawEvent): boolean {
  const structured = event.data['structured'];
  return event.type === 'tool.call.completed' && isRecord(structured) && 'shown' in structured;
}

/**
 * A call's result as the thread view shows it: the words `read` for it by the server, when it
 * has some, in place of the first part the log keeps (any after it, such as a hook's, stay).
 */
function resultText(output: unknown, read: string | undefined): string {
  if (read === undefined) {
    return partsText(output).text;
  }
  const after = Array.isArray(output) ? partsText(output.slice(1)).text : '';
  return trimBlankLines(after === '' ? read : `${read}\n${after}`);
}

/**
 * A thread's items from its stored events, in order: messages, tool calls with their
 * results, approvals with their decisions, and questions with their answers. A memory call's
 * result reads as `results` has it, and its call is not drawn from the memory's text, which
 * the person may have forgotten since. A message of the person's after the one that started a
 * turn, before the turn ended, is one they sent while it ran: `steered`.
 */
export function threadItems(
  events: readonly RawEvent[],
  results: Results = NO_RESULTS,
): readonly ThreadItem[] {
  const items: ThreadItem[] = [];
  // Whether a turn runs at this point of the log, and whether the person has asked in it.
  let running = false;
  let asked = false;
  const index = new Map<string, number>();
  const put = (item: ThreadItem): void => {
    index.set(item.key, items.length);
    items.push(item);
  };
  const update = (key: string, change: (item: ThreadItem) => ThreadItem): void => {
    const at = index.get(key);
    const item = at === undefined ? undefined : items[at];
    if (at !== undefined && item !== undefined) {
      items[at] = change(item);
    }
  };
  for (const event of events) {
    const data = event.data;
    switch (event.type) {
      case 'turn.started':
        running = true;
        asked = false;
        break;
      case 'turn.ended':
        running = false;
        break;
      case 'message.added': {
        const person = data['role'] === 'user';
        const theirs = person && !byGlave(data);
        const parts = partsText(data['parts']);
        put({
          kind: 'message',
          key: `message-${String(event.seq)}`,
          seq: event.seq,
          from: person ? 'person' : 'model',
          text: person ? promptOf(data['parts']) : parts.text,
          reasoning: parts.reasoning,
          steered: theirs && running && asked,
          files: filesOf(data['parts']),
        });
        asked ||= theirs;
        break;
      }
      case 'tool.call.requested': {
        const call = text(data['call_id']);
        const tool = text(data['tool']);
        if (tool === 'question') {
          put({
            kind: 'question',
            key: `call-${call}`,
            seq: event.seq,
            call,
            questions: questionsOf(data['arguments']),
            answers: null,
          });
        } else {
          put({
            kind: 'call',
            key: `call-${call}`,
            seq: event.seq,
            tool,
            summary: argumentsSummary(data['arguments']),
            status: 'running',
            output: '',
          });
        }
        break;
      }
      case 'tool.call.completed': {
        const call = text(data['call_id']);
        const output = resultText(data['output'], results.get(call));
        const kept = keptByReference(event);
        update(`call-${call}`, (item) => {
          if (item.kind === 'call') {
            const status = data['is_error'] === true ? 'failed' : 'done';
            return kept ? { ...item, status, output, summary: '' } : { ...item, status, output };
          }
          if (item.kind === 'question') {
            return { ...item, answers: answersFrom(output) };
          }
          return item;
        });
        break;
      }
      case 'approval.requested': {
        const approval = text(data['call_id']);
        const outside = data['outside_sandbox'];
        put({
          kind: 'approval',
          key: `approval-${approval}`,
          seq: event.seq,
          approval,
          tool: text(data['tool']),
          summary: text(data['summary']),
          caution: text(data['caution']),
          outsideSandbox: isRecord(outside),
          modelReason: isRecord(outside) ? text(outside['reason']) : '',
          decision: null,
        });
        break;
      }
      case 'approval.decided': {
        const approval = text(data['call_id']);
        const decision = data['decision'];
        update(`approval-${approval}`, (item) =>
          item.kind === 'approval'
            ? { ...item, decision: isDecision(decision) ? decision : 'deny' }
            : item,
        );
        if (decision === 'deny' || decision === 'no_one_there' || decision === 'cancelled') {
          update(`call-${approval}`, (item) =>
            item.kind === 'call' ? { ...item, status: 'declined' } : item,
          );
        }
        break;
      }
      default:
        break;
    }
  }
  return items;
}

/** The draft after a live update: text and reasoning grow; anything else leaves it. */
export function applyUpdate(draft: Draft, update: Update): Draft {
  switch (update.kind) {
    case 'text':
      return { ...draft, text: draft.text + update.text };
    case 'reasoning':
      return { ...draft, reasoning: draft.reasoning + update.text };
    default:
      return draft;
  }
}

/**
 * The steps a running turn takes now, in the server's words, oldest first, after a live
 * update: a tool's step from when the server says it began until it says it finished or did
 * not run; anything else leaves them.
 */
export function stepsAfter(steps: readonly string[], update: Update): readonly string[] {
  switch (update.kind) {
    case 'tool_started':
      return [...steps, update.summary];
    case 'tool_finished':
    case 'tool_declined': {
      const at = steps.indexOf(update.summary);
      return at === -1 ? steps : steps.filter((_, index) => index !== at);
    }
    default:
      return steps;
  }
}

/** `events` with `more` added, each `seq` once, in `seq` order. */
export function mergeEvents(
  events: readonly RawEvent[],
  more: readonly RawEvent[],
): readonly RawEvent[] {
  const bySeq = new Map<number, RawEvent>();
  for (const event of [...events, ...more]) {
    bySeq.set(event.seq, event);
  }
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}

/** The person's answers, read back from what a `question` call's result told the model. */
export function answersFrom(output: string): readonly string[] {
  return output
    .split('\n')
    .slice(1)
    .map((line) => line.split(' → ').slice(1).join(' → '))
    .map((answer) => (answer === '(no answer)' ? '' : answer));
}
