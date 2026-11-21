import type { RawEvent, Update } from '@glave/client';
import type {
  GlzDecision,
  GlzQuestion,
  GlzReceiptStatus,
  GlzServiceDetail,
  GlzServiceField,
} from '@glave/glaze';

/** What every item of a thread has: its key, and the `seq` of the event that put it there. */
interface Placed {
  readonly key: string;
  readonly seq: number;
}

/**
 * What the model's reasoning came to, as the server says: its headline, the model's own words,
 * and how long it reasoned, in milliseconds; each `null` while the server has not said.
 */
export interface Thought {
  readonly headline: string | null;
  readonly ms: number | null;
}

export const NO_THOUGHT: Thought = { headline: null, ms: null };

/** A message in a thread, as the thread view shows it. */
export interface MessageItem extends Placed {
  readonly kind: 'message';
  readonly from: 'person' | 'model';
  readonly text: string;
  readonly reasoning: string;
  /** What its reasoning came to, as the server recorded it with the message. */
  readonly thought: Thought;
  /** Whether the person sent it while the turn ran, and the model read it as it worked. */
  readonly steered: boolean;
  /** The files sent with it, in order. */
  readonly files: readonly SentFile[];
  /** Whether it is the person's Build it of a proposed plan (`plan.approved`), with no text. */
  readonly built?: boolean;
}

/** A file a message was sent with: its name, and its size in bytes. */
export interface SentFile {
  readonly name: string;
  readonly bytes: number;
}

/** A tool call: what it did and what came of it. */
export interface CallItem extends Placed {
  readonly kind: 'call';
  /** Its id, as its events name it. */
  readonly call: string;
  readonly tool: string;
  /** The step in a line, as the server recorded it: `bash: cargo test`. */
  readonly summary: string;
  readonly status: GlzReceiptStatus;
  readonly output: string;
  /** The images its result holds, such as one the agent made, in order. */
  readonly images: readonly HeldImage[];
}

/** An image a thread holds, as a tool's result names it: read back by its content's hash. */
export interface HeldImage {
  readonly sha256: string;
  readonly mediaType: string;
  /** The name it is saved as. */
  readonly name: string;
}

/** The images a browser draws, by media type. */
const DRAWN: readonly string[] = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];

/** An action waiting for, or answered by, the person. */
export interface ApprovalItem extends Placed {
  readonly kind: 'approval';
  readonly approval: string;
  readonly tool: string;
  /** What would run, never why it asks. */
  readonly summary: string;
  /**
   * What it asks, in the server's sentence ("Run this command?"); for a request recorded before
   * the server said one, its summary.
   */
  readonly ask: string;
  /** What would run, as the server gives it apart from its sentence: `cargo test`. */
  readonly detail: string;
  /** Why it asks whatever the mode, in the server's sentences; empty when only the mode asks. */
  readonly caution: string;
  /**
   * Whether the command would run outside the sandbox, or in it with the network
   * (`withNetwork`): each one asks, and is allowed once.
   */
  readonly outsideSandbox: boolean;
  /** Whether it would run in the sandbox with the network, which commands otherwise go without. */
  readonly withNetwork: boolean;
  /** The model's own words for why it must run outside the sandbox; empty when it gave none. */
  readonly modelReason: string;
  /** For a card asking to repeat work (`ask-repeat`), what it asks to repeat; absent otherwise. */
  readonly repeat?: RepeatAsk;
  readonly decision: GlzDecision | null;
}

/** A line of a schedule's card, by what it says, as the server gives it. */
export interface CardLine {
  /**
   * What it says: when it runs, the chat its runs go to, or what each run starts with; `null`
   * for a line of a card recorded before the server gave them typed.
   */
  readonly says: 'when' | 'chat' | 'prompt' | null;
  readonly text: string;
}

/** What a card asking to repeat work asks, and what came of it, as the server says. */
export interface RepeatAsk {
  /** The card's lines under its question, as the server gives them. */
  readonly lines: readonly CardLine[];
  /** The schedule's own chat, once the call made it (its result's `structured`); else `null`. */
  readonly chat: string | null;
}

