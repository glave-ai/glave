import { expect, test } from '@playwright/test';

import { fakeApi } from './fake-api';
import { byTestId, expectAccessible, watchForProblems } from './support';

const SIGNED_IN = '#token=glave_local_test';

test('glave serves a web app manifest that makes it installable', async ({ page, request }) => {
  const manifest = await request.get('/manifest.webmanifest');
  expect(manifest.status()).toBe(200);
  expect(manifest.headers()['content-type']).toBe('application/manifest+json');
  const body: unknown = await manifest.json();
  expect(body).toMatchObject({ name: 'glave', display: 'standalone', start_url: '/' });
  for (const icon of ['/icon-192.png', '/icon-512.png']) {
    const answer = await request.get(icon);
    expect(answer.headers()['content-type']).toBe('image/png');
  }
  const problems = watchForProblems(page);
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(byTestId(page, 'nav-thread').first()).toBeVisible();
  // Chromium's own check: nothing stops installing it but the test browser's private profile.
  const cdp = await page.context().newCDPSession(page);
  const { installabilityErrors } = (await cdp.send('Page.getInstallabilityErrors')) as {
    installabilityErrors: readonly { errorId: string }[];
  };
  const errors = installabilityErrors.map(({ errorId }) => errorId);
  expect(errors.filter((error) => error !== 'in-incognito')).toEqual([]);
  expect(problems()).toEqual([]);
});

test('Install app shows the browser’s own prompt, then goes away', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(byTestId(page, 'nav-thread').first()).toBeVisible();
  await expect(byTestId(page, 'install-app')).toHaveCount(0);
  // What Chromium does when the app can be installed.
  await page.evaluate(() => {
    const prompt = Object.assign(new Event('beforeinstallprompt', { cancelable: true }), {
      prompt: () => {
        document.body.dataset['prompted'] = 'yes';
        return Promise.resolve();
      },
      userChoice: Promise.resolve({ outcome: 'accepted' }),
    });
    window.dispatchEvent(prompt);
  });
  const install = byTestId(page, 'install-app');
  await expect(install).toBeVisible();
  await expectAccessible(page);
  await install.click();
  await expect(page.locator('body')).toHaveAttribute('data-prompted', 'yes');
  await expect(install).toHaveCount(0);
});
