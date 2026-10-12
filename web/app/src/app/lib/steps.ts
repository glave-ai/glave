import type { GlzIconData } from '@glave/glaze';
import {
  FilePenLine,
  FilePlus,
  FileText,
  FolderSearch,
  Globe,
  SquareTerminal,
  TextSearch,
  Wrench,
} from 'lucide';

import type { CallItem, ThreadItem } from './conversation';

/** Tool calls in a row, folded under one line, as Claude's and ChatGPT's apps fold them. */
export interface StepsItem {
  readonly kind: 'steps';
  readonly key: string;
  readonly calls: readonly CallItem[];
  /** What they came to: "Ran 4 steps", "Ran 3 steps, 1 failed", "Running 2 steps". */
  readonly summary: string;
}

const TOOL_ICONS: Readonly<Record<string, GlzIconData>> = {
  bash: SquareTerminal,
  edit: FilePenLine,
  write: FilePlus,
  read: FileText,
  search: TextSearch,
  glob: FolderSearch,
  web_fetch: Globe,
};

/** The icon for a tool: the harness's own by name, a wrench for any other. */
export function toolIcon(tool: string): GlzIconData {
  return TOOL_ICONS[tool] ?? Wrench;
}

function count(n: number, one: string, many: string): string {
  return `${String(n)} ${n === 1 ? one : many}`;
}

/** What steps came to, in a line: running while any runs, then how many failed or were declined. */
export function stepsSummary(calls: readonly CallItem[]): string {
  const steps = count(calls.length, 'step', 'steps');
  if (calls.some((call) => call.status === 'running')) {
    return `Running ${steps}`;
  }
  const failed = calls.filter((call) => call.status === 'failed').length;
  const declined = calls.filter((call) => call.status === 'declined').length;
  return [
    `Ran ${steps}`,
    ...(failed > 0 ? [`${String(failed)} failed`] : []),
    ...(declined > 0 ? [`${String(declined)} declined`] : []),
  ].join(', ');
}

/**
 * A thread's items with each run of two or more tool calls in a row folded into one steps
 * item; a lone call stays as it is. Anything else (a message, an approval, a question) ends a
 * run, so what the person must see is never folded away.
 */
export function foldSteps(items: readonly ThreadItem[]): readonly (ThreadItem | StepsItem)[] {
  const folded: (ThreadItem | StepsItem)[] = [];
  let run: CallItem[] = [];
  const flush = (): void => {
    const [first] = run;
    if (run.length === 1 && first !== undefined) {
      folded.push(first);
    } else if (first !== undefined) {
      folded.push({
        kind: 'steps',
        key: `steps-${first.key}`,
        calls: run,
        summary: stepsSummary(run),
      });
    }
    run = [];
  };
  for (const item of items) {
    if (item.kind === 'call') {
      run.push(item);
    } else {
      flush();
      folded.push(item);
    }
  }
  flush();
  return folded;
}