/** Questions the model asked the person. */
export interface QuestionItem extends Placed {
  readonly kind: 'question';
  readonly call: string;
  readonly questions: readonly GlzQuestion[];
  readonly answers: readonly string[] | null;
}

/** What a connected service asks the person in the middle of a call. */
export interface ServiceItem extends Placed {
  readonly kind: 'service';
  readonly call: string;
  /** The service, by the name the person knows it by. */
  readonly service: string;
  /** Who asks, in the server's sentence: "GitHub asks you, not the model". */
  readonly ask: string;
  /** Where the answer goes, in the server's sentences. */
  readonly note: string;
  /** What it asks, in the service's own words. */
  readonly message: string;
  readonly fields: readonly GlzServiceField[];
  /** What glave's own question adds (a skill's text and files), as the server sent it. */
  readonly detail: GlzServiceDetail | null;
  /** What came of it, in the server's line; `null` while it waits. */
  readonly told: string | null;
}

/**
 * A connector of glave's catalog a search found that the person may connect and has not, as the
 * server suggests it: the person connects it, never the model.
 */
export interface SuggestionItem extends Placed {
  readonly kind: 'suggestion';
  /** The connector, by its id in the catalog: `linear`. */
  readonly connector: string;
  /** What the server tells the person of it, in its sentences. */
  readonly told: string;
}

/** One thing in a thread, in order. */
export type ThreadItem =
  MessageItem | CallItem | ApprovalItem | QuestionItem | ServiceItem | SuggestionItem;

/** The reply being written while a turn streams, before its message is recorded. */
export interface Draft {
  readonly text: string;
  readonly reasoning: string;
  /** What its reasoning has come to so far, as the server says while it streams. */
  readonly thought: Thought;
}

export const NO_DRAFT: Draft = { text: '', reasoning: '', thought: NO_THOUGHT };

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

