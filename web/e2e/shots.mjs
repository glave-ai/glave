// Screenshots of a running app for a pull request: `node e2e/shots.mjs <url> <out-dir>`.
// Light and dark, the top of /glaze and its conversation section.
import { chromium } from '@playwright/test';

const [url = 'http://127.0.0.1:4310', out = 'test-results'] = process.argv.slice(2);
const browser = await chromium.launch();
for (const colorScheme of ['light', 'dark']) {
  const page = await browser.newPage({ colorScheme, viewport: { width: 1280, height: 900 } });
  await page.goto(`${url}/glaze`);
  await page.getByTestId('showcase-heading').waitFor();
  await page.screenshot({ path: `${out}/glaze-${colorScheme}.png` });
  await page.locator('#ai').scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${out}/glaze-ai-${colorScheme}.png` });
  await page.close();
}
await browser.close();
