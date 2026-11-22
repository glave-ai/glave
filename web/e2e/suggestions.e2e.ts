import { CONNECT_FIRST } from './fake-model';
import {
  LIVE,
  byTestId,
  expect,
  expectAccessible,
  served,
  signInCode,
  test,
  watchForProblems,
} from './support';

/**
 * A real glave, its model the fake service of fake-model.ts: asked for an app the person has
 * not connected, the model's search finds it in glave's catalog, and the chat offers it in a
 * card in the server's words; Connect connects it at once, everywhere (DeepWiki asks no one to
 * sign in), and the card shrinks to a line. Disconnected again after, as the glave is shared.
 */
test('an app a search finds is offered in a card, connected from it, and shrinks to a line', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  const live = served(LIVE);
  await page.goto(`${live}/#code=${await signInCode(LIVE)}`);
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Find an app that reads public repositories');
  await prompt.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText(CONNECT_FIRST);

  const card = byTestId(page, 'connect-card');
  await expect(byTestId(page, 'connect-card-name')).toHaveText('DeepWiki');
  await expect(byTestId(page, 'connect-card-told')).toHaveText(
    "DeepWiki isn't connected. Connect it to let me use it for you.",
  );
  await expect(byTestId(page, 'connect-card-connect')).toHaveText('Connect DeepWiki');
  await expect(byTestId(page, 'connect-card-not-now')).toHaveText('Not now');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  await byTestId(page, 'connect-card-connect').click();
  await expect(card).toBeHidden();
  await expect(byTestId(page, 'connect-card-state')).toHaveText('DeepWiki connected');
  await expect(byTestId(page, 'connect-card-used')).toHaveText('Everywhere, including chats');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  // Disconnected again from the person's connections, the glave as it was.
  await byTestId(page, 'person').click();
  await byTestId(page, 'nav-settings').click();
  await byTestId(page, 'settings-connections').click();
  const connected = byTestId(page, 'connectors-connected').locator('[data-connector="deepwiki"]');
  await connected.getByTestId('connector-menu-trigger').click();
  await byTestId(page, 'connector-menu-action').getByText('Disconnect').click();
  await byTestId(page, 'connectors-disconnect-ok').click();
  await expect(
    page.locator('[data-connector="deepwiki"]').getByTestId('connector-action'),
  ).toHaveText('Connect');
  expect(problems()).toEqual([]);
});
