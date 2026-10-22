import { type Locator, type Page, expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible } from './support';

/** The composer's three menus: each by its button, and the part of what it opens that shows. */
const MENUS = {
  'approval mode': { trigger: 'approval-mode-trigger', shown: 'approval-mode-option' },
  model: { trigger: 'model-picker-trigger', shown: 'model-picker-listbox' },
  reasoning: { trigger: 'effort-picker-trigger', shown: 'effort-picker-option' },
} as const;

type Name = keyof typeof MENUS;

/** A menu's button, and what shows while it is open. */
interface Menu {
  readonly trigger: Locator;
  readonly shown: Locator;
}

function menu(page: Page, name: Name): Menu {
  const { trigger, shown } = MENUS[name];
  return { trigger: byTestId(page, trigger), shown: byTestId(page, shown).first() };
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
  // Hope offers reasoning levels, so the composer shows all three menus.
  await byTestId(page, 'model-picker-trigger').click();
  await byTestId(page, 'model-picker-option').filter({ hasText: 'Hope' }).click();
  await expect(byTestId(page, 'effort-picker-trigger')).toBeVisible();
});

for (const name of Object.keys(MENUS) as readonly Name[]) {
  test(`the ${name} menu's own button opens it and closes it`, async ({ page }) => {
    const { trigger, shown } = menu(page, name);
    await trigger.click();
    await expect(shown).toBeVisible();
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
    await expectAccessible(page);
    // Pressed again while open, the button closes it, and it stays closed (#594), focus on the
    // button with no ring, as a press leaves it.
    await trigger.click();
    await expect(shown).toBeHidden();
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await expect(trigger).toBeFocused();
    expect(await trigger.evaluate((button) => button.matches(':focus-visible'))).toBe(false);
    await trigger.click();
    await expect(shown).toBeVisible();
  });

  test(`Escape closes the ${name} menu and gives focus back to its button`, async ({ page }) => {
    const { trigger, shown } = menu(page, name);
    await trigger.click();
    await expect(shown).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(shown).toBeHidden();
    await expect(trigger).toBeFocused();
  });
}

test('a click outside an open menu closes it and still does what it does', async ({ page }) => {
  const approval = menu(page, 'approval mode');
  const model = menu(page, 'model');
  const reasoning = menu(page, 'reasoning');
  // A click on another menu's button closes the one open and opens its own.
  await approval.trigger.click();
  await expect(approval.shown).toBeVisible();
  await model.trigger.click();
  await expect(approval.shown).toBeHidden();
  await expect(model.shown).toBeVisible();
  await reasoning.trigger.click();
  await expect(model.shown).toBeHidden();
  await expect(reasoning.shown).toBeVisible();
  await approval.trigger.click();
  await expect(reasoning.shown).toBeHidden();
  await expect(approval.shown).toBeVisible();
  // A click in the composer closes it and puts the cursor there, ready to type.
  const input = byTestId(page, 'thread-composer-input');
  await input.click();
  await expect(approval.shown).toBeHidden();
  await expect(input).toBeFocused();
  await page.keyboard.type('Go on');
  await expect(input).toHaveValue('Go on');
});
