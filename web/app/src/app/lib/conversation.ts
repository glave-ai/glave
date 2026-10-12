import type { RawEvent, Update } from '@glave/client';
import type { GlzDecision, GlzQuestion, GlzReceiptStatus } from '@glave/glaze';

/** A message in a thread, as the thread view shows it. */
export interface MessageItem {
  readonly kind: 'message';
  readonly key: string;
  readonly from: 'person' | 'model';
  readonly text: string;
  readonly reasoning: string;
  /**
   * The model that wrote it: its `model.responded`'s, else its turn's start; `null` for the
   * person's messages.
   */
  readonly model: string | null;
}

/** A tool call: what it did and what came of it. */
export interface CallItem {
  readonly kind: 'call';
  readonly key: string;
  readonly tool: string;
  readonly summary: string;
  readonly status: GlzReceiptStatus;
  readonly output: string;
}

/** An action waiting for, or answered by, the person. */
export interface ApprovalItem {
  readonly kind: 'approval';
  readonly key: string;
  readonly approval: string;
  readonly tool: string;
  readonly summary: string;
  readonly decision: GlzDecision | null;
}

/** Questions the model asked the person. */
export interface QuestionItem {
  readonly kind: 'question';
  readonly key: string;
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

/** The text and reasoning of a list of content parts. */
export function partsText(parts: unknown): { readonly text: string; readonly reasoning: string } {
  const list = Array.isArray(parts) ? (parts as readonly unknown[]) : [];
  const of = (type: string): string =>
    list
      .filter(isRecord)
      .filter((part) => part['type'] === type)
      .map((part) => text(part['text']))
      .filter((part) => part !== '')
      .join('\n');
  return { text: of('text'), reasoning: of('reasoning') };
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

/**
 * A thread's items from its stored events, in order: messages, tool calls with their
 * results, approvals with their decisions, and questions with their answers.
 */
export function threadItems(events: readonly RawEvent[]): readonly ThreadItem[] {
  const items: ThreadItem[] = [];
  let model: string | null = null;
  /** The assistant message whose `model.responded` comes next. */
  let answered: string | null = null;
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
        model = modelOf(data['model']);
        break;
      case 'message.added': {
        const person = data['role'] === 'user';
        const parts = partsText(data['parts']);
        const key = `message-${String(event.seq)}`;
        put({
          kind: 'message',
          key,
          from: person ? 'person' : 'model',
          text: person ? promptOf(data['parts']) : parts.text,
          reasoning: parts.reasoning,
          model: person ? null : model,
        });
        answered = person ? null : key;
        break;
      }
      case 'model.responded': {
        // The answer an assistant message came from follows it: its model wrote the message.
        const by = modelOf(data['model']);
        if (answered !== null && by !== null) {
          update(answered, (item) => (item.kind === 'message' ? { ...item, model: by } : item));
        }
        answered = null;
        break;
      }
      case 'tool.call.requested': {
        const call = text(data['call_id']);
        const tool = text(data['tool']);
        if (tool === 'question') {
          put({
            kind: 'question',
            key: `call-${call}`,
            call,
            questions: questionsOf(data['arguments']),
            answers: null,
          });
        } else {
          put({
            kind: 'call',
            key: `call-${call}`,
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
        const output = partsText(data['output']).text;
        update(`call-${call}`, (item) => {
          if (item.kind === 'call') {
            return { ...item, status: data['is_error'] === true ? 'failed' : 'done', output };
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
        put({
          kind: 'approval',
          key: `approval-${approval}`,
          approval,
          tool: text(data['tool']),
          summary: text(data['summary']),
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

/** The model a `turn.started` names, if it names one. */
function modelOf(value: unknown): string | null {
  const name = isRecord(value) ? text(value['name']) : '';
  return name === '' ? null : name;
}

/** The model the latest turn started with, for a reply still streaming. */
export function latestModel(events: readonly RawEvent[]): string | null {
  const started = events.filter((event) => event.type === 'turn.started').at(-1);
  return started === undefined ? null : modelOf(started.data['model']);
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
