import { type Page, expect, test } from '@playwright/test';

import { MARKDOWN_PROMPT, THREAD, fakeApi, streamReply } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible, watchForProblems } from './support';

/** What the streaming reply showed: how many states, and any with markup in them. */
interface Seen {
  readonly states: number;
  readonly markup: readonly string[];
}

/**
 * Starts recording each state of the streaming reply, on every change to the page, and any
 * that shows markdown's markup outside its code: `**`, a backtick, `~~` or a link half written.
 * Read with `seenReply`.
 */
async function watchReply(page: Page): Promise<void> {
  await page.evaluate(() => {
    const seen = { states: 0, markup: [] as string[] };
    Object.assign(window, { seenReply: seen });
    new MutationObserver(() => {
      const reply = document.querySelector('[data-testid="thread-reply"] .glz-markdown');
      if (reply === null) {
        return;
      }
      const prose = reply.cloneNode(true) as Element;
      for (const code of prose.querySelectorAll('pre, code')) {
        code.remove();
      }
      seen.states += 1;
      if (/\*\*|`|~~|\]\(/u.test(prose.textContent)) {
        seen.markup.push(prose.textContent);
      }
    }).observe(document.body, { subtree: true, childList: true, characterData: true });
  });
}

async function seenReply(page: Page): Promise<Seen> {
  return page.evaluate(() => (window as unknown as { seenReply: Seen }).seenReply);
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a reply streams in as markdown, never showing its markup or fetching an image (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    const fetched: string[] = [];
    page.on('request', (request) => {
      if (new URL(request.url()).hostname.endsWith('evil.example')) {
        fetched.push(request.url());
      }
    });
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await fakeApi(page);
    await streamReply(page);
    const response = await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    // Images come from the app, and from files the person adds (`blob:`), under Trusted Types.
    const policy = response?.headers()['content-security-policy'] ?? '';
    expect(policy).toMatch(/(^|; )img-src 'self' blob:(;|$)/u);
    expect(policy).toContain("require-trusted-types-for 'script'");
    await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
    await watchReply(page);
    const composer = byTestId(page, 'thread-composer-input');
    await composer.fill(MARKDOWN_PROMPT);
    await composer.press('Enter');
    // While it streams, the reply is markdown: bold as it is written, never its asterisks.
    const live = byTestId(page, 'thread-reply');
    await expect(live.locator('strong', { hasText: 'empty input' })).toBeVisible();
    const reply = byTestId(page, 'thread-message').last();
    await expect(reply).toContainText('Document the change');
    await expect(live).toHaveCount(0);
    const seen = await seenReply(page);
    expect(seen.states).toBeGreaterThan(20);
    expect(seen.markup).toEqual([]);

    await expect(reply.getByRole('heading', { name: 'The parser is fixed' })).toBeVisible();
    await expect(reply.locator('th')).toHaveText(['Input', 'Before', 'After']);
    await expect(reply.getByRole('img', { name: 'Done', exact: true })).toHaveCount(1);
    await expect(reply.getByRole('img', { name: 'Not done', exact: true })).toHaveCount(1);
    const issue = reply.getByRole('link', { name: 'the issue (opens in a new tab)' });
    await expect(issue).toHaveAttribute('target', '_blank');
    await expect(issue).toHaveAttribute('rel', 'noopener noreferrer');
    // Images are links to them, and raw HTML is text: nothing is fetched.
    await expect(reply.locator('img')).toHaveCount(0);
    await expect(
      reply.getByRole('link', { name: 'Image: Coverage by file (opens in a new tab)' }),
    ).toHaveAttribute('href', 'https://evil.example/chart.png?thread=make-the-tests-pass');
    await expect(reply).toContainText('<img src="https://evil.example/pixel.png">');
    // The code block: its language, highlighting loaded for it, and a copy button.
    await expect(reply.locator('.glz-markdown-language')).toHaveText('rust');
    await expect(reply.locator('.hljs-keyword').first()).toHaveText('fn');
    await expect(reply.getByTestId('thread-markdown-copy')).toHaveAccessibleName('Copy code');
    await expectAccessible(page);
    expect(fetched).toEqual([]);
    expect(problems()).toEqual([]);
  });
}

test('a code block’s copy button copies its code', async ({ page, context, browserName }) => {
  test.skip(browserName === 'webkit', "Playwright's WebKit cannot be granted the clipboard");
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await fakeApi(page);
  await streamReply(page, '```sh\ncargo test --workspace\n```', 0);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  const composer = byTestId(page, 'thread-composer-input');
  await composer.fill(MARKDOWN_PROMPT);
  await composer.press('Enter');
  const reply = byTestId(page, 'thread-message').last();
  // Highlighting, which loads after the block first shows, draws the block again and gives it
  // a new copy button: the one clicked is the one that stays.
  await expect(reply.locator('.hljs-built_in')).toHaveText('test');
  const copy = reply.getByTestId('thread-markdown-copy');
  await copy.click();
  await expect(copy).toHaveAccessibleName('Copied');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('cargo test --workspace');
});