/** What a message's reasoning came to, as `message.added` records it (`thought`). */
export function thoughtOf(value: unknown): Thought {
  if (!isRecord(value)) {
    return NO_THOUGHT;
  }
  const headline = text(value['headline']);
  const ms = value['ms'];
  return { headline: headline === '' ? null : headline, ms: typeof ms === 'number' ? ms : null };
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

/** The images among a tool result's `parts`, each whole as it was kept, in order. */
export function imagesOf(parts: unknown): readonly HeldImage[] {
  const list = Array.isArray(parts) ? (parts as readonly unknown[]) : [];
  return list
    .filter(isRecord)
    .filter((part) => part['type'] === 'file' && isRecord(part['blob']))
    .map((part) => {
      const blob = isRecord(part['blob']) ? part['blob'] : {};
      return {
        sha256: text(blob['sha256']),
        mediaType: text(blob['media_type']),
        name: text(part['name']) || $localize`:@@conversation.image:image`,
      };
    })
    .filter((image) => image.sha256 !== '' && DRAWN.includes(image.mediaType));
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

/** The kind of a field as `service.asked` records it, as the card draws it. */
function fieldKindOf(kind: Data): GlzServiceField['kind'] {
  const choices = Array.isArray(kind['options']) ? (kind['options'] as readonly unknown[]) : [];
  const given = kind['default'];
  switch (kind['type']) {
    case 'choice':
      return {
        type: 'choice',
        options: choices
          .filter(isRecord)
          .map((option) => ({ value: text(option['value']), label: text(option['label']) })),
        multiple: kind['multiple'] === true,
        default: Array.isArray(given) ? given.map(text) : [],
      };
    case 'yes_no':
      return { type: 'yes_no', default: typeof given === 'boolean' ? given : null };
    case 'number':
      return {
        type: 'number',
        integer: kind['integer'] === true,
        default: typeof given === 'number' ? given : null,
      };
    default:
      return { type: 'text', default: typeof given === 'string' ? given : null };
  }
}

/** The fields a `service.asked` asks for. */
export function serviceFieldsOf(fields: unknown): readonly GlzServiceField[] {
  const list = Array.isArray(fields) ? (fields as readonly unknown[]) : [];
  return list.filter(isRecord).map((field) => ({
    name: text(field['name']),
    label: text(field['label']),
    description: text(field['description']),
    required: field['required'] === true,
    kind: fieldKindOf(isRecord(field['kind']) ? field['kind'] : {}),
  }));
}

/** What a `service.asked` adds, as the server recorded it: `null` for a service's own. */
export function serviceDetailOf(adds: unknown): GlzServiceDetail | null {
  if (!isRecord(adds)) {
    return null;
  }
  const files = Array.isArray(adds['files']) ? (adds['files'] as readonly unknown[]) : [];
  const cut = adds['cut'];
  return {
    front: text(adds['front']),
    text: text(adds['text']),
    textCut: adds['text_cut'] === true,
    files: files.filter(isRecord).map((file) => ({
      path: text(file['path']),
      text: text(file['text']),
      runsCode: file['runs_code'] === true,
      cut: file['cut'] === true,
    })),
    cut: typeof cut === 'string' ? cut : null,
  };
}

const DECISIONS: readonly string[] = ['allow_once', 'allow_for_thread', 'deny'];

function isDecision(value: unknown): value is GlzDecision {
  return typeof value === 'string' && DECISIONS.includes(value);
}

/**
 * What an approval asks to repeat, when it is a schedule's card: one carries what it was written
 * from (`repeating`), whatever its question says. Its lines, typed (`lines`), else its `detail`,
 * a line each; `null` for any other approval.
 */
function repeatOf(data: Data): RepeatAsk | null {
  if (!isRecord(data['repeating'])) {
    return null;
  }
  const lines = data['lines'];
  const typed: readonly CardLine[] = isRecord(lines)
    ? (['when', 'chat', 'prompt'] as const).map((says) => ({ says, text: text(lines[says]) }))
    : text(data['detail'])
        .split('\n')
        .map((line) => ({ says: null, text: line }));
  return { lines: typed.filter((line) => line.text.trim() !== ''), chat: null };
}

/** The chat a `schedule` call's result says it made (`structured`, `chat`), if it made one. */
function chatOf(data: Data): string | null {
  const structured = data['structured'];
  const chat = data['is_error'] === true || !isRecord(structured) ? '' : text(structured['chat']);
  return chat === '' ? null : chat;
}

/** Whether glave wrote a message the person's side of the conversation holds, such as a note. */
export function byGlave(data: Data): boolean {
  const author = data['author'];
  return isRecord(author) && author['kind'] === 'glave';
}

/**
 * A thread's items from its stored events, in order: messages, tool calls with their
 * results, approvals with their decisions, questions with their answers, what services ask
 * with what came of it, and the connectors a search suggests. A message of the
 * person's after the one that started a turn, before the turn ended, is one they sent while it
 * ran: `steered`.
 */
export function threadItems(events: readonly RawEvent[]): readonly ThreadItem[] {
  const items: ThreadItem[] = [];
  // Whether a turn runs at this point of the log, and whether the person has asked in it.
  let running = false;
  let asked = false;
  // Whether the person just approved the plan, so glave's next message is the prompt that
  // builds it, which their Build it stands for.
  let approved = false;
  const index = new Map<string, number>();
  // The latest of what each call's service asked, by the call: a call may ask more than once.
  const asking = new Map<string, string>();
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
      case 'plan.approved':
        put({
          kind: 'message',
          key: `plan-approved-${String(event.seq)}`,
          seq: event.seq,
          from: 'person',
          text: '',
          reasoning: '',
          thought: NO_THOUGHT,
          steered: false,
          files: [],
          built: true,
        });
        approved = true;
        break;
      case 'message.added': {
        const person = data['role'] === 'user';
        const theirs = person && !byGlave(data);
        if (approved && person && !theirs) {
          // Glave's prompt to build the plan: the person asked for it with their Build it.
          approved = false;
          asked = true;
          break;
        }
        const parts = partsText(data['parts']);
        put({
          kind: 'message',
          key: `message-${String(event.seq)}`,
          seq: event.seq,
          from: person ? 'person' : 'model',
          text: person ? promptOf(data['parts']) : parts.text,
          reasoning: parts.reasoning,
          thought: thoughtOf(data['thought']),
          steered: theirs && running && asked,
          files: filesOf(data['parts']),
        });
        asked ||= theirs;
        approved = false;
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
            call,
            tool,
            // The step in the server's line; a call recorded before it said one, its tool.
            summary: text(data['summary']) || tool,
            status: 'running',
            output: '',
            images: [],
          });
        }
        break;
      }
      case 'tool.call.completed': {
        const call = text(data['call_id']);
        const output = partsText(data['output']).text;
        const images = imagesOf(data['output']);
        update(`approval-${call}`, (item) =>
          item.kind === 'approval' && item.repeat !== undefined
            ? { ...item, repeat: { ...item.repeat, chat: chatOf(data) } }
            : item,
        );
        update(`call-${call}`, (item) => {
          if (item.kind === 'call') {
            const status = data['is_error'] === true ? 'failed' : 'done';
            return { ...item, status, output, images };
          }
          if (item.kind === 'question') {
            return { ...item, answers: answersOf(data['structured']) };
          }
          return item;
        });
        break;
      }
      case 'approval.requested': {
        const approval = text(data['call_id']);
        const outside = data['outside_sandbox'];
        const repeat = repeatOf(data);
        put({
          kind: 'approval',
          key: `approval-${approval}`,
          seq: event.seq,
          approval,
          tool: text(data['tool']),
          summary: text(data['summary']),
          ask: text(data['ask']) || text(data['summary']),
          detail: text(data['detail']),
          caution: text(data['caution']),
          outsideSandbox: isRecord(outside),
          withNetwork: isRecord(outside) && outside['lifted'] === 'network',
          modelReason: isRecord(outside) ? text(outside['reason']) : '',
          ...(repeat === null ? {} : { repeat }),
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
      case 'service.asked': {
        const call = text(data['call_id']);
        const key = `service-${String(event.seq)}`;
        put({
          kind: 'service',
          key,
          seq: event.seq,
          call,
          service: text(data['service']),
          ask: text(data['ask']),
          note: text(data['note']),
          message: text(data['message']),
          fields: serviceFieldsOf(data['fields']),
          detail: serviceDetailOf(data['adds']),
          told: null,
        });
        asking.set(call, key);
        break;
      }
      case 'service.answered':
        update(asking.get(text(data['call_id'])) ?? '', (item) =>
          item.kind === 'service' ? { ...item, told: text(data['told']) } : item,
        );
        break;
      case 'connector.suggested':
        put({
          kind: 'suggestion',
          key: `suggestion-${String(event.seq)}`,
          seq: event.seq,
          connector: text(data['connector']),
          told: text(data['told']),
        });
        break;
      default:
        break;
    }
  }
  return items;
}

