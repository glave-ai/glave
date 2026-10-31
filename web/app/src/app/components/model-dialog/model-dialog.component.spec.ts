import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Filters } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { ROUTED } from '../../testing/fixtures';
import { ModelDialogComponent } from './model-dialog.component';

const FLASH = 'deepseek/deepseek-v4.1-flash';

async function render(filters: Filters = {}): Promise<{
  fixture: ReturnType<typeof TestBed.createComponent<ModelDialogComponent>>;
  root: HTMLElement;
  fake: FakeSession;
  settle: () => Promise<void>;
}> {
  const fake = new FakeSession()
    .answer('filters/get', { filters, europe: ['DE'] })
    .answer('filters/set', (params) => ({
      filters: (params as { readonly filters: Filters }).filters,
      europe: ['DE'],
    }))
    .answer('model/list', { models: ROUTED });
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(ModelDialogComponent);
  fixture.componentRef.setInput('open', true);
  const settle = async (): Promise<void> => {
    await fixture.whenStable();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
  };
  await settle();
  return { fixture, root: fixture.nativeElement as HTMLElement, fake, settle };
}

function byTestId(root: HTMLElement, id: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${id}"]`);
}

function options(root: HTMLElement): readonly HTMLButtonElement[] {
  return Array.from(
    root.querySelectorAll<HTMLButtonElement>('[data-testid="model-picker-option"]'),
  );
}

/** Chooses Groq to serve the model shown, and uses it. */
async function useGroq(root: HTMLElement, settle: () => Promise<void>): Promise<void> {
  const groq = Array.from(root.querySelectorAll('[data-testid="model-dialog-who"] glz-radio')).find(
    (radio) => radio.textContent.includes('Groq'),
  );
  groq?.querySelector('input')?.click();
  await settle();
  byTestId(root, 'model-dialog-use')?.click();
  await settle();
}

describe('ModelDialogComponent', () => {
  it('lists each model with its cheapest price and best speed among the providers left', async () => {
    const { root } = await render();
    const rows = options(root).map((row) =>
      Array.from(row.querySelectorAll('span span'), (part) => part.textContent.trim()),
    );
    expect(rows).toEqual([
      [
        'DeepSeek V4.1 Flash',
        'DeepSeek · 131K context · 2 providers',
        'from $0.12 in · $0.50 out',
        'up to 210 tokens a second',
      ],
      ['Anthropic · Claude Haiku 5.5', '', '', ''],
    ]);
    expect(byTestId(root, 'model-filters')).not.toBeNull();
    await expectAccessible(root);
  });

  it('finds a model by its name or maker as the person types', async () => {
    const { root, settle } = await render();
    const search = byTestId(root, 'model-picker-search') as HTMLInputElement;
    search.value = 'anthropic';
    search.dispatchEvent(new Event('input'));
    await settle();
    expect(options(root)).toHaveLength(1);
    search.value = 'nothing like it';
    search.dispatchEvent(new Event('input'));
    await settle();
    expect(byTestId(root, 'model-picker-empty')?.textContent).toContain(
      'No model matches your search.',
    );
  });

  it('picks a model no router serves at once', async () => {
    const { fixture, root, settle } = await render();
    options(root)[1]?.click();
    await settle();
    expect(fixture.componentInstance.value()).toBe('claude-haiku-5-5');
    expect(fixture.componentInstance.open()).toBe(false);
  });

  it('shows who serves a model, which the filters hide and why, and keeps the one picked', async () => {
    const { fixture, root, fake, settle } = await render();
    options(root)[0]?.click();
    await settle();
    expect(byTestId(root, 'model-dialog-hidden')?.textContent).toContain(
      'Your filters hide DeepSeek (based in China).',
    );
    const labels = (): readonly string[] =>
      Array.from(root.querySelectorAll('[data-testid="model-dialog-who"] glz-radio'), (radio) =>
        radio.textContent.replace(/\s+/g, ' ').trim(),
      );
    expect(labels().map((label) => label.split(' · ')[0])).toEqual([
      'Let glave choose: lowest priceWhichever provider within your filters is cheapest when it’s asked',
      'Let glave choose: fastestWhichever provider within your filters is quickest when it’s asked',
      'GroqBased in United States',
      'FireworksBased in United States, where it runs isn’t said',
    ]);
    await expectAccessible(root);
    byTestId(root, 'model-dialog-show-all')?.click();
    await settle();
    expect(labels()).toHaveLength(5);
    await useGroq(root, settle);
    expect(fake.paramsOf('filters/set')).toEqual([
      { filters: { serves: { [FLASH]: { provider: 'groq' } } } },
    ]);
    expect(fixture.componentInstance.value()).toBe(FLASH);
  });

  it('goes back to the list from who serves a model', async () => {
    const { root, settle } = await render();
    options(root)[0]?.click();
    await settle();
    byTestId(root, 'model-dialog-back')?.click();
    await settle();
    expect(options(root)).toHaveLength(2);
  });

  it('marks the model in use', async () => {
    const { fixture, root, settle } = await render();
    fixture.componentRef.setInput('current', FLASH);
    await settle();
    expect(options(root).map((row) => row.getAttribute('aria-current'))).toEqual(['true', 'false']);
    expect(options(root)[0]?.querySelector('glz-icon')?.getAttribute('aria-label')).toBe('Chosen');
    await expectAccessible(root);
  });

  it('opens again at the list, as it was before the person searched', async () => {
    const { fixture, root, settle } = await render();
    const search = byTestId(root, 'model-picker-search') as HTMLInputElement;
    search.value = 'deepseek';
    search.dispatchEvent(new Event('input'));
    await settle();
    options(root)[0]?.click();
    await settle();
    fixture.componentRef.setInput('open', false);
    await settle();
    fixture.componentRef.setInput('open', true);
    await settle();
    expect(options(root)).toHaveLength(2);
    expect((byTestId(root, 'model-picker-search') as HTMLInputElement).value).toBe('');
  });

  it('stays open, saying why, when the server keeps not who serves it', async () => {
    const { fixture, root, fake, settle } = await render();
    fake.answer(
      'filters/set',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'The store is not answering.' }),
    );
    options(root)[0]?.click();
    await settle();
    await useGroq(root, settle);
    expect(fixture.componentInstance.open()).toBe(true);
    expect(fixture.componentInstance.value()).toBeNull();
    expect(byTestId(root, 'model-dialog-problem')?.textContent).toContain(
      'The store is not answering.',
    );
  });
});
