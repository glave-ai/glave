// Screenshots of a running app for a pull request: `node e2e/shots.mjs <url> <out-dir>`.
// Light and dark: the top of /glaze and its conversation section, and the app's main screen
// (a thread, its session API answered by the end-to-end tests' fake) on a laptop, with its
// panel beside it, then a file's changes in full and its steps open, and on a phone.
import { chromium } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api.ts';

const [url, out = 'test-results'] = process.argv.slice(2);
if (url === undefined) {
  throw new Error('usage: node e2e/shots.mjs <url> <out-dir>, the url where glave serves the app');
}
const SCREENS = [
  { name: '1440x900', width: 1440, height: 900, wide: true, touch: false },
  { name: '390x844', width: 390, height: 844, wide: false, touch: true },
];

const browser = await chromium.launch();
for (const colorScheme of ['light', 'dark']) {
  const page = await browser.newPage({ colorScheme, viewport: { width: 1280, height: 900 } });
  await page.goto(`${url}/glaze`);
  await page.getByTestId('showcase-heading').waitFor();
  await page.screenshot({ path: `${out}/glaze-${colorScheme}.png` });
  await page.locator('#ai').scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${out}/glaze-ai-${colorScheme}.png` });
  await page.close();

  for (const screen of SCREENS) {
    const main = await browser.newPage({
      colorScheme,
      viewport: { width: screen.width, height: screen.height },
      hasTouch: screen.touch,
      isMobile: screen.touch,
    });
    await fakeApi(main);
    await main.goto(`${url}/t/${THREAD}#code=e2e-code`);
    await main.getByTestId('thread-message').last().waitFor();
    await main.screenshot({ path: `${out}/main-${screen.name}-${colorScheme}.png` });
    if (screen.wide) {
      await main.getByTestId('panel-file').first().click();
      await main.screenshot({ path: `${out}/changes-${screen.name}-${colorScheme}.png` });
      await main.keyboard.press('Escape');
      // The thread's tool calls opened from their steps, the pointer over the reply.
      await main.getByTestId('thread-steps-summary').click();
      await main.getByTestId('thread-message').last().hover();
      await main.screenshot({ path: `${out}/steps-${screen.name}-${colorScheme}.png` });
    }
    await main.close();
  }
}
await browser.close();
