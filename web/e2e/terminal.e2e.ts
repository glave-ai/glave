import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  LIVE,
  byTestId,
  expect,
  expectAccessible,
  homeOf,
  served,
  signInCode,
  test,
  watchForProblems,
} from './support';

/** What `method` with `params` answers from the real glave of LIVE, asked with its local token. */
async function asked(method: string, params: object): Promise<Record<string, unknown>> {
  const token = readFileSync(join(homeOf(LIVE), 'serve.token'), 'utf8').trim();
  const response = await fetch(`${served(LIVE)}/`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  });
  const answer = (await response.json()) as { readonly result?: Record<string, unknown> };
  if (answer.result === undefined) {
    throw new Error(`glave refused ${method}: ${JSON.stringify(answer)}`);
  }
  return answer.result;
}

/**
 * A real glave, the person's own: a thread in a project's folder has a Terminal tab beside its
 * facts, which opens their own shell in that folder, under glave's Content Security Policy, and
 * runs what they type in it.
 */
test('the Terminal tab opens the person’s own shell in the thread’s folder and runs what they type', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  const folder = mkdtempSync(join(tmpdir(), 'glave-e2e-terminal-'));
  try {
    const made = await asked('project/create', {
      name: 'terminal',
      folder: { kind: 'path', path: folder },
    });
    const project = (made['project'] as { readonly id: string }).id;
    const started = await asked('thread/start', { project });
    const thread = (started['thread'] as { readonly id: string }).id;

    await page.goto(`${served(LIVE)}/t/${thread}#code=${await signInCode(LIVE)}`);
    const tabs = byTestId(page, 'panel-tabs-tab');
    await expect(tabs).toHaveText(['Overview', 'Terminal']);
    await tabs.filter({ hasText: 'Terminal' }).click();
    await expect(byTestId(page, 'terminal-who')).toHaveText("Your shell, not the agent's");
    await expect(byTestId(page, 'terminal-folder')).toContainText('glave-e2e-terminal-');

    const screen = byTestId(page, 'terminal-screen');
    await screen.click();
    await page.keyboard.type('echo hi-$((40+2))');
    await page.keyboard.press('Enter');
    await expect(screen).toContainText('hi-42');
    for (const colorScheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme });
      await expectAccessible(page);
    }

    await byTestId(page, 'terminal-close').click();
    await expect(screen).toBeHidden();
    expect(problems()).toEqual([]);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