/**
 * The draft after a live update: text and reasoning grow, and what the reasoning came to as the
 * server says it (its headline, then how long it took); a model call beginning (`waiting`)
 * starts its step afresh, so what a call tried again had written goes; anything else leaves it.
 */
export function applyUpdate(draft: Draft, update: Update): Draft {
  switch (update.kind) {
    case 'waiting':
      return NO_DRAFT;
    case 'text':
      return { ...draft, text: draft.text + update.text };
    case 'reasoning':
      return { ...draft, reasoning: draft.reasoning + update.text };
    case 'thinking':
      return { ...draft, thought: { ...draft.thought, headline: update.headline } };
    case 'thought':
      return { ...draft, thought: { ...draft.thought, ms: update.ms } };
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

/**
 * `events` with `more` added, each `seq` once, in `seq` order: `events` itself when `more`
 * holds none it lacks, so nothing drawn from them is drawn again.
 */
export function mergeEvents(
  events: readonly RawEvent[],
  more: readonly RawEvent[],
): readonly RawEvent[] {
  const known = new Set(events.map((event) => event.seq));
  const added = more.filter((event) => !known.has(event.seq));
  if (added.length === 0) {
    return events;
  }
  const bySeq = new Map<number, RawEvent>();
  for (const event of [...events, ...added]) {
    bySeq.set(event.seq, event);
  }
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}

/**
 * The person's answers to a `question` call, one for each question, empty for none, as its
 * result records them for clients (`structured`, under `answers`); none when it records none,
 * as when the call was not answered.
 */
export function answersOf(structured: unknown): readonly string[] {
  const answers = isRecord(structured) ? structured['answers'] : undefined;
  return Array.isArray(answers)
    ? answers.map((answer: unknown) => (typeof answer === 'string' ? answer : ''))
    : [];
}
