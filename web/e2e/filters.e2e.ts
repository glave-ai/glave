import { THREAD, fakeApi } from './fake-api';
import { fakeFilters } from './fake-filters';
import { SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

const FLASH = 'deepseek/deepseek-v4.1-flash';

test('where a router serves a model, the person chooses who serves it within their filters', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  const { kept } = await fakeFilters(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await byTestId(page, 'model-picker-trigger').click();
  const dialog = byTestId(page, 'model-dialog');
  await expect(dialog).toBeVisible();
  const flash = byTestId(page, 'model-picker-option').filter({ hasText: 'DeepSeek V4.1 Flash' });
  await expect(flash).toContainText('3 providers');
  await expectAccessible(page);

  // Providers based in the US only: DeepSeek's own, in China, is hidden, and said to be.
  await byTestId(page, 'model-filters-place-option').filter({ hasText: 'US only' }).click();
  await expect(flash).toContainText('1 provider');
  await expect.poll(() => kept.at(-1)).toEqual({ based_in: ['US'] });
  await flash.click();
  await expect(byTestId(page, 'model-dialog-hidden')).toContainText(
    'Your filters hide DeepSeek (based in China) and Nebius (based in Netherlands).',
  );
  await expectAccessible(page);
  await byTestId(page, 'model-dialog-who').getByText('Groq', { exact: true }).click();
  await byTestId(page, 'model-dialog-use').click();
  await expect(dialog).toBeHidden();
  await expect
    .poll(() => kept.at(-1))
    .toEqual({ based_in: ['US'], serves: { [FLASH]: { provider: 'groq' } } });
  await expect(byTestId(page, 'model-picker-trigger')).toContainText('DeepSeek V4.1 Flash');

  // Opened again, the list marks the model in use.
  await byTestId(page, 'model-picker-trigger').click();
  await expect(flash).toHaveAttribute('aria-current', 'true');
});
