import type { BrowserContext } from '@playwright/test';

import { oneDevice } from './devices';
import {
  ADMIN_WORDS,
  HERE,
  HERE_WEBKIT,
  PASSKEYS,
  PASSKEYS_ORIGIN,
  byTestId,
  expect,
  expectAccessible,
  heldUntilHeard,
  served,
  setupCode,
  signInCode,
  test,
  watchForProblems,
} from './support';

/** Words a person on their own server should never meet, nor a command's flags. */
const ADMIN_OR_FLAGS = new RegExp(`${ADMIN_WORDS.source}|--`, 'iu');

/**
 * Sends every request to the public origin on to the real glave, as a tunnel on its computer
 * does: the page stays at `https://glave.test`, a secure origin, WebAuthn's relying party.
 */
async function tunnel(context: BrowserContext): Promise<void> {
  const glave = served(PASSKEYS);
  await context.route(`${PASSKEYS_ORIGIN}/**`, async (route) => {
    // A watch streams for as long as the page lives, which a tunnel of whole answers cannot
    // carry: it is left waiting, as a quiet one would be.
    if (route.request().headers()['glave-method']?.endsWith('/watch') === true) {
      return;
    }
    const asked = new URL(route.request().url());
    const response = await route.fetch({
      url: `${glave}${asked.pathname}${asked.search}`,
      headers: { ...route.request().headers(), host: new URL(PASSKEYS_ORIGIN).host },
    });
    await route.fulfill({ response });
  });
}

/**
 * Signing in from elsewhere with passkeys against a real glave reached at a public origin
 * (`specs/session.md`, "Signing in"): the setup link glave on its computer makes opens a page
 * that makes a passkey on this device and signs the browser in, the link working once; a new
 * tab is signed in with it, and once signed out the passkey signs it in again, and Settings
 * lists it.
 */
test('a setup link makes a passkey here, and the passkey signs the browser in again', async ({
  context,
  browser,
  browserName,
}) => {
  await tunnel(context);
  const { tab } = oneDevice(context, browserName);
  const code = await setupCode(PASSKEYS);

  const page = await tab();
  const problems = watchForProblems(page);
  await page.goto(`${PASSKEYS_ORIGIN}/#setup=${code}`);
  await expect(byTestId(page, 'sign-in')).toContainText('Make a passkey for this device');
  expect(page.url()).not.toContain(code);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_OR_FLAGS);
  await byTestId(page, 'sign-in-passkey').click();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();

  // A new tab of the browser is signed in. Signed out, it signs in again with the passkey, and
  // Settings lists it (Chromium's run and WebKit's each add one to the same glave).
  const again = await tab();
  const problemsAgain = watchForProblems(again);
  await again.goto(`${PASSKEYS_ORIGIN}/settings`);
  await problems.signingOut(() =>
    problemsAgain.signingOut(async () => {
      await byTestId(again, 'settings-sign-out').click();
      await expect(byTestId(again, 'sign-in')).toContainText('Sign in to glave');
      await expect(byTestId(page, 'sign-in')).toBeVisible();
    }),
  );
  await expectAccessible(again);
  await byTestId(again, 'sign-in-passkey').click();
  await expect(byTestId(again, 'home-composer-input')).toBeVisible();
  await byTestId(again, 'person').click();
  await byTestId(again, 'nav-settings').click();
  await expect(byTestId(again, 'passkeys-passkey').first()).toBeVisible();
  await expectAccessible(again);

  // The setup link is spent, for another browser too.
  const another = await browser.newContext();
  await tunnel(another);
  const spent = await oneDevice(another, browserName).tab();
  const problemsSpent = watchForProblems(spent);
  await spent.goto(`${PASSKEYS_ORIGIN}/#setup=${code}`);
  await byTestId(spent, 'sign-in-passkey').click();
  await expect(byTestId(spent, 'sign-in-problem')).toBeVisible();
  expect([...problems(), ...problemsAgain(), ...problemsSpent()]).toEqual([]);
  await another.close();
});

/**
 * A passkey on this computer, against a real glave reached only here (`specs/session.md`,
 * "Signing in"): before any is made, a browser's sign-in page offers none; signed in by glave's
 * link at `localhost`, the app offers once to make one, in the words of the computer's own
 * authenticator; then another browser on this computer, which the system gives the same
 * passkeys, signs in with it in one step. Each engine has a glave of its own, as each makes the
 * first passkey.
 */
test('a passkey made here signs another browser on this computer in', async ({
  browser,
  browserName,
}) => {
  const site = browserName === 'webkit' ? HERE_WEBKIT : HERE;
  const here = `http://localhost:${new URL(served(site)).port}`;
  const nobody = await heldUntilHeard(await browser.newContext());
  const signedOut = await nobody.newPage();
  await signedOut.goto(`${here}/`);
  await expect(byTestId(signedOut, 'sign-in')).toBeVisible();
  await expect(byTestId(signedOut, 'sign-in-passkey')).toHaveCount(0);
  await nobody.close();

  const first = await heldUntilHeard(await browser.newContext());
  const device = oneDevice(first, browserName);
  const page = await device.tab();
  const problems = watchForProblems(page);
  await page.goto(`${here}/#code=${await signInCode(site)}`);
  const offer = byTestId(page, 'passkey-offer');
  await expect(offer).toBeVisible();
  await expect(offer).toContainText(/Sign in with (Touch ID|Windows Hello|a passkey) next time\?/u);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  await byTestId(page, 'passkey-offer-make').click();
  await expect(offer).toBeHidden();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();

  const second = await heldUntilHeard(await browser.newContext());
  const other = await oneDevice(second, browserName, await device.carried()).tab();
  const problemsThere = watchForProblems(other);
  await other.goto(`${here}/`);
  const signIn = byTestId(other, 'sign-in-passkey');
  await expect(signIn).toHaveText(/Sign in with (Touch ID|Windows Hello|a passkey)/u);
  await expectAccessible(other);
  await signIn.click();
  await expect(byTestId(other, 'home-composer-input')).toBeVisible();
  await expect(byTestId(other, 'passkey-offer')).toBeHidden();
  expect([...problems(), ...problemsThere()]).toEqual([]);
  await second.close();
  await first.close();
});
