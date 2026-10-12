// Screenshots of a running app for a pull request: `node e2e/shots.mjs <url> <out-dir>`.
// Light and dark: the top of /glaze and its conversation section, and the app's main screen
// (a thread, its session API answered by the end-to-end tests' fake) on a laptop, with the
// tools open, and on a phone.
import { chromium } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api.ts';

const [url = 'http://127.0.0.1:4310', out = 'test-results'] = process.argv.slice(2);
const SCREENS = [
  { name: '1440x900', width: 1440, height: 900, tools: true },
  { name: '390x844', width: 390, height: 844, tools: false },
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
    });
    await fakeApi(main);
    await main.goto(`${url}/t/${THREAD}#token=glave_local_test`);
    await main.getByTestId('thread-message').last().waitFor();
    if (screen.tools) {
      await main.getByTestId('thread-tools').click();
      await main.getByTestId('tools-tabs-tab').nth(1).click();
      await main.getByTestId('file').first().click();
    }
    await main.screenshot({ path: `${out}/main-${screen.name}-${colorScheme}.png` });
    await main.close();
  }
}
await browser.close();
